//! Versioned, thread-confined C interface for native IME hosts.
//! A handle registry rejects stale and wrong-thread handles without dereferencing them.

// The workspace denies unsafe code; this crate is the C ABI native hosts link against, so every handle,
// pointer and string it accepts crosses a boundary the compiler cannot check.
// The exemption is stated here rather than left implicit by opting out of
// the workspace lint table, which would also silently drop every other lint
// the workspace adds later.
#![allow(unsafe_code)]

use msime_client_core::edition::Edition;
use msime_client_core::host_surface::{
    offered_input_schemes, HostCapabilities, HostPlatform, SurfaceRoute,
};
use msime_client_services::dictionary::access::DictionaryAccess;
/// 按用户的设置打一串键看候选，给 `msime-mcp candidates` 用。
pub mod candidate_preview;
pub mod settings_dictionary;
pub mod system_fonts;
use msime_client_core::preferences::{
    CantoneseInputMethod, InputScheme, KoreanLayout, KoreanPreferences, MultilingualLanguage,
    NineKeyShuangpinProfile, Preferences, PreferencesSnapshot, PreferencesStore,
    ShuangpinCustomProfile, ShuangpinProfile, TouchKeyboardLayout, TouchKeyboardScheme,
    TouchShuangpinLayout, TouchWubiLayout, VietnameseInputMethod, VietnamesePreferences,
    VietnameseToneStyle, WubiProfile,
};
use msime_client_core::punctuation::{
    route as punctuation_route, PunctuationContext, PunctuationRoute,
};
use msime_client_core::resources::{ResourceSet, ResourceStore, VerifiedMarker};
use msime_engine::host::{
    check_shuangpin_custom_profile, release_gloss_caches, shuangpin_builtin_table, Command,
    EngineOptions, Session,
};
use msime_engine::{
    release_idle_personal_models, validate_shuangpin_custom_profile, CandidateEdge, SchemeSet,
    SchemeType, ShuangpinCustomProfileError, ShuangpinCustomTable, ShuangpinCustomTablePart,
    ShuangpinProfileKind,
};
#[cfg(unix)]
use msime_input_runtime::TranslationQuery;
#[cfg(unix)]
use msime_input_runtime::UnixSocketProvider;
use msime_input_runtime::{
    Action, AiAssistantProviderConfig, CandidateId, CharacterWidth, GlideKeyboard, GlidePoint,
    OnlineCandidate, OnlineQuery, Reranker, Runtime, SentenceModel, Transition, TranslationService,
};
use msime_input_runtime::EmojiPanelQuery;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, CString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
// Only the Unix socket streaming entry point takes raw callback context.
#[cfg(unix)]
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
/// `msime_client_account` 在 #8201 新增的操作。
mod bounded_file;
mod chinese_english_spacing;
/// 多分类自动云同步的驱动；C 入口在 `ffi::cloud_sync`。与 `cloud_dictionary` 一样和 C 入口模块同名，所以公开，免得遮住 `ffi` 的同名模块。
mod dictionary;
/// 「我的词条」经 WebDAV 网盘同步的宿主入口，见 `msime_client_dictionary_sync`。公开 [`dictionary_webdav::request_json`]，给直接链接这个 crate 的 Tauri 设置应用用，免得它绕一圈 C ABI。
/// 同一台设备上两个各有一份个人词库的进程之间交换「我的词条」，见 `msime_client_user_words_mirror`。
// The exports live in `ffi`, but Rust consumers - this crate's own tests and
// examples, and anything linking the rlib - have always reached them at the
// crate root. Re-exported so moving the file changes no caller.
/// Largest preference document a host may hand to the C ABI, in bytes.
///
/// A valid document carries the custom touch-keyboard skin, whose photo alone may be 682,668 base64 characters; at the 16 KiB every other buffer uses, a document holding a photo could never be written back, which failed every later preference write from that host.
pub(crate) const PREFERENCES_DOCUMENT_LIMIT: usize = 1 << 20;
/// Largest serialized HostOptions document accepted by native entry points.
/// Preferences may carry a base64 custom keyboard photo, so the old 16 KiB
/// ABI limit rejected valid saved preferences before a session could start.
pub(crate) const HOST_OPTIONS_DOCUMENT_LIMIT: usize = PREFERENCES_DOCUMENT_LIMIT;
/// Largest dictionary-management request: one HostOptions document plus the
/// bounded personal-dictionary import payload and a small amount of framing.
pub(crate) const DICTIONARY_REQUEST_LIMIT: usize = HOST_OPTIONS_DOCUMENT_LIMIT + 1_200_000;

pub(crate) fn valid_sha256(value: &str) -> bool {
    msime_client_core::is_ascii_hex(value, 64)
}

pub(crate) fn valid_uuid_string(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

mod ffi;
pub use ffi::*;
pub mod local_session;

/// 会话实际用的每页候选数：偏好或宿主覆盖的值，按本库编译到的平台能排的上限截断（`HostPlatform::max_candidate_page_size`）。共享文档在别的设备上存了 10 时，只排得下九个的宿主仍拿到 9。
fn host_page_size(requested: u8) -> u8 {
    requested.min(ffi::host::compiled_platform().max_candidate_page_size())
}

/// 会话的候选窗是否竖排：桌面宿主按 `candidate_layout` 画候选窗，触屏宿主的候选栏总是横排，不读这一项（它的缺省是竖排，照读会让外接键盘的左右键不再选词）。
fn host_vertical_candidates(layout: msime_client_core::preferences::CandidateLayout) -> bool {
    ffi::host::compiled_platform().is_desktop()
        && layout == msime_client_core::preferences::CandidateLayout::Vertical
}

/// 会话实际用的中英混输触发字符数：本库编译到的平台不读这一项时（macOS、Windows 和 Linux 三个桌面宿主，`HostPlatform::reads_mixed_input_minimum_prefix`）固定为默认值，文档里存的数不起作用，设置页也不再给出这一行。
fn host_english_minimum_prefix(stored: u8) -> u8 {
    english_minimum_prefix_for(ffi::host::compiled_platform(), stored)
}

fn english_minimum_prefix_for(
    platform: msime_client_core::host_surface::HostPlatform,
    stored: u8,
) -> u8 {
    if platform.reads_mixed_input_minimum_prefix() {
        stored
    } else {
        msime_client_core::preferences::DEFAULT_MIXED_INPUT_MINIMUM_PREFIX
    }
}

/// 粤拼会话是否从第一键起按粤语双拼解释。桌面宿主（macOS、Windows、Linux）只读自己的 `cantonese.input_method`，触屏宿主只读当前触屏方案是不是 `cantonese_shuangpin`：从 iOS 同步过来的触屏方案不会把桌面的粤拼改成双拼，桌面的选择也不改触屏键盘。
fn force_cantonese_shuangpin(scheme: InputScheme, preferences: &Preferences) -> bool {
    force_cantonese_shuangpin_for(ffi::host::compiled_platform(), scheme, preferences)
}

fn force_cantonese_shuangpin_for(
    platform: msime_client_core::host_surface::HostPlatform,
    scheme: InputScheme,
    preferences: &Preferences,
) -> bool {
    scheme == InputScheme::Cantonese
        && if platform.is_desktop() {
            preferences.cantonese.input_method == CantoneseInputMethod::Shuangpin
        } else {
            preferences.touch_keyboard_schemes.selected
                == Some(TouchKeyboardScheme::CantoneseShuangpin)
        }
}

/// 会话实际用的 AI 联想候选数量：本库编译到的平台固定了这个数（`HostPlatform::fixed_ai_candidate_limit`，macOS、Windows、Linux、鸿蒙）时用固定值，不读偏好里存的值；否则用偏好。座位配置、在线查询和请求描述都从这里取，`chat_completion_http_request` 要求请求和配置里的数量一致。
fn host_ai_candidate_limit(stored: u8) -> u8 {
    ffi::host::compiled_platform()
        .fixed_ai_candidate_limit()
        .unwrap_or(stored)
}
mod pronunciation;
pub use dictionary::{
    dictionary_request_json, dictionary_words, edit_dictionary_word, edit_user_quick_phrase,
    import_dictionary_words, lookup_candidates, msime_client_dictionary,
    msime_client_dictionary_import_entries, msime_client_dictionary_scel_text,
    msime_client_dictionary_validate, msime_client_personal_dictionary_request,
    msime_client_personal_dictionary_sync, personal_dictionary_request_json, scel_text_json,
    user_quick_phrases, CandidateOrigin, DictionaryOptions, LookupCandidate, LookupScheme, NewWord,
    QuickPhrase, QuickPhraseEdit, QuickPhrasePage, Word, WordEdit, WordImport, WordKind, WordPage,
};
mod diagnostics;
pub use diagnostics::{msime_client_set_diagnostic_sink, DiagnosticSink};
mod dictionary_updates;
mod key_sound;
mod scripts;
// 生命周期核心；`pub` 是为了让集成测试（零分配断言）直接调用纯推导函数。
pub mod lifecycle;
// 对共享状态目录没有写权限的宿主怎么准备运行时（iOS 键盘没有完全访问，#10945）。
mod read_only_state;

thread_local! {
    static SESSIONS: RefCell<HashMap<u64, HostSession>> = RefCell::new(HashMap::new());
}

struct HostSession {
    runtime: Runtime,
    options: EngineOptions,
    /// HostOptions 里宿主声明的「快捷短语混进普通候选」能力（macOS、Linux 请求）：偏好重建 `options` 时和 `mixed_input.quick_phrase` 相与；记在会话上是因为 `options` 里那个字段已经和能力与偏好相与过，认不出宿主原来要不要这项能力。
    quick_phrase_candidates_requested: bool,
    /// 原始宿主能力；偏好重建时不能从已经回退的有效布局推断它。
    korean_romaja_requested: bool,
    /// HostOptions 记录的版本：偏好里的方案只在本版本提供的方案里取，回退到本版本的默认方案。
    edition: &'static Edition,
    applied: Preferences,
    requested: Option<PreferencesSnapshot>,
    /// `requested` 的宿主状态（各个 setter、AI 配置缓存、音效设置）都已按它设好。上一次 `update` 在设到一半时出错，就还是 `false`，同一份快照再来时要完整重做，不能走「没变」的捷径。
    requested_settled: bool,
    /// Whether the requested preferences still need an Engine replacement. This decision is made
    /// when the document arrives so every keystroke does not compare the full preference tree.
    preferences_pending: bool,
    punctuation_override: Option<bool>,
    paired_punctuation_override: Option<bool>,
    punctuation_lock_override: Option<u8>,
    english_mode: bool,
    /// 宿主经 `msime_client_set_caps_lock` 报告的大写锁定状态。不报告的宿主一直是 `false`，`caps_lock_ascii_punctuation` 对它不起作用。
    caps_lock: bool,
    page_size_override: Option<u8>,
    /// 宿主经 `msime_client_set_key_grid` 或 `msime_client_set_nine_key_mode` 设下的组码网格，`Some(None)` 是宿主关掉了网格。只对设下它时的方案和布局有效，见 `apply_pending`。
    /// 宿主经 `msime_client_set_private_session` 标出的隐私会话（Android 的隐私模式和不允许学习的输入框，鸿蒙和 iOS 的隐私模式）：不记选词位置和上屏效率。与用户自己关掉的「学习」无关。
    statistics_private: bool,
    /// 只用来看候选的会话（`ffi::session::SessionUse::Preview`）：在任何平台上都不记选词位置和上屏效率。
    preview: bool,
    /// 宿主在会话选项里写了 `selection_statistics: false`：选词由另一个会话记，这个会话不记选词位置和上屏效率。
    statistics_recorded_elsewhere: bool,
    /// An AI provider credential the host keeps outside the preferences (the iOS Keychain), handed over for this session only and never written back.
    ai_credential: Option<SessionAiCredential>,
    /// Cached copy used by every online query until preferences change.
    ai_provider_cache: Option<AiAssistantProviderConfig>,
    /// The key sound, melody, commit, achievement and music settings of the newest preferences, which take effect at once rather than waiting for the composition to end: none of them is Engine state.
    sound: key_sound::SessionSound,
    /// 按需下载资源包所在的状态目录（HostOptions 的 `preferences_directory`），不是绝对路径时为空。
    state_root: Option<PathBuf>,
    /// HostOptions 记录的 `language_dictionaries` 目录：资源包里没有某个词库时退回这里的那份。
    recorded_language_dictionaries: Option<PathBuf>,
    /// 会话打开后有资源包新装好（或被移除），`options` 里的词典路径已经更新，Engine 还要在输入空闲时重建。
    resources_pending: bool,
    /// HostOptions 记录的键盘模型路径（`sentence_model`），为 `None` 时用资源目录里的那份；`neural_keyboard` 在会话中途打开时按它加载逐键重排，见 [`ffi::keyboard_reranker`]。
    recorded_sentence_model: Option<String>,
    /// 上一次挂逐键重排时 ONNX 模型的来源目录（[`ffi::onnx_rerank_source`]），没有 ONNX 模型或没开特性时为 `None`。聚焦时与此刻的来源比较：设置应用刚装好（或移除了）`rerank-model-small` 资源包时重新挂。
    keyboard_reranker_source: Option<PathBuf>,
    /// 宿主在会话选项里写了 `keyboard_reranker: false`（镜像会话）时为 `false`：「增强」打开也不挂逐键重排。
    keyboard_reranker_allowed: bool,
    /// 宿主在会话选项里写了 `pause_rerank: true`：跑不进逐键预算时把模型延后到停顿，见 [`ffi::RerankerMount::KeystrokeOrPause`]。
    pause_rerank: bool,
    /// HostOptions 记录的 ONNX 重排模型目录（`rerank_models`），为 `None` 时在随包资源目录旁边找，见 [`ffi::keyboard_reranker`]。
    recorded_rerank_models: Option<String>,
    /// HostOptions 记录的落定重排模型路径（`settled_model`）：随包的模型优先于下载的资源包，见 [`settled_model_file`]。
    recorded_settled_model: Option<String>,
    /// 当前挂在 runtime 上的落定重排模型文件，没有模型时为 `None`。
    settled_model: Option<PathBuf>,
    /// runtime 上是否真挂着落定重排模型：`settled_model` 有路径而文件读不了或不是可用的模型时为 `false`。只用于诊断，不看 runtime 自己的槽，因为后台评分进行中模型被评分任务借走，槽是空的。
    settled_model_attached: bool,
    /// 逐键重排器在后台线程里挂（[`load_keyboard_reranker`]）：第一次挂要加载 ONNX 模型、自测打一页的耗时，大小两个都装着时各一遍，Mac 上一秒多，忙时更久，放在输入线程上会把这段时间里的按键全堵住，超过 IMK 的等待时限后应用把键当作没处理，原样插进文字（#10743）。挂好前照常打字、不排序；槽有值后，下一次输入空闲的 `apply_pending` 换上并清掉这一项。为 `None` 时没有在挂的。
    keyboard_reranker_loading: Option<KeyboardRerankerSlot>,
    /// 聚焦时发现落定重排模型的文件变了（资源包刚装好或被移除），`settled_model` 已经更新，新模型在后台线程加载到这个槽里；槽有值后，下一次输入空闲的 `apply_pending` 把它换上并清掉这一项。为 `None` 时没有待换的模型。
    settled_model_loading: Option<SettledModelSlot>,
    /// 生命周期核心的状态，只由 `msime_client_set_lifecycle` 设上。为 `None` 时（移动端宿主从不调用它）不推导任何事件，transition JSON 逐字节不变。
    lifecycle: Option<Box<lifecycle::LifecycleState>>,
    /// 文档带 `read_only_state`（宿主对共享状态目录没有写权限，见 [`read_only_state`]）：不学习、回车也不学英文词，偏好变了重建 Engine 时也不打开它们；置顶、删除、固定排位和取消固定都拒绝。
    read_only_state: bool,
    // Declared after runtime so the Engine is dropped before releasing access.
    /// 只读宿主在只读布局下（词库就是随包资源，没有谁维护）不取租约，为 `None`；其余会话都持着。
    _dictionary_access: Option<DictionaryAccess>,
    /// 资源是下载的词库代次时占用它，设置宿主回收旧版本时不会删掉会话还在读的目录；同样放在 runtime 之后，Engine 先关掉文件再放手。
    _generation_lease: Option<crate::dictionary_updates::GenerationLease>,
}

#[derive(PartialEq, Eq)]
struct SessionAiCredential {
    endpoint: String,
    token: String,
}

/// 本地模式由偏好开关，但它们读的词典是不可变的运行时资源。可选资源不在时，触发键不能被吞掉：在资源齐全之前，Engine 看到的这个模式必须是关的。
///
/// 版本不提供的功能同样当作资源不在：五笔版的资源锁不带日文词典和键盘神经模型，临时日文、日文混输和键盘神经联想无论偏好怎么写都是关的。资源目录按版本的锁校验，本来就放不进日文词典，这里再按版本表关一次，是为了状态目录里留有下载来的日文资源包时也不会打开它。
/// 偏好里拆字开关的取值：用户没动过时按宿主形态决定（见 `LocalModePreferences::chaizi`）——触屏宿主读作开（屏幕键盘从菜单进入），桌面宿主读作关（打开后句首和中英混打的大写 C 会进入模式）。Windows v0.11.1 的 `radical_mode` 出厂为开，与本仓桌面缺省不同，差异已记录，是否对齐留待产品决策。
const TOUCH_KEYBOARD_HOST: bool = cfg!(any(
    target_os = "android",
    target_os = "ios",
    target_env = "ohos"
));

fn chaizi_enabled(preferences: &msime_client_core::preferences::LocalModePreferences) -> bool {
    preferences.chaizi.unwrap_or(TOUCH_KEYBOARD_HOST)
}

/// 「移动光标后编辑光标后的部分」只在三个桌面宿主上交给 Engine：偏好随同步到达手机时，触屏宿主没有设置入口、也不画模式提示，所以那里一律按关。
const CARET_EDIT_SUFFIX_HOST: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos"))
));

fn caret_edit_suffix_applies(preferences: &Preferences) -> bool {
    CARET_EDIT_SUFFIX_HOST && preferences.caret_edit_suffix
}

/// 请求的偏好与会话已应用的是否不同到需要重建 Engine。持续上报授权的历史（`usage_reporting_consent`）不算：它只属于本机，由上报观察线程自己从偏好文件读（`ffi/reporting.rs`），Engine 和会话都不用；宿主交来的选项文档多半不带它，算进去会让每个新会话第一次读偏好时白白排上一次重建，这次重建落在第一次上屏之后，把刚出来的上屏后联想清掉（Linux IBus 上实测）。
fn session_preferences_differ(requested: &Preferences, applied: &Preferences) -> bool {
    if requested.usage_reporting_consent == applied.usage_reporting_consent {
        return requested != applied;
    }
    let mut requested = requested.clone();
    requested.usage_reporting_consent = applied.usage_reporting_consent;
    requested != *applied
}

/// 上屏后联想只在接好了显示和按键的宿主上交给 Engine（`HostPlatform::shows_next_word_prediction`）：别的宿主收到联想会把它当成组字，数字、空格或回车可能把用户没选的词上屏。偏好随同步到达这些宿主时一律按关。
fn next_word_prediction_applies(preferences: &Preferences) -> bool {
    ffi::host::compiled_platform().shows_next_word_prediction() && preferences.next_word_prediction
}

fn apply_local_mode_resource_gates(options: &mut EngineOptions, edition: &Edition) {
    let resources = std::path::Path::new(&options.resources);
    let has_emoji_catalog = resources.join("msime-others.db").is_file();
    let has_english_dictionary = resources.join("msime-english.db").is_file();
    options.local_emoji &= has_emoji_catalog;
    options.local_kaomoji &= has_emoji_catalog;
    options.local_temporary_english &= has_english_dictionary;
    // 引擎里的键盘模型读的是同一份随版本资源锁走的模型；宿主不打开它（见 `engine_sentence_association`），不带模型的版本更不交给 Engine。
    options.sentence_association.neural_keyboard &= edition.features.neural_keyboard;
}

/// 整句联想「增强」挂不挂 runtime 的逐键重排器（[`ffi::keyboard_reranker`]）：偏好打开、且本版本带键盘模型。它只管 runtime 这一层，引擎里的键盘模型不跟这个开关走，见 `engine_sentence_association`。
fn keyboard_reranker_enabled(
    preferences: &msime_client_core::preferences::SentenceAssociationPreferences,
    edition: &Edition,
) -> bool {
    preferences.neural_keyboard && edition.features.neural_keyboard
}

fn punctuation_lock_code(lock: msime_client_core::preferences::PunctuationLock) -> u8 {
    match lock {
        msime_client_core::preferences::PunctuationLock::Follow => 0,
        msime_client_core::preferences::PunctuationLock::Chinese => 1,
        msime_client_core::preferences::PunctuationLock::English => 2,
    }
}

/// The Engine's sentence association switches. Neither model switch reaches it: the desktop one gates the runtime's settled reranker (`Runtime::set_settled_rerank_enabled`), the only place that model runs, and the keyboard one (整句联想「增强」) mounts the runtime's per-keystroke reranker (`keyboard_reranker_enabled`), so the Engine's own keyboard-model rerank stays off.
fn engine_sentence_association(
    preferences: &msime_client_core::preferences::SentenceAssociationPreferences,
    _state_root: Option<&std::path::Path>,
) -> msime_engine::SentenceAssociationOptions {
    // 语法模型按资源包下载的机制已移除；没有随包的搭配模型，引擎逐字节退回无模型基线。
    msime_engine::SentenceAssociationOptions {
        word_lattice: preferences.word_lattice,
        // 引擎自己那层键盘模型（九键跨切分重排、全拼和双拼词库内重排）不跟「增强」打开：在 runtime 的逐键重排之外它几乎不加分，却让按键延迟和内存再涨一截，取舍见 .agents/notes/implemented/feature/2026-10-29-enhanced-runtime-reranker-only.md。它留给直接构造 `EngineOptions` 的评测和实验（`convert_eval`）。
        neural_keyboard: false,
        show_next_on_duplicate: preferences.show_next_on_duplicate,
        collocation_model: PathBuf::new(),
        collocation_rerank: false,
        // 搭配调参系数逐字段透传；偏好层已把非法值回退到出厂值。
        collocation_weight: preferences.sentence_collocation_weight,
        collocation_grammar: msime_engine::octagram::GramConfig {
            collocation_max_length: preferences.sentence_collocation_max_length,
            collocation_min_length: preferences.sentence_collocation_min_length,
            collocation_penalty: preferences.sentence_collocation_penalty,
            non_collocation_penalty: preferences.sentence_collocation_non_penalty,
            weak_collocation_penalty: preferences.sentence_collocation_weak_penalty,
            rear_penalty: preferences.sentence_collocation_rear_penalty,
        },
    }
}

/// 资源包卸载前摘掉引擎进程内的 `.gram` 映射，避免 Windows 删除目录时仍被文件句柄占用。
pub fn evict_grammar_model(path: &std::path::Path) {
    msime_engine::octagram::GramDb::evict_shared(path);
}

/// 引擎的引号与方头括号样式，由 client-core 从偏好解析（`corner_when_traditional` 在那里按繁体开关定下）。View 报告的是同一个解析结果。
fn apply_punctuation_marks(
    options: &mut EngineOptions,
    preferences: &msime_client_core::preferences::Preferences,
) {
    let marks = msime_client_core::punctuation::punctuation_marks(preferences);
    options.corner_quotes = marks.quotes == msime_client_core::punctuation::QuoteMarks::Corner;
    options.hollow_brackets =
        marks.brackets == msime_client_core::preferences::BracketStyle::Hollow;
    options.enumeration_comma_backslash = marks.enumeration_comma_backslash;
    options.enumeration_comma_slash = marks.enumeration_comma_slash;
}

/// The switch the Engine is handed. Windows turns the punctuation switch on whenever punctuation is locked to Chinese, while the Engine drops Chinese punctuation whenever its switch is off, so a lock to Chinese has to carry the switch with it.
fn engine_chinese_punctuation(enabled: bool, lock: u8) -> bool {
    enabled || lock == 1
}

impl HostSession {
    /// 判断一个标点键去向所用的上下文。`msime_client_punctuation_with_context` 和大写锁定改道（[`Self::caps_lock_punctuation`]）用的是同一份。
    fn punctuation_context(&self, character: u8, preceding: Option<char>) -> PunctuationContext {
        let lock = match self.punctuation_lock_override {
            Some(1) => msime_client_core::preferences::PunctuationLock::Chinese,
            Some(2) => msime_client_core::preferences::PunctuationLock::English,
            Some(_) => msime_client_core::preferences::PunctuationLock::Follow,
            None => self.applied.punctuation_lock,
        };
        PunctuationContext {
            character,
            preceding,
            host_context_available: self
                .runtime
                .punctuation_host_context_available(self.english_mode),
            has_composition: !self.runtime.is_idle(),
            chinese_punctuation: self
                .punctuation_override
                .unwrap_or(self.applied.chinese_punctuation),
            smart_punctuation: self.applied.smart_punctuation,
            direct_digit: self.applied.smart_punctuation_direct_digit,
            direct_letter: self.applied.smart_punctuation_direct_letter,
            lock,
            caps_lock_ascii: self.caps_lock && self.applied.caps_lock_ascii_punctuation,
        }
    }

    /// 大写锁定时，把经字符或标点入口送来的标点键改走字面 ASCII 路线（#6370）。宿主大多把标点键当普通字符交给 `msime_client_character`，这条路线上没有标点判断，所以在分发前统一改道，每个报告了大写锁定的宿主都不用再写一份。只看大写锁定这一条：智能标点要读前文，不在这里判断。不改道时原样返回。
    fn caps_lock_punctuation(&self, action: Action) -> Action {
        if !(self.caps_lock && self.applied.caps_lock_ascii_punctuation) {
            return action;
        }
        let ascii = match action {
            Action::Character { value, .. } | Action::Punctuation(value)
                if value.is_ascii_punctuation() =>
            {
                value
            }
            _ => return action,
        };
        let mut context = self.punctuation_context(ascii, None);
        context.smart_punctuation = false;
        match punctuation_route(context) {
            PunctuationRoute::Ascii => Action::PunctuationAscii(ascii),
            PunctuationRoute::Engine => action,
        }
    }

    /// `engine_chinese_punctuation` for the live overrides over the applied preferences.
    fn live_engine_chinese_punctuation(&self) -> bool {
        engine_chinese_punctuation(
            self.punctuation_override
                .unwrap_or(self.applied.chinese_punctuation),
            self.punctuation_lock_override
                .unwrap_or_else(|| punctuation_lock_code(self.applied.punctuation_lock)),
        )
    }

    fn ai_provider_config(&self) -> Option<&AiAssistantProviderConfig> {
        self.ai_provider_cache.as_ref()
    }
    fn ai_query_is_current(&self, query: &OnlineQuery) -> bool {
        self.english_query_allowed(query)
            && self
                .ai_provider_config()
                .is_some_and(|config| query.ai_assistant.as_ref() == Some(config))
    }
    fn english_query_allowed(&self, query: &OnlineQuery) -> bool {
        if query.mode != msime_engine::OnlineInputMode::English {
            return true;
        }
        let enabled = self
            .requested
            .as_ref()
            .map_or(self.applied.english_online_candidates, |snapshot| {
                snapshot.preferences.english_online_candidates
            });
        enabled
            && self.runtime.online_query().is_ok_and(|live| {
                live.is_some_and(|live| {
                    live.mode == query.mode
                        && live.scheme == query.scheme
                        && live.session_id == query.session_id
                        && live.generation == query.generation
                        && live.identity == query.identity
                        && live.query_text == query.query_text
                        && live.cache_key == query.cache_key
                        && live.pinyin_segments == query.pinyin_segments
                        && live.ai_context == query.ai_context
                        && live.cloud_eligible == query.cloud_eligible
                        && live.ai_eligible == query.ai_eligible
                })
            })
    }
    fn set_ai_provider_cache(&mut self, preferences: &Preferences) {
        let ai = &preferences.ai_assistant;
        self.ai_provider_cache = ai.enabled.then(|| AiAssistantProviderConfig {
            enabled: true,
            provider: ai.provider.clone(),
            model: ai.model.clone(),
            endpoint: ai.endpoint.clone(),
            candidate_limit: host_ai_candidate_limit(ai.candidate_limit),
            thinking: ai.thinking.clone(),
            prompt_id: ai.prompt_id.clone(),
            prompt_custom_1: ai.prompt_custom_1.clone(),
            prompt_custom_2: ai.prompt_custom_2.clone(),
            prompt_custom_3: ai.prompt_custom_3.clone(),
        });
    }
    /// 以最新请求的开关与来源配置判断回包；组字结束才应用的旧偏好不能让已关闭或已切换的服务继续回答。
    fn cloud_query_is_current(&self, query: &OnlineQuery) -> bool {
        let preferences = self
            .requested
            .as_ref()
            .map_or(&self.applied, |snapshot| &snapshot.preferences);
        self.english_query_allowed(query)
            && self.cloud_candidates_enabled()
            && query.cloud_request.as_ref().map_or_else(
                || {
                    query.mode != msime_engine::OnlineInputMode::English
                        && preferences.cloud_candidate_provider == Default::default()
                },
                |request| request.provider_id == preferences.cloud_candidate_provider.identity(),
            )
    }

    fn cloud_candidates_enabled(&self) -> bool {
        self.requested
            .as_ref()
            .map_or(self.applied.cloud_candidates, |snapshot| {
                snapshot.preferences.cloud_candidates
            })
    }
    /// 只看候选的会话、宿主声明由别的会话记统计的会话，以及隐私会话不做预测。
    fn private_session(&self) -> bool {
        self.preview || self.statistics_recorded_elsewhere || self.statistics_private
    }

    /// Rebuild the Engine for the requested preferences once the composition is idle. The `Ok` value is why the preferred scheme was not the one applied, when it was not.
    fn apply_pending(&mut self) -> Result<Option<String>, String> {
        if self.runtime.is_idle() {
            if let Some(size) = self.page_size_override {
                // 这里只改分页本身，引擎的首屏门槛由 `Runtime::set_page_size` 一并下发（它读的是同一份 `page_size`），
                // 所以 override 的变化不必触发重建。fcitx5 的「9 个候选」和 IBus 的每页候选数菜单都会写它，见 `ffi/input.rs`。
                self.runtime
                    .set_page_size(host_page_size(size))
                    .map_err(|e| e.to_string())?;
            }
            // 落定重排模型不属于 Engine，换模型不用重建 Engine。约 25 MB 的权重由 `refresh_resource_packs` 起的后台线程加载，这里只在加载完之后、输入空闲时换上，不在输入线程上读文件；还没加载完就留到下一次。
            // 后台挂好的逐键重排器同样在输入空闲时换上：组字中途换了打分器，整句池跟着变，正在看的候选会忽然重排。
            let attached = self.keyboard_reranker_loading.as_ref().and_then(|slot| {
                slot.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
            });
            if let Some(mounted) = attached {
                self.keyboard_reranker_loading = None;
                ffi::MountedReranker::mount(mounted, &mut self.runtime);
            }
            if let Some(model) = self
                .settled_model_loading
                .as_ref()
                .and_then(|slot| slot.get().cloned())
            {
                self.settled_model_loading = None;
                self.settled_model_attached = model.is_some();
                self.runtime.set_settled_reranker(model.map(Reranker::new));
                self.report_settled_model();
            }
        }
        // 上屏后联想也算空闲，但重建 Engine 会清掉它：等联想结束（下一个按键或选中之外的任何操作）再重建。
        if !(self.preferences_pending || self.resources_pending)
            || !self.runtime.is_idle()
            || self.runtime.shows_prediction()
        {
            return Ok(None);
        }
        // 只因资源包变化而重建时，没有新请求的偏好，按已应用的那份重建。
        let preferences = self
            .requested
            .as_ref()
            .map(|snapshot| &snapshot.preferences)
            .unwrap_or(&self.applied)
            .clone();
        let mut options = self.options.clone();
        let (scheme, scheme_fallback) = effective_scheme(
            &preferences,
            &offered_input_schemes(self.edition),
            &LanguageDictionaries::of_options(&self.options),
            self.edition.default_scheme,
        );
        options.scheme = scheme_code(scheme);
        let profile_fallback = apply_shuangpin_profile(&mut options, &preferences);
        // 自定义双拼表不能用的原因只在真的跑双拼时才值得报告。
        let fallback =
            scheme_fallback.or(profile_fallback.filter(|_| scheme == InputScheme::Shuangpin));
        options.shuangpin_preedit_uses_raw = preferences.shuangpin_preedit_uses_raw;
        options.shuangpin_preedit_separated = preferences.shuangpin_preedit_separated;
        options.single_character_only = preferences.single_character_only;
        options.caret_edit_suffix = caret_edit_suffix_applies(&preferences);
        options.learning = preferences.learning && !self.read_only_state;
        options.next_word_prediction = next_word_prediction_applies(&preferences);
        options.learn_english_on_enter =
            preferences.enter_learns_english_word && !self.read_only_state;
        options.autocorrect_transposition = preferences.quanpin.autocorrect_transposition;
        options.autocorrect_neighbor = preferences.quanpin.autocorrect_neighbor;
        options.fuzzy_pinyin_rules = preferences.fuzzy_pinyin.active_rules();
        options.frequency_mode = preferences.frequency.mode.as_str().into();
        options.frequency_trigger_count = preferences.frequency.trigger_count;
        options.frequency_linear_step = preferences.frequency.linear_step;
        options.mixed_candidates = preferences.mixed_input.enabled;
        options.mixed_english = preferences.mixed_input.english;
        options.english_minimum_prefix =
            host_english_minimum_prefix(preferences.mixed_input.minimum_prefix);
        options.english_first_page_count = preferences.mixed_input.first_page_count;
        options.mixed_emoji = preferences.mixed_input.emoji;
        options.mixed_kaomoji = preferences.mixed_input.kaomoji;
        options.mixed_symbols = preferences.mixed_input.symbols;
        options.quick_phrase_candidates =
            self.quick_phrase_candidates_requested && preferences.mixed_input.quick_phrase;
        options.quick_phrase_frequency = preferences.mixed_input.quick_phrase_frequency;
        options.local_unicode = preferences.local_modes.unicode;
        options.local_date_time = preferences.local_modes.date_time;
        options.local_date_time_inline = preferences.local_modes.date_time_inline;
        options.local_date_time_menu = preferences.local_modes.date_time_menu;
        options.local_quick_phrase = preferences.local_modes.quick_phrase;
        options.local_emoji = preferences.local_modes.emoji;
        options.local_kaomoji = preferences.local_modes.kaomoji;
        options.local_super_jianpin = preferences.local_modes.super_jianpin;
        options.local_super_jianpin_slash = preferences.local_modes.super_jianpin_slash;
        options.local_temporary_english = preferences.local_modes.temporary_english;
        options.local_temporary_japanese = preferences.local_modes.temporary_japanese;
        options.local_expression = preferences.local_modes.expression;
        options.local_command = preferences.local_modes.command;
        options.local_mention = preferences.local_modes.mention;
        options.local_chaizi = chaizi_enabled(&preferences.local_modes);
        options.local_symbol = preferences.local_modes.symbol;
        options.local_shift_letter_entry = preferences.local_modes.shift_letter_entry;
        // 繁体输出开着时拆字行按繁体字形去重；改这个偏好同样会让 Engine 在空闲时重建。
        options.chaizi_traditional_output = preferences.traditional_chinese_output;
        // 先定下辅助码设置：插件表的戳要看当前方案的辅助码是否打开、选了哪个辅助码表包。
        let helpcode = helpcode_for_scheme(&preferences, scheme);
        options.helpcode = helpcode.enabled;
        options.strict_quanpin_helpcode = helpcode.strict_filter;
        options.show_helpcode = helpcode.show_in_candidate_window;
        options.helpcode_schema = helpcode.schema.as_str().into();
        options.helpcode_trigger_key = helpcode.trigger_key.bytes().next();
        options.shuangpin_helpcode_modes = shuangpin_helpcode_modes(helpcode.modes);
        options.sentence_association = engine_sentence_association(
            &preferences.sentence_association,
            self.state_root.as_deref(),
        );
        // 引擎按宿主的每页候选数决定全长组要不要并入补全词；和下面交给 runtime 的值同源。
        let page_size = host_page_size(
            self.page_size_override
                .unwrap_or(preferences.candidate_page_size),
        );
        options.first_page_rows = usize::from(page_size);
        // Unconditional, because `Runtime::crop_alternative_readings` runs whether or not a model is
        // attached: the host always shows one whole-sentence reading. Asking for the rest only ever
        // gives it more to choose from, and even with no model the engine's own pick among them is
        // better than the one it makes when it searches without alternatives.
        options.sentence_alternatives = true;
        apply_local_mode_resource_gates(&mut options, self.edition);
        options.paired_punctuation = preferences.paired_punctuation;
        options.punctuation_lock = punctuation_lock_code(preferences.punctuation_lock);
        options.chinese_punctuation =
            engine_chinese_punctuation(preferences.chinese_punctuation, options.punctuation_lock);
        // 繁体输出开关一变，`corner_when_traditional` 解析出的样式也跟着变；两者都是偏好，都会在空闲时走到这里重建。
        apply_punctuation_marks(&mut options, &preferences);
        // 先构建并校验，失败时原会话仍然可用。
        let mut engine = Session::new(&options).map_err(|e| e.to_string())?;
        engine.set_prediction_allowed(!self.statistics_private);
        // 云候选席位不是 EngineOptions；新引擎默认关闭，每次重建都要恢复当前偏好。
        engine.set_cloud_candidates_enabled(preferences.cloud_candidates);
        engine.set_english_online_candidates_enabled(preferences.english_online_candidates);
        if self.punctuation_override.is_some() || self.punctuation_lock_override.is_some() {
            engine
                .set_chinese_punctuation_enabled(engine_chinese_punctuation(
                    self.punctuation_override
                        .unwrap_or(preferences.chinese_punctuation),
                    self.punctuation_lock_override
                        .unwrap_or(options.punctuation_lock),
                ))
                .map_err(|e| e.to_string())?;
        }
        if let Some(enabled) = self.paired_punctuation_override {
            engine
                .set_paired_punctuation_enabled(enabled)
                .map_err(|e| e.to_string())?;
        }
        if let Some(lock) = self.punctuation_lock_override {
            engine
                .set_punctuation_lock(lock)
                .map_err(|e| e.to_string())?;
        }
        engine
            .set_dedicated_english(self.english_mode)
            .map_err(|e| e.to_string())?;
        // The places of `@` mode are not an engine option: every new engine starts with them off, so the switch is carried over on each rebuild.
        engine
            .set_mention_places(preferences.local_modes.mention_places)
            .map_err(|e| e.to_string())?;
        self.runtime
            .replace_engine(engine, page_size)
            .map_err(|e| e.to_string())?;
        let desktop_turned_on = preferences.sentence_association.neural_desktop
            && !self.applied.sentence_association.neural_desktop;
        self.runtime
            .set_settled_rerank_enabled(preferences.sentence_association.neural_desktop);
        self.runtime
            .set_punctuation_marks(msime_client_core::punctuation::punctuation_marks(
                &preferences,
            ));
        // The keyboard model follows its switch the same way, but is dropped rather than idled while off: it is the one that runs inside every keystroke.
        let keyboard = self.keyboard_reranker_allowed
            && keyboard_reranker_enabled(&preferences.sentence_association, self.edition);
        if keyboard != (self.runtime.has_reranker() || self.keyboard_reranker_loading.is_some()) {
            if keyboard {
                self.keyboard_reranker_loading = Some(load_keyboard_reranker(
                    &options.resources,
                    options.helpcode_fallback_resources.as_deref(),
                    self.recorded_rerank_models.as_deref(),
                    None,
                    self.state_root.as_deref(),
                    self.recorded_sentence_model.as_deref(),
                    self.pause_rerank,
                ));
            } else {
                self.keyboard_reranker_loading = None;
                self.runtime.set_candidate_scorer(ffi::keyboard_reranker(
                    false,
                    &options.resources,
                    options.helpcode_fallback_resources.as_deref(),
                    self.recorded_rerank_models.as_deref(),
                    None,
                    self.state_root.as_deref(),
                    self.recorded_sentence_model.as_deref(),
                ));
            }
            self.keyboard_reranker_source = ffi::onnx_rerank_source(
                &options.resources,
                options.helpcode_fallback_resources.as_deref(),
                self.recorded_rerank_models.as_deref(),
                None,
                self.state_root.as_deref(),
            );
        }
        self.options = options;
        self.applied = preferences;
        self.preferences_pending = false;
        self.resources_pending = false;
        // 会话建立时用的是运行时选项里嵌的偏好快照，开关可能要等宿主随后送来的偏好才打开（#10341 里报告者的 `runtime-options.json` 就写着关），所以打开的这一刻再报告一次模型接上没有。新模型还在后台加载时不报告，加载完换上时会报告。
        if desktop_turned_on && self.settled_model_loading.is_none() {
            self.report_settled_model();
        }
        Ok(fallback)
    }

    /// 开着「桌面神经联想」的输入会话把落定重排模型接上了没有，写一行诊断（见 [`report_settled_model`]）。只看候选的会话和开关关着的会话不写。
    fn report_settled_model(&self) {
        if self.preview || !self.runtime.settled_rerank_enabled() {
            return;
        }
        report_settled_model(
            &self.options.resources,
            self.state_root.as_deref(),
            self.settled_model.as_deref(),
            self.settled_model_attached,
        );
    }

    /// 「中英文之间加空格」开关，跟着最新请求的偏好走：它只是上屏后处理，不是 Engine 状态，不必等组字结束后重建。
    fn chinese_english_spacing(&self) -> bool {
        self.requested
            .as_ref()
            .map_or(&self.applied, |snapshot| &snapshot.preferences)
            .chinese_english_spacing
    }

    fn complete_transition(&mut self, mut result: Transition) -> Transition {
        let generation = self.runtime.generation();
        let note = match self.apply_pending() {
            Ok(fallback) => fallback,
            Err(error) => Some(format!("Preferences update deferred: {error}")),
        };
        if let Some(note) = note {
            let prior = result.diagnostic.take().unwrap_or_default();
            result.diagnostic = Some(format!("{prior} {note}").trim().to_owned());
        }
        // 替换只改变 view 的代次，不改变已完成的上屏。不转全角的方案（韩文）无论宽度开关如何都写半角 ASCII 标点和数字；专用英文模式在任何方案里都按自己的规则走，所以它的上屏和中文方案下一样转全角。网址模式的网址部分始终是半角：上屏后 view 已回到 `none`，所以看上屏前记下的 `commit_context`；结束网址的那个标点不是网址能收的字符，照常转全角。
        let half_width_text = SchemeType::from_u8(result.view.scheme)
            .is_some_and(|scheme| !scheme.widens_full_width())
            && !result.view.dedicated_english
            && result.view.local_mode == "none";
        let url_commit = result
            .commit_context
            .as_ref()
            .is_some_and(|context| context.local_mode == "url");
        // 脚本插件在 `/` 指令里给的行（UUID 之类）原样上屏，不转全角：转了以后上屏的和候选窗里显示的不一样。同一次 `/` 模式里内置指令（`/rq`）的上屏照旧转全角。
        let plugin_commit = result
            .commit_context
            .as_ref()
            .is_some_and(|context| context.plugin_row);
        // 位置以原上屏的真实后缀为准；失配时不猜测编辑器里的光标偏移。
        if result.commit_cursor_suffix.as_ref().is_some_and(|suffix| {
            !result
                .commit
                .as_ref()
                .is_some_and(|commit| commit.ends_with(suffix))
        }) {
            result.commit_cursor_suffix = None;
        }
        if result.view.character_width == CharacterWidth::Fullwidth
            && !half_width_text
            && !plugin_commit
        {
            for c in result
                .commit
                .iter_mut()
                .chain(result.commit_cursor_suffix.iter_mut())
            {
                *c = c
                    .chars()
                    .map(|x| {
                        if url_commit && x.is_ascii() && msime_engine::url::accepts(x as u8) {
                            x
                        } else if x == ' ' {
                            '\u{3000}'
                        } else if ('!'..='~').contains(&x) {
                            char::from_u32(x as u32 + 0xfee0).unwrap()
                        } else {
                            x
                        }
                    })
                    .collect();
            }
        }
        // 中英文之间加空格排在全角转换之后：先加的半角空格会被转成 U+3000；全角下字母已是全角，本来就不会触发。宿主的简繁转换在拿到上屏文本之后才做，OpenCC 只换汉字、不动空格。之后的 `before_commit` 看到的是加过空格的文本。
        if self.chinese_english_spacing()
            && chinese_english_spacing::applies(result.commit_context.as_ref())
        {
            // 分别转换正文与后缀。边界新增的空格留在光标前，光标仍紧邻原后缀的首字符。
            for commit in result
                .commit
                .iter_mut()
                .chain(result.commit_cursor_suffix.iter_mut())
            {
                if let Some(spaced) = chinese_english_spacing::space(commit) {
                    *commit = spaced;
                }
            }
        }
        if result.commit_cursor_suffix.as_ref().is_some_and(|suffix| {
            !result
                .commit
                .as_ref()
                .is_some_and(|commit| commit.ends_with(suffix))
        }) {
            result.commit_cursor_suffix = None;
        }
        // Dispatch already built the view for this transition. Rebuild it only
        // when applying a deferred page-size or preference change advanced the
        // runtime generation; otherwise cloning the page again costs every key.
        if self.runtime.generation() != generation {
            result.view = self.runtime.view();
        }
        result
    }

    fn update(&mut self, snapshot: PreferencesSnapshot) -> Result<Value, String> {
        if snapshot.format_version != 1 {
            return Err("unsupported preferences format".into());
        }
        // Android 键盘显示期间每秒把读到的偏好原样交来一次，绝大多数时候和上一份逐字相同。上一份已经校验过、所有 setter 已经按它设过（这些宿主状态只有偏好会改，`msime_client_set_multilingual_output` 的临时投影除外），`preferences_pending` 也一直等于「它与 `applied` 不同」，所以这里只补上会被别处改动的那几样，不再校验、比较整棵偏好、克隆偏好和重建音效设置。
        if self.requested_settled
            && self.requested.as_ref().is_some_and(|previous| {
                previous.revision == snapshot.revision && *previous == snapshot
            })
        {
            return self.update_unchanged();
        }
        snapshot.preferences.validate().map_err(|e| e.to_string())?;
        if let Some(previous) = &self.requested {
            if snapshot.revision < previous.revision
                || (snapshot.revision == previous.revision && snapshot != *previous)
            {
                return Err("stale or conflicting preferences revision".into());
            }
        }
        let ai_changed = self
            .requested
            .as_ref()
            .map_or(&self.applied.ai_assistant, |previous| {
                &previous.preferences.ai_assistant
            })
            != &snapshot.preferences.ai_assistant;
        let cloud_disabled = self
            .requested
            .as_ref()
            .map_or(self.applied.cloud_candidates, |previous| {
                previous.preferences.cloud_candidates
            })
            && !snapshot.preferences.cloud_candidates;
        let cloud_changed = self
            .requested
            .as_ref()
            .map_or(&self.applied.cloud_candidate_provider, |previous| {
                &previous.preferences.cloud_candidate_provider
            })
            != &snapshot.preferences.cloud_candidate_provider;
        if (cloud_disabled || cloud_changed) && !self.runtime.is_idle() {
            // 关闭云候选或切换来源配置时移除旧云行，使迟到回包失效。
            self.runtime
                .clear_online_candidates(0)
                .map_err(|e| e.to_string())?;
        }
        let previous_preferences = self
            .requested
            .as_ref()
            .map_or(&self.applied, |previous| &previous.preferences);
        if ai_changed && !self.runtime.is_idle() {
            // AI 配置或令牌变化时先清理旧候选并使旧查询失效，再发布新偏好。
            self.runtime
                .clear_online_candidates(1)
                .map_err(|e| e.to_string())?;
        }
        self.preferences_pending = session_preferences_differ(&snapshot.preferences, &self.applied);
        self.requested = Some(snapshot);
        self.requested_settled = false;
        let requested_preferences = self
            .requested
            .as_ref()
            .expect("requested snapshot exists")
            .preferences
            .clone();
        self.set_ai_provider_cache(&requested_preferences);
        self.runtime
            .set_english_online_candidates_enabled(requested_preferences.english_online_candidates)
            .map_err(|e| e.to_string())?;
        self.runtime
            .set_cloud_candidates_enabled(requested_preferences.cloud_candidates)
            .map_err(|e| e.to_string())?;
        // 宿主状态，不是 Engine 状态：跟着最新请求的偏好走，不必等这次偏好重建 Engine，也不必等组字结束。
        self.runtime
            .set_backspace_reverts_selection(requested_preferences.backspace_reverts_selection);
        // Esc 保留已选词同样是宿主状态：改完下一次 Esc 就按新开关解释。
        self.runtime
            .set_escape_keeps_selected_word(requested_preferences.escape_keeps_selected_word);
        // 左右键的候选回退（`navigation.left_right`）同样是宿主状态：改完立刻生效，左右键不必等 Engine 重建。
        self.runtime
            .set_left_right_navigation(requested_preferences.navigation.left_right);
        self.runtime
            .set_vertical_candidates(host_vertical_candidates(
                requested_preferences.candidate_layout,
            ));
        self.runtime
            .set_wubi_auto_commit_unique(requested_preferences.wubi_auto_commit_unique);
        // 首页英文候选数同样是宿主状态：在线座位整理按它给英文组留位，改完下一次刷新就生效。
        self.runtime
            .set_english_first_page_count(requested_preferences.mixed_input.first_page_count);
        // 顶字同样是宿主状态：纯判定在 Engine 与 runtime 之间，改偏好不必等组字结束。
        self.runtime
            .set_wubi_fifth_code_top_commit(requested_preferences.wubi_fifth_code_top_commit);
        self.runtime
            .set_wubi_candidate_selection(requested_preferences.wubi_candidate_selection.clone());
        // 「二三候选」键也是宿主编排状态：跟着最新请求的偏好走，下一键生效。
        self.runtime
            .set_second_third_candidate(requested_preferences.second_third_candidate);
        self.sound
            .update(key_sound::SoundSettings::new(&requested_preferences.plugins));
        self.requested_settled = true;
        let fallback = self.apply_pending()?;
        Ok(self.preferences_response(fallback))
    }

    /// `update` 收到与上一份逐字相同的快照：只做可能因别处改动而需要重做的事，回复与完整更新相同。
    fn update_unchanged(&mut self) -> Result<Value, String> {
        let snapshot = self.requested.as_ref().expect("requested snapshot exists");
        // 设置页可能重新导入或删掉了正在用的音效包、效果包：只给清单盖戳，戳变了才重读效果包，和完整更新里新建的设置结果相同。
        self.sound.restamp();
        // 组字结束后待应用的偏好、资源包和落定重排模型仍在这里换上。
        let fallback = self.apply_pending()?;
        Ok(self.preferences_response(fallback))
    }

    /// `update` 的回复：宿主都会读其中的 `view`。
    fn preferences_response(&self, fallback: Option<String>) -> Value {
        let snapshot = self.requested.as_ref().expect("requested snapshot exists");
        let mut response = json!({ "revision": snapshot.revision, "deferred": session_preferences_differ(&snapshot.preferences, &self.applied), "view": self.runtime.view(), "floating_toolbar": { "enabled": snapshot.preferences.floating_toolbar.enabled, "english_mode": snapshot.preferences.floating_toolbar.english_mode, "scale_percent": snapshot.preferences.floating_toolbar.scale_percent, "font_size": snapshot.preferences.floating_toolbar.font_size, "fullwidth": snapshot.preferences.floating_toolbar.fullwidth, "punctuation": snapshot.preferences.floating_toolbar.punctuation, "character_set": snapshot.preferences.floating_toolbar.character_set, "emoji": snapshot.preferences.floating_toolbar.emoji, "screen_keyboard": snapshot.preferences.floating_toolbar.screen_keyboard, "settings": snapshot.preferences.floating_toolbar.settings } });
        // Applied at once, so no transition will carry the reason the preferred scheme was replaced; a deferred change reports it from `complete_transition` instead.
        if let Some(fallback) = fallback {
            response["diagnostic"] = fallback.into();
        }
        response
    }
}

fn profile_code(profile: ShuangpinProfile) -> u8 {
    match profile {
        ShuangpinProfile::Xiaohe => 0,
        ShuangpinProfile::Ziranma => 1,
        ShuangpinProfile::Shoudao => 2,
        ShuangpinProfile::Microsoft => 3,
        ShuangpinProfile::SmartAbc => 5,
        ShuangpinProfile::Custom => 4,
    }
}

/// 偏好里的自定义双拼表换成 Engine 的表。
fn engine_shuangpin_custom_table(profile: &ShuangpinCustomProfile) -> ShuangpinCustomTable {
    let pairs = |part: &std::collections::BTreeMap<String, String>| {
        part.iter()
            .map(|(unit, key)| (unit.clone(), key.clone()))
            .collect()
    };
    ShuangpinCustomTable {
        initials: pairs(&profile.initials),
        finals: pairs(&profile.finals),
        zero_initials: pairs(&profile.zero_initials),
    }
}

/// 偏好里的自定义双拼表能不能用，由 Engine 判定；不能用时是给日志和代理看的英文原因。设置页、MCP 等写偏好的一方保存之前用它校验。
pub fn validate_custom_shuangpin_profile(profile: &ShuangpinCustomProfile) -> Result<(), String> {
    validate_shuangpin_custom_profile(&engine_shuangpin_custom_table(profile))
        .map_err(|error| error.to_string())
}

/// 自定义双拼表为什么不能用，按 Engine 的 `ShuangpinCustomProfileError` 逐项摊开，给设置页按种类写自己的提示、标出出错的那一格。Engine 在第一个错误处就停，所以一次只有一条。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShuangpinCustomProblem {
    pub kind: ShuangpinCustomProblemKind,
    /// 出错的是表的哪一部分：`initials`、`finals` 或 `zero_initials`。编码冲突不属于某一部分，没有这一项；多字母声母占了别的键时是 `initials`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<&'static str>,
    /// 出错的单位，例如 `zh`、`iong`、`er`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// 不合法或已被占用的键（已转成小写）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// 两个音节撞上的那个两键编码，以及这两个音节。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub second: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShuangpinCustomProblemKind {
    UnknownUnit,
    DuplicateUnit,
    MissingUnit,
    InvalidKey,
    InitialKeyTaken,
    AmbiguousCode,
}

impl ShuangpinCustomProblem {
    fn of_kind(kind: ShuangpinCustomProblemKind) -> Self {
        Self {
            kind,
            part: None,
            unit: None,
            key: None,
            code: None,
            first: None,
            second: None,
        }
    }
}

impl From<ShuangpinCustomProfileError> for ShuangpinCustomProblem {
    fn from(error: ShuangpinCustomProfileError) -> Self {
        use ShuangpinCustomProblemKind as Kind;
        match error {
            ShuangpinCustomProfileError::UnknownUnit { part, unit } => Self {
                part: Some(part.name()),
                unit: Some(unit),
                ..Self::of_kind(Kind::UnknownUnit)
            },
            ShuangpinCustomProfileError::DuplicateUnit { part, unit } => Self {
                part: Some(part.name()),
                unit: Some(unit),
                ..Self::of_kind(Kind::DuplicateUnit)
            },
            ShuangpinCustomProfileError::MissingUnit { part, unit } => Self {
                part: Some(part.name()),
                unit: Some(unit.to_owned()),
                ..Self::of_kind(Kind::MissingUnit)
            },
            ShuangpinCustomProfileError::InvalidKey { part, unit, key } => Self {
                part: Some(part.name()),
                unit: Some(unit),
                key: Some(key),
                ..Self::of_kind(Kind::InvalidKey)
            },
            ShuangpinCustomProfileError::InitialKeyTaken { unit, key } => Self {
                part: Some(ShuangpinCustomTablePart::Initials.name()),
                unit: Some(unit.to_owned()),
                key: Some(key),
                ..Self::of_kind(Kind::InitialKeyTaken)
            },
            ShuangpinCustomProfileError::AmbiguousCode {
                code,
                first,
                second,
            } => Self {
                code: Some(code),
                first: Some(first),
                second: Some(second),
                ..Self::of_kind(Kind::AmbiguousCode)
            },
        }
    }
}

/// 编辑中的自定义双拼表有什么问题，能用时是 `None`。结论与 [`validate_custom_shuangpin_profile`] 相同，但用 Engine 不留存的检查：设置页每改一个键就问一次，每张合法的中间表都留下来就是永久泄漏。
pub fn custom_shuangpin_profile_problem(
    profile: &ShuangpinCustomProfile,
) -> Option<ShuangpinCustomProblem> {
    check_shuangpin_custom_profile(&engine_shuangpin_custom_table(profile))
        .err()
        .map(ShuangpinCustomProblem::from)
}

/// 内置双拼方案写成自定义表的样子，作为编辑自定义方案的起点；`Custom` 本身没有内置的表，返回 `None`。
pub fn builtin_shuangpin_custom_profile(
    profile: ShuangpinProfile,
) -> Option<ShuangpinCustomProfile> {
    let table =
        ShuangpinProfileKind::from_u8(profile_code(profile)).and_then(shuangpin_builtin_table)?;
    let map = |pairs: Vec<(String, String)>| pairs.into_iter().collect();
    Some(ShuangpinCustomProfile {
        initials: map(table.initials),
        finals: map(table.finals),
        zero_initials: map(table.zero_initials),
    })
}

/// 把双拼方案写进 Engine 选项。选的是自定义方案时先让 Engine 校验用户的表：表是空的或不合法时按小鹤运行，返回原因，免得一张写坏的表让 Engine 建不起来、整个输入法停在旧会话上。
fn apply_shuangpin_profile(
    options: &mut EngineOptions,
    preferences: &Preferences,
) -> Option<String> {
    options.shuangpin_custom_profile = None;
    options.shuangpin_profile = profile_code(preferences.shuangpin_profile);
    if preferences.shuangpin_profile != ShuangpinProfile::Custom {
        return None;
    }
    match validate_custom_shuangpin_profile(&preferences.shuangpin_custom_profile) {
        Ok(()) => {
            options.shuangpin_custom_profile = Some(engine_shuangpin_custom_table(
                &preferences.shuangpin_custom_profile,
            ));
            None
        }
        Err(error) => {
            options.shuangpin_profile = profile_code(ShuangpinProfile::Xiaohe);
            Some(format!(
                "Custom shuangpin profile unusable because {error}; using Xiaohe."
            ))
        }
    }
}


/// 交给 Engine 的 `enabled_schemes`：本版本提供的拼音方案。
fn engine_schemes(edition: &Edition) -> SchemeSet {
    offered_input_schemes(edition)
        .into_iter()
        .filter_map(|scheme| SchemeType::from_u8(scheme_code(scheme)))
        .fold(SchemeSet::EMPTY, SchemeSet::with)
}

fn scheme_code(scheme: InputScheme) -> u8 {
    match scheme {
        InputScheme::Quanpin => 0,
        InputScheme::Shuangpin => 1,
        InputScheme::Wubi => 2,
        InputScheme::Japanese => 3,
        InputScheme::Korean => 4,
        InputScheme::Cantonese => 5,
        InputScheme::Zhuyin => 6,
        InputScheme::Vietnamese => 7,
        InputScheme::Tibetan => 8,
        InputScheme::Stroke => 9,
    }
}

fn vietnamese_input_method_code(vietnamese: VietnamesePreferences) -> u8 {
    match vietnamese.input_method {
        VietnameseInputMethod::Telex => 0,
        VietnameseInputMethod::Vni => 1,
    }
}

fn korean_layout_code(korean: KoreanPreferences, supported: bool) -> u8 {
    u8::from(supported && korean.layout == KoreanLayout::Romaja)
}

fn vietnamese_tone_style_code(vietnamese: VietnamesePreferences) -> u8 {
    match vietnamese.tone_style {
        VietnameseToneStyle::Modern => 0,
        VietnameseToneStyle::Classic => 1,
    }
}

/// The Cantonese, Zhuyin and Stroke dictionaries a host has installed. Each is its own artifact rather than part of the shared resource set, so any of them may be missing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LanguageDictionaries {
    pub(crate) cantonese: Option<std::path::PathBuf>,
    pub(crate) zhuyin: Option<std::path::PathBuf>,
    pub(crate) stroke: Option<std::path::PathBuf>,
    /// 五笔码表不在：资源目录不带 `msime-wubi.db`（本平台的发布包可以不带它），也没有下载的那份，见 [`wubi_dictionary_missing`]。缺省为假，五笔照常可用。
    pub(crate) wubi_missing: bool,
}

impl LanguageDictionaries {
    fn trusted_regular_file(path: &Path) -> bool {
        msime_path_trust::reject_symlinked_components(path).is_ok() && path.is_file()
    }

    /// 每个词库只用 HostOptions 记录的 `recorded` 目录（随包内置）里的那份；不在就是缺席。
    pub(crate) fn resolve(recorded: Option<&Path>) -> Self {
        let find = |name: &str| {
            recorded
                .filter(|directory| directory.is_absolute())
                .map(|directory| directory.join(name))
                .filter(|path| Self::trusted_regular_file(path))
        };
        LanguageDictionaries {
            cantonese: find("msime-cantonese.db"),
            zhuyin: find("msime-zhuyin.db"),
            stroke: find("msime-stroke.db"),
            wubi_missing: false,
        }
    }

    /// `msime-cantonese.db`, `msime-zhuyin.db` and `msime-stroke.db` in `directory`, each when it is a file.
    fn in_directory(directory: &std::path::Path) -> Self {
        let present = |name: &str| {
            let path = directory.join(name);
            Self::trusted_regular_file(&path).then_some(path)
        };
        LanguageDictionaries {
            cantonese: present("msime-cantonese.db"),
            zhuyin: present("msime-zhuyin.db"),
            stroke: present("msime-stroke.db"),
            wubi_missing: false,
        }
    }

    /// The dictionaries an Engine was configured with. 拼音只读共享资源，没有单独的词库。
    fn of_options(_options: &EngineOptions) -> Self {
        LanguageDictionaries::default()
    }

    fn is_empty(&self) -> bool {
        self.cantonese.is_none() && self.zhuyin.is_none() && self.stroke.is_none()
    }

    /// Whether `scheme` can run with these dictionaries: Cantonese, Zhuyin and Stroke need their own, 五笔要有码表（随包或下载的 `msime-wubi.db`）, every other scheme reads only the shared resources.
    fn serve(&self, scheme: InputScheme) -> bool {
        match scheme {
            InputScheme::Cantonese => self.cantonese.is_some(),
            InputScheme::Zhuyin => self.zhuyin.is_some(),
            InputScheme::Stroke => self.stroke.is_some(),
            InputScheme::Wubi => !self.wubi_missing,
            _ => true,
        }
    }
}

/// 五笔码表 `msime-wubi.db` 由 Engine 从资源目录读取；没有下载的资源包，所以这里总是空路径。
fn wubi_dictionary() -> Option<PathBuf> {
    None
}

/// 五笔方案此时有没有码表可读：本平台的发布包可以不带 `msime-wubi.db`（[`ON_DEMAND_ARTIFACTS`]），资源目录里没有它，宿主也没有交来下载的那份（`wubi_dictionary` 为空）。
pub(crate) fn wubi_dictionary_missing(resources: &Path, wubi_dictionary: &str) -> bool {
    let name = msime_engine::assets::WUBI_DICTIONARY;
    ON_DEMAND_ARTIFACTS
        .iter()
        .any(|group| group.contains(&name))
        && wubi_dictionary.is_empty()
        && !resources.join(name).is_file()
}

/// EngineOptions 里的路径文本，没有路径（或路径不是 UTF-8）时为空。
fn path_text(path: Option<PathBuf>) -> String {
    path.and_then(|path| path.to_str().map(str::to_owned))
        .unwrap_or_default()
}

/// 内置辅助码表的只读回退根目录（`EngineOptions::helpcode_fallback_resources`）。
///
/// 辅助码表随安装包放在资源目录的 `helpcodes/` 下，不在词库发布里，所以装上在线词库更新、`resources` 换成下载的代次目录后，那里没有这些表，Engine 只得到空表，辅助码什么也筛不掉。文档记了回退目录时用它（macOS 输入法、`prepare_host_with_updates` 准备出的文档）；没记时取更新记录里的随包资源目录，覆盖只经 `refresh_host_options` 换过 `resources` 的旧文档。`resources` 自己带着 `helpcodes/` 时不需要回退，也就不读更新记录。
fn helpcode_fallback_resources(
    recorded: Option<String>,
    resources: &Path,
    state_root: Option<&Path>,
    edition: &'static Edition,
) -> Option<String> {
    let _ = (state_root, edition);
    recorded.filter(|_| resources.join("helpcodes").is_dir())
}

/// HostOptions 的 `preferences_directory`，只认绝对路径：按需下载的资源包装在它下面。
fn absolute_state_root(preferences_directory: Option<&str>) -> Option<PathBuf> {
    preferences_directory
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// 按 `preferences` 交给 Engine 的方案，以及没用偏好里那个方案时的原因。本构建或本版本不提供的方案、没装词库的粤拼、注音和笔画，都回退到上一次的中文方案（它能跑时），否则回退到 `default`，所以别的宿主写下的文档不会让这个宿主没有能用的方案。偏好本身不改：词库装好后，下一个会话就跑用户选的方案。
///
/// `supported` 是运行中版本提供的方案（`offered_input_schemes`），`default` 是该版本的默认方案；full 分别是全部十个方案和全拼。所以在五笔版里，即使同步下来的偏好写着全拼，Engine 跑的也是五笔。
pub(crate) fn effective_scheme(
    preferences: &Preferences,
    supported: &[InputScheme],
    dictionaries: &LanguageDictionaries,
    default: InputScheme,
) -> (InputScheme, Option<String>) {
    let usable = |scheme: InputScheme| supported.contains(&scheme) && dictionaries.serve(scheme);
    let preferred = preferences.scheme;
    if usable(preferred) {
        return (preferred, None);
    }
    let reason = if supported.contains(&preferred) {
        "its dictionary is not installed"
    } else {
        "this host does not offer it"
    };
    let fallback = preferences
        .last_chinese_scheme
        .map(InputScheme::from)
        .filter(|scheme| usable(*scheme))
        .unwrap_or(default);
    (
        fallback,
        Some(format!(
            "Input scheme {preferred:?} unavailable because {reason}; using {fallback:?}."
        )),
    )
}

/// 偏好里的双拼辅助码触发方式 → 引擎侧同名字段；写入端已归一，这里对消费端再归一一次兜底。
fn shuangpin_helpcode_modes(
    modes: msime_client_core::preferences::ShuangpinHelpcodeModes,
) -> msime_engine::ShuangpinHelpcodeModes {
    let modes = modes.normalized();
    msime_engine::ShuangpinHelpcodeModes {
        // 目前只有 macOS 宿主把 Tab 送入字符接口；其它宿主保留原触发方式。
        manual_tab: cfg!(target_os = "macos") && modes.manual_tab,
        direct: modes.direct,
        trigger_backtick: modes.trigger_backtick,
        trigger_semicolon: modes.trigger_semicolon,
        trigger_uppercase: modes.trigger_uppercase,
        mid_sentence: modes.mid_sentence,
        mid_sentence_backtick: modes.mid_sentence_backtick,
        mid_sentence_semicolon: modes.mid_sentence_semicolon,
        mid_sentence_uppercase: modes.mid_sentence_uppercase,
        direct_helpcode: modes.direct_helpcode,
        direct_helpcode_slash: modes.direct_helpcode_slash,
        direct_helpcode_uppercase: modes.direct_helpcode_uppercase,
    }
}

/// The helpcode settings for the scheme actually run, which differs from `Preferences::active_helpcode` only when `effective_scheme` fell back.
fn helpcode_for_scheme(
    preferences: &Preferences,
    scheme: InputScheme,
) -> msime_client_core::preferences::HelpcodePreferences {
    if scheme == preferences.scheme {
        preferences.active_helpcode()
    } else {
        Preferences {
            scheme,
            ..preferences.clone()
        }
        .active_helpcode()
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HostOptions {
    api_version: u32,
    resources: String,
    /// 当前宿主随包提供的内置辅助码回退资源根目录。macOS 输入法只在内存文档中注入；`prepare_host_with_updates` 在 `resources` 换成下载代次时写它；都没有时 `helpcode_fallback_resources()` 从更新记录里找。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    helpcode_fallback_resources: Option<String>,
    user_data: String,
    cache: String,
    dictionaries: String,
    preferences: Preferences,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preferences_directory: Option<String>,
    /// The host draws a half-composed phrase itself: picking a candidate that covers only part of
    /// the input leaves the chosen piece in `view.phrase_prefix` instead of committing it, and the
    /// whole phrase commits at once when the composition ends. Absent means the previous behaviour,
    /// where each piece went to the document as it was picked, because a host that does not draw
    /// the field would otherwise show nothing for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phrase_preedit: Option<bool>,
    /// `false`：这个会话不记选词位置和上屏效率。给与另一个会话处理同一次选词的镜像会话用：Windows TIP 在应用进程里的宿主会话跟着 Server 选同一个词，两边都记就会算两遍，由 Server 的会话记。缺省照常记。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection_statistics: Option<bool>,
    /// `false`：这个会话不挂逐键重排（整句联想「增强」的模型），开关打开也不挂。给只跟着另一个会话选词的镜像会话用：Windows TIP 在每个打字的应用进程里各有一个宿主会话，候选窗是 Server 画的、顺序是 Server 的会话排的，TIP 按文字选中同一个词（`tsf/HostServerSelection.h`），自己再排一遍用不上，却要在每个进程里各加载一份模型（ONNX 大模型 31 MB）。缺省照常挂。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keyboard_reranker: Option<bool>,
    /// 放两档 ONNX 逐键重排模型（`rerank-model/`、`rerank-model-small/`）的目录。缺省时在随包资源目录旁边找（[`ffi::rerank_models_root`]）。Windows 的 Server 在内存里写它：它的随包资源目录在 Program Files 的 `server\resources`，模型装在旁边，但 `helpcode_fallback_resources` 是 DataDir（辅助码表在那里），装了在线词库更新后 `resources` 又换成 DataDir 下的代次，两条都找不到安装包里的模型。鸿蒙的键盘同样在内存里写它：模型随 HAP 解压在扩展的 `resourceDir`，而资源目录是复制到 `filesDir/engine` 的那份（SQLite 打不开只读的安装目录），上一级找不到模型。不写进准备好的 runtime-options.json。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rerank_models: Option<String>,
    /// `true`：宿主在用户停顿时调 `msime_client_start_settled_rerank`（鸿蒙的键盘在停顿 150 ms 后调）。这样的宿主上，跑不进逐键预算的设备不再干脆不挂整句联想「增强」的模型，而是延后到停顿时用它把当前组字重解一遍，见 [`ffi::RerankerMount::KeystrokeOrPause`]。缺省是不调：模型跑不进预算就不挂，与以前相同。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pause_rerank: Option<bool>,
    /// macOS 和 Linux 的两个宿主把普通组字里的快捷短语也作为候选显示；缺省关闭以保持其他宿主和旧文档的入口契约。建会话时写进 `EngineOptions`，偏好变更重建 Engine 时沿用同一份选项。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quick_phrase_candidates: Option<bool>,
    /// 仅完成键盘接入的桌面宿主显式打开；缺字段时同步偏好仍按两套式运行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    korean_romaja: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clipboard_history_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    online_provider_socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    translation_provider_socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cloud_dictionary_provider_socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cloud_clipboard_provider_socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    voice_provider_socket: Option<String>,
    /// Absolute path to the candidate reranking model, when it is installed as its own artifact
    /// rather than placed beside the dictionaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sentence_model: Option<String>,
    /// Absolute path to the larger model run once typing settles, for hosts that install one.
    ///
    /// Its own option rather than an entry in the dictionary lock, because the lock is shared by
    /// every platform and `ResourceStore::verify` requires a resource directory to match it
    /// exactly — a desktop-only artifact there would mean teaching the manifest, the Rust
    /// verifier, the PowerShell verifier, four staging scripts and a CMake parser what a platform
    /// is, all on the path that guarantees a shipped dictionary is intact. Twenty five megabytes
    /// inside an iOS keyboard extension is also precisely what the small preset exists to avoid.
    ///
    /// A host that wants the second model installs it where it likes and names it here; one that
    /// does not leaves this absent and behaves exactly as before. Today that is every host: the
    /// desktop platforms are the ones this is for, and they set it alongside shipping the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settled_model: Option<String>,
    /// Absolute path to the bundle's built-in sound packs (`resources/sound-packs` in the repository), for a host whose bundle does not put them in `sound-packs` beside `resources`, the directory used when this is absent. Installed packs, command tables and the `@` name list are read from `plugins` under `preferences_directory`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sound_packs: Option<String>,
    /// 桌面安装包随带的官方插件包目录（绝对路径，`<kind>/<id>/`，构建产物是 `target/plugin-packs`）。没有时取内置音效包目录旁边的 `plugin-packs`，那里也没有就没有官方包。macOS 的 bundle 和 Linux 的安装把它放在音效包旁边，不用写；Windows 的 Server 写它，因为那里的音效包在普通用户可写的数据目录里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plugin_packs: Option<String>,
    /// Absolute path to the directory holding `msime-cantonese.db`, `msime-zhuyin.db` and `msime-stroke.db`, for a host that installs any of them. Absent, or a directory missing one of them, means that scheme falls back as `effective_scheme` describes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language_dictionaries: Option<String>,
    /// 产品版本（`Edition::HOST_OPTIONS_KEY`）。只有不是 full 的版本才写：full 的文档因此与引入版本之前逐字节相同，旧版输入法（本结构拒绝未知键）照样能读。缺省就是 full；不是版本表里的 id 时整份文档被拒，而不是猜成 full。
    #[serde(default, skip_serializing_if = "Option::is_none", with = "edition_id")]
    edition: Option<&'static Edition>,
    /// `true`：宿主对共享状态目录（`preferences_directory`）没有写权限，只读地用别的进程准备好的代次，或者只读布局下的随包词库（iOS 键盘没有完全访问，#10945；见 [`read_only_state`]）。这样的会话不学习、不记选词统计和已载入的代次，词库租约只读打开已有的锁文件。只由 `msime_client_prepare_host` 在内存里交给宿主，不写进任何发布的 runtime-options.json：旧版输入法（本结构拒绝未知键）读不了带它的文档。缺省是能写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    read_only_state: Option<bool>,
}

/// 宿主选项里的偏好副本用不了时，输入运行时改用的偏好：`preferences_directory` 下有实时的 preferences.json 就读它（读不了时在内存里救下认得的设置），没有就救副本自己的，默认值取文档记录的版本。文档里没有偏好副本、也没有实时文档时返回 `None`。读不了的偏好经诊断出口报告，盘上的文件不动。
fn runtime_preferences_for(document: &Value) -> Option<Preferences> {
    let directory = document
        .get("preferences_directory")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        // load_for_runtime() 会创建传入的目录；从没在那里写过偏好的宿主没有可替代的内容。
        .filter(|directory| {
            directory.is_absolute() && directory.join("preferences.json").is_file()
        });
    if let Some(directory) = directory {
        if let Ok(runtime) = PreferencesStore::new(&directory).load_for_runtime() {
            report_unreadable_preferences(&runtime);
            return Some(runtime.snapshot.preferences);
        }
    }
    let copy = document.get("preferences")?.clone();
    // 副本本身没问题、是文档别处不对时照原样交回，不当成偏好读不了去报告。
    if let Ok(preferences) = Preferences::deserialize(&copy) {
        if preferences.validate().is_ok() {
            return Some(preferences);
        }
    }
    let edition = Edition::of_host_options(document)?;
    let (preferences, not_kept) =
        Preferences::salvaged_from(copy, edition_default_preferences(edition)).ok()?;
    crate::diagnostics::report(
        "runtime options preferences unusable",
        &format!(
            "the preferences copy in the runtime options does not validate and there is no live preferences.json; running on the settings that could be kept, settings not kept: {}",
            not_kept.join(", ")
        ),
    );
    Some(preferences)
}

/// `edition` 的默认偏好，与 [`PreferencesStore`] 修复损坏文件时垫底的相同。
fn edition_default_preferences(edition: &'static Edition) -> Preferences {
    if edition.is_full() {
        Preferences::default()
    } else {
        Preferences::for_edition(edition)
    }
}

/// 读得了时什么也不做；读不了时报告输入运行时改用了在内存里救下的设置。设置界面经严格读取看到同一个错误，修不修复由用户决定。
///
/// 只报告错误的种类和没能保留的设置键，不带 serde 的错误原文：类型不对时它会引出文档里的字符串取值，那可能是用户填的地址或密钥，而诊断日志会被打进诊断包发出去。
fn report_unreadable_preferences(runtime: &msime_client_core::preferences::RuntimePreferences) {
    use msime_client_core::preferences::PreferencesError;
    let Some(error) = &runtime.load_error else {
        return;
    };
    let kind = match error {
        PreferencesError::Json(_) => {
            "the document does not parse as this version's preferences".to_owned()
        }
        other => other.to_string(),
    };
    crate::diagnostics::report(
        "preferences document unreadable",
        &format!("{kind}; settings not kept: {}; typing continues on the settings that could be kept, the file is left untouched until it is repaired in settings or rewritten by a newer version", runtime.not_kept.join(", ")),
    );
}

/// HostOptions 的 `edition` 键与版本表条目之间的转换。
mod edition_id {
    use msime_client_core::edition::Edition;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        edition: &Option<&'static Edition>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match edition {
            Some(edition) => serializer.serialize_str(&edition.id),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<&'static Edition>, D::Error> {
        match Option::<String>::deserialize(deserializer)? {
            None => Ok(None),
            Some(id) => Edition::by_id(&id)
                .map(Some)
                .ok_or_else(|| serde::de::Error::custom(format!("unknown edition {id:?}"))),
        }
    }
}

impl HostOptions {
    /// 文档记录的版本，缺省是 full。
    fn edition(&self) -> &'static Edition {
        self.edition.unwrap_or_else(Edition::full)
    }

    /// 解析 HostOptions 文档。其中的 `preferences` 是准备运行时配置那一刻的副本，升级时被原样带下去（`refresh_options_file`）。#2830 退役 `autocorrect` 时，带着它的副本曾让会话、快照和词库请求全部被拒，macOS 每个按键都像在英文模式下一样直接交给应用；偏好结构现在不拒绝不认识的键，新增或退役一个键不会再让副本读不了，但取值的类型或格式变化、以及新版本校验更严（#9802 的文本编辑排列）仍会。副本解析不了或通不过校验时，改用 `preferences_directory` 下实时的 preferences.json——设置界面会保持它最新并负责修复，宿主在会话打开后本来也会应用它；实时文档也读不了时在内存里救下它认得的设置（[`PreferencesStore::load_for_runtime`]），没有实时文档时救副本自己的，都不改写任何文件。副本能用时照原样使用。
    pub(crate) fn from_document(mut document: Value) -> Option<Self> {
        if let Ok(options) = Self::deserialize(&document) {
            if options.preferences.validate().is_ok() {
                return Some(options);
            }
        }
        document["preferences"] = serde_json::to_value(runtime_preferences_for(&document)?).ok()?;
        Self::deserialize(&document).ok()
    }

    fn into_engine_options(self) -> EngineOptions {
        // 每个平台都这样查找；桌面宿主按发布包是否省略日文词典和语言词库决定是否从状态目录读取已下载资源。
        let state_root = absolute_state_root(self.preferences_directory.as_deref());
        let dictionaries =
            LanguageDictionaries::resolve(self.language_dictionaries.as_deref().map(Path::new));
        // Session creation has no diagnostic to carry the reason; the fallback itself is what matters here.
        let edition = self.edition();
        let (scheme, _) = effective_scheme(
            &self.preferences,
            &offered_input_schemes(edition),
            &dictionaries,
            edition.default_scheme,
        );
        let helpcode = helpcode_for_scheme(&self.preferences, scheme);
        let helpcode_fallback_resources = helpcode_fallback_resources(
            self.helpcode_fallback_resources,
            Path::new(&self.resources),
            state_root.as_deref(),
            edition,
        );
        let mut options = EngineOptions {
            resources: self.resources,
            user_data: self.user_data,
            cache: self.cache,
            dictionaries: self.dictionaries,
            // 词库维护类操作也走这里：`HostOptions` 没有 `page_size_override`，只能用偏好值。
            first_page_rows: usize::from(host_page_size(self.preferences.candidate_page_size)),
            scheme: scheme_code(scheme),
            enabled_schemes: engine_schemes(edition),
            shuangpin_profile: profile_code(self.preferences.shuangpin_profile),
            shuangpin_custom_profile: None,
            shuangpin_preedit_uses_raw: self.preferences.shuangpin_preedit_uses_raw,
            shuangpin_preedit_separated: self.preferences.shuangpin_preedit_separated,
            single_character_only: self.preferences.single_character_only,
            caret_edit_suffix: caret_edit_suffix_applies(&self.preferences),
            learning: self.preferences.learning,
            learn_english_on_enter: self.preferences.enter_learns_english_word,
            next_word_prediction: next_word_prediction_applies(&self.preferences),
            autocorrect_transposition: self.preferences.quanpin.autocorrect_transposition,
            autocorrect_neighbor: self.preferences.quanpin.autocorrect_neighbor,
            fuzzy_pinyin_rules: self.preferences.fuzzy_pinyin.active_rules(),
            frequency_mode: self.preferences.frequency.mode.as_str().into(),
            frequency_trigger_count: self.preferences.frequency.trigger_count,
            frequency_linear_step: self.preferences.frequency.linear_step,
            mixed_candidates: self.preferences.mixed_input.enabled,
            mixed_english: self.preferences.mixed_input.english,
            english_minimum_prefix: host_english_minimum_prefix(
                self.preferences.mixed_input.minimum_prefix,
            ),
            english_first_page_count: self.preferences.mixed_input.first_page_count,
            mixed_emoji: self.preferences.mixed_input.emoji,
            mixed_kaomoji: self.preferences.mixed_input.kaomoji,
            mixed_symbols: self.preferences.mixed_input.symbols,
            local_unicode: self.preferences.local_modes.unicode,
            local_date_time: self.preferences.local_modes.date_time,
            local_date_time_inline: self.preferences.local_modes.date_time_inline,
            local_date_time_menu: self.preferences.local_modes.date_time_menu,
            local_quick_phrase: self.preferences.local_modes.quick_phrase,
            local_emoji: self.preferences.local_modes.emoji,
            local_kaomoji: self.preferences.local_modes.kaomoji,
            local_super_jianpin: self.preferences.local_modes.super_jianpin,
            local_super_jianpin_slash: self.preferences.local_modes.super_jianpin_slash,
            local_temporary_english: self.preferences.local_modes.temporary_english,
            local_temporary_japanese: self.preferences.local_modes.temporary_japanese,
            local_expression: self.preferences.local_modes.expression,
            local_command: self.preferences.local_modes.command,
            local_mention: self.preferences.local_modes.mention,
            local_chaizi: chaizi_enabled(&self.preferences.local_modes),
            local_symbol: self.preferences.local_modes.symbol,
            local_shift_letter_entry: self.preferences.local_modes.shift_letter_entry,
            chaizi_traditional_output: self.preferences.traditional_chinese_output,
            command_table: Vec::new(),
            plugin_commands: Vec::new(),
            mention_entries: Vec::new(),
            quick_phrase_table: Vec::new(),
            // 宿主声明能力、用户在设置里打开，两个条件都满足才把快捷短语混进普通候选；`mixed_input.quick_phrase` 缺键的旧文档读作开，保住 macOS/Linux 升级前的行为。
            quick_phrase_candidates: self.quick_phrase_candidates.unwrap_or(false)
                && self.preferences.mixed_input.quick_phrase,
            quick_phrase_frequency: self.preferences.mixed_input.quick_phrase_frequency,
            helpcode_table: None,
            helpcode_fallback_resources,
            // 用户自己的辅助码表放在状态根下（`msime_client_core::helpcode::user_helpcode_root`），`resources` 是安装包或词库代次，升级会换掉它。
            custom_helpcode_root: state_root
                .as_deref()
                .and_then(Path::to_str)
                .map(str::to_owned),
            sentence_association: engine_sentence_association(
                &self.preferences.sentence_association,
                absolute_state_root(self.preferences_directory.as_deref()).as_deref(),
            ),
            rescoring_context: String::new(),
            sentence_alternatives: true,
            helpcode: helpcode.enabled,
            strict_quanpin_helpcode: helpcode.strict_filter,
            show_helpcode: helpcode.show_in_candidate_window,
            // 双拼专属的触发方式；其它方案给默认值，引擎只在双拼下读它。手改出互斥组合的文档在消费端再归一一次，与 `PreferencesStore::save` 的写入端归一一致。
            shuangpin_helpcode_modes: shuangpin_helpcode_modes(helpcode.modes),
            helpcode_schema: helpcode.schema.as_str().into(),
            helpcode_trigger_key: helpcode.trigger_key.bytes().next(),
            chinese_punctuation: engine_chinese_punctuation(
                self.preferences.chinese_punctuation,
                punctuation_lock_code(self.preferences.punctuation_lock),
            ),
            paired_punctuation: self.preferences.paired_punctuation,
            punctuation_lock: punctuation_lock_code(self.preferences.punctuation_lock),
            corner_quotes: false,
            hollow_brackets: false,
            enumeration_comma_backslash: true,
            enumeration_comma_slash: false,
        };
        apply_punctuation_marks(&mut options, &self.preferences);
        // 建会话没有地方带原因，表不能用时按小鹤建起来就够了；之后的偏好变更会在诊断里报告原因。
        let _ = apply_shuangpin_profile(&mut options, &self.preferences);
        apply_local_mode_resource_gates(&mut options, edition);
        options
    }
}

/// Bootstrap a new host using the reviewed desktop data and Engine-owned replay.
/// Call only while all sessions using state_root are stopped. Does not activate it.
/// The settled-rerank model installed beside a resource bundle, when one is there.
///
/// A sibling directory rather than a file inside `resources`, because that directory is verified
/// against `desktop-dictionary.lock.json` and must match it *exactly* — an extra file there fails
/// the check whose job is to prove a shipped dictionary is intact. The lock is also shared by all
/// six platforms, and this model is wanted by three: it buys 49 points of top-1 on the harvested
/// failure set and costs p95 153ms per keystroke, which is why it runs on the settle timer, and
/// why 25MB of it has no business inside an iOS keyboard extension.
///
/// Absence is the normal case and is not an error. A host that installs the model puts it here;
/// one that does not is left exactly as it was.
fn settled_model_beside(resources: &std::path::Path) -> Option<String> {
    let path = resources
        .parent()?
        .join("settled-model")
        .join("sentence-model-desktop.safetensors");
    path.is_file().then(|| path.to_str())??.to_owned().into()
}

/// 后台加载落定重排模型的结果槽：加载完之前是空的，加载完是模型，模型不能用（文件坏了或版本不符）或没有模型时是 `None`。
type SettledModelSlot = Arc<OnceLock<Option<Arc<SentenceModel>>>>;

/// 后台挂好的逐键重排器：外层 `None` 是还在挂，`Some(None)` 是挂完了但这台设备上没有可挂的（没装模型、超出按键预算、线程起不来）。挂上的那个带着挂在哪里（按键上，或延后到停顿）。打分器只是 `Send`，所以用锁而不是 `OnceLock`。
type KeyboardRerankerSlot = Arc<Mutex<Option<Option<ffi::MountedReranker>>>>;

/// 在后台线程里挂逐键重排器（[`ffi::keyboard_reranker_for`]，开关开着；`pause` 是宿主会在停顿时重解，见 [`ffi::RerankerMount::KeystrokeOrPause`]），立即返回结果槽。挂上之后还在同一个进程里再挂时共用已加载的那一份，只是第一次慢，见 `keyboard_reranker_loading`。线程起不来时槽里直接是 `Some(None)`，会话照常输入，只是不排序。
fn load_keyboard_reranker(
    resources: &str,
    bundle: Option<&str>,
    models: Option<&str>,
    sound_packs: Option<&Path>,
    state_root: Option<&Path>,
    configured: Option<&str>,
    pause: bool,
) -> KeyboardRerankerSlot {
    let slot: KeyboardRerankerSlot = Arc::new(Mutex::new(None));
    let resources = resources.to_owned();
    let bundle = bundle.map(str::to_owned);
    let models = models.map(str::to_owned);
    let sound_packs = sound_packs.map(Path::to_path_buf);
    let state_root = state_root.map(Path::to_path_buf);
    let configured = configured.map(str::to_owned);
    let loading = Arc::clone(&slot);
    let spawned = std::thread::Builder::new()
        .name("msime-keyboard-reranker".into())
        .spawn(move || {
            let mount = if pause {
                ffi::RerankerMount::KeystrokeOrPause
            } else {
                ffi::RerankerMount::Keystroke
            };
            let scorer = ffi::keyboard_reranker_for(
                mount,
                &resources,
                bundle.as_deref(),
                models.as_deref(),
                sound_packs.as_deref(),
                state_root.as_deref(),
                configured.as_deref(),
            );
            *loading
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(scorer);
        });
    if spawned.is_err() {
        *slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(None);
    }
    slot
}

/// 在后台线程里加载落定重排模型，立即返回结果槽。约 25 MB 的权重不能在输入线程上读：聚焦和按键都在那个线程上。`path` 为 `None`（模型被移除）时槽里直接是 `None`。线程起不来时同样放 `None`，会话照常输入，只是没有落定重排。
fn load_settled_model(resources: &str, path: Option<PathBuf>) -> SettledModelSlot {
    let slot: SettledModelSlot = Arc::new(OnceLock::new());
    let Some(path) = path else {
        let _ = slot.set(None);
        return slot;
    };
    let resources = resources.to_owned();
    let loading = Arc::clone(&slot);
    let spawned = std::thread::Builder::new()
        .name("msime-settled-model".into())
        .spawn(move || {
            let _ = loading.set(sentence_model_settled(&resources, path.to_str()));
        });
    if spawned.is_err() {
        let _ = slot.set(None);
    }
    slot
}

/// 随包的落定重排模型：HostOptions 记录的 `settled_model`，没记录时是资源目录里的那份（[`sentence_model_settled`] 的默认位置）。只认存在的文件。
fn bundled_settled_model(resources: &str, recorded: Option<&str>) -> Option<PathBuf> {
    let path = match recorded {
        Some(path) => PathBuf::from(path),
        None => Path::new(resources).join(SETTLED_MODEL_FILE),
    };
    if !path.is_absolute() {
        return None;
    }
    path.is_file().then_some(path)
}

/// 会话使用的落定重排模型文件：只有随包的那份，资源包下载已移除。
pub(crate) fn settled_model_file(
    resources: &str,
    recorded: Option<&str>,
    _state_root: Option<&Path>,
) -> Option<PathBuf> {
    bundled_settled_model(resources, recorded)
}

/// 把会话的落定重排模型接上了没有写进诊断日志：会话建立时、开关打开时和模型文件变了（资源包装好或移除）换上之后各一行，按键和评分路径上不写。只在「桌面神经联想」开着时由调用方报告，关着的用户不会看到这些行。
///
/// 没有这几行时，「没装模型」「装了但会话没拿到状态目录」「装了但读不了」在日志里一模一样，都是什么也没有（#10341）。macOS 的 `diagnostic.log` 和 Windows 的 `logs\server.log` 都只记冒号前的类别（写成 `host_api: <类别>`），路径只出现在没登记出口的 Linux stderr 里：
/// - `settled model attached`：模型已加载并挂在会话上；
/// - `settled model not loaded`：找到了模型文件却没能用上，原因在同一次加载报的 `sentence model unreadable` 或 `sentence model ignored` 里（同一个文件在本进程只加载一次，后来的会话只有这一行）；
/// - `settled model missing`：随包的和状态目录下的 `settled-model` 资源包都没有；
/// - `settled model missing without state directory`：没有随包的模型，而宿主没给绝对路径的 `preferences_directory`，所以根本没去找下载的资源包。
pub(crate) fn report_settled_model(
    resources: &str,
    state_root: Option<&Path>,
    model: Option<&Path>,
    attached: bool,
) {
    match (model, state_root) {
        (Some(model), _) if attached => {
            diagnostics::report("settled model attached", &model.display().to_string())
        }
        (Some(model), _) => {
            diagnostics::report("settled model not loaded", &model.display().to_string())
        }
        (None, Some(state_root)) => diagnostics::report(
            "settled model missing",
            &format!(
                "no settled-model pack under {}",
                state_root.display()
            ),
        ),
        (None, None) => diagnostics::report(
            "settled model missing without state directory",
            &format!("no absolute preferences_directory; resources {resources}"),
        ),
    }
}

/// HostOptions 文档（`host_options`）所描述的安装布局里随包的落定重排模型。设置应用据此决定要不要提供 `settled-model` 资源包的下载：有随包的模型时会话不会用下载的那份。
pub fn packaged_settled_model(host_options: &Value) -> Option<PathBuf> {
    bundled_settled_model(
        host_options.get("resources")?.as_str()?,
        host_options.get("settled_model").and_then(Value::as_str),
    )
}

/// HostOptions 文档（`host_options`）所描述的安装布局是否随包带齐了粤拼、注音和笔画词库。
pub fn packaged_language_dictionaries(host_options: &Value) -> bool {
    let recorded = host_options
        .get("language_dictionaries")
        .and_then(Value::as_str)
        .map(Path::new);
    let dictionaries = LanguageDictionaries::resolve(recorded);
    dictionaries.cantonese.is_some()
        && dictionaries.zhuyin.is_some()
        && dictionaries.stroke.is_some()
}

/// HostOptions 文档（`host_options`）的资源目录本身是否带着五笔码表 `msime-wubi.db`。
pub fn packaged_wubi_dictionary(host_options: &Value) -> bool {
    let Some(resources) = host_options
        .get("resources")
        .and_then(Value::as_str)
        .map(Path::new)
        .filter(|resources| resources.is_absolute())
    else {
        return false;
    };
    LanguageDictionaries::trusted_regular_file(
        &resources.join(msime_engine::assets::WUBI_DICTIONARY),
    )
}

/// The Cantonese, Zhuyin and Stroke dictionaries installed beside a resource bundle: `language-dictionaries/msime-cantonese.db`, `language-dictionaries/msime-zhuyin.db` and `language-dictionaries/msime-stroke.db`, built by `msime-dict-builder`. A sibling of `resources` for the same reason as `settled_model_beside`: the resource directory must match the shared dictionary lock exactly, and only the hosts that offer these schemes ship them. Absence is the normal case.
pub(crate) fn language_dictionaries_beside(resources: &std::path::Path) -> LanguageDictionaries {
    language_dictionaries_directory(resources)
        .map(|directory| LanguageDictionaries::in_directory(&directory))
        .unwrap_or_default()
}

fn language_dictionaries_directory(resources: &std::path::Path) -> Option<std::path::PathBuf> {
    Some(resources.parent()?.join("language-dictionaries"))
}

/// The `language_dictionaries` value HostOptions records for `resources`: the directory beside them, only when it holds a dictionary, so a host without them writes the document it always did.
pub fn installed_language_dictionaries(resources: &std::path::Path) -> Option<String> {
    if language_dictionaries_beside(resources).is_empty() {
        return None;
    }
    language_dictionaries_directory(resources)
        .and_then(|directory| directory.to_str().map(str::to_owned))
}

/// Drop the `\\?\` prefix Windows canonicalisation adds.
///
/// The resource path is recorded in every HostOptions document this writes, and those documents have always held the plain drive form. Keeping it means an existing document and a freshly prepared one name the same directory the same way, and a native host reading the document gets the spelling it always got.
///
/// Only the drive form is unwrapped. `\\?\UNC\server\share` means something different from
/// `\\server\share` to the filesystem, so it is left alone rather than rewritten into a path that
/// happens to parse.
fn without_verbatim_prefix(path: std::path::PathBuf) -> std::path::PathBuf {
    let text = match path.to_str() {
        Some(text) => text,
        None => return path,
    };
    let stripped = match text.strip_prefix(r"\\?\") {
        Some(stripped) => stripped,
        None => return path,
    };
    let drive = stripped.as_bytes();
    if drive.len() >= 3 && drive[0].is_ascii_alphabetic() && drive[1] == b':' && drive[2] == b'\\' {
        return std::path::PathBuf::from(stripped);
    }
    path
}

/// Hash the resource set unless the last successful verification still describes what is on disk.
///
/// This runs at every Server start, and the desktop set is 169 MB: about half a second of SHA-256
/// before the first keystroke can be served, repeated at every login. `VerifiedMarker` records what
/// was verified so the repeat is skipped while the files are untouched.
///
/// The marker lives under the state root rather than beside the resources: on Windows the
/// resources are installed under Program Files, which the Server does not get to write to.
///
/// Failing to write the marker is not failing to start. The next launch hashes again, which is the
/// behaviour this function replaces, so the cost of that miss is the cost of doing nothing here.
fn reject_symlinked_state_root(path: &Path) -> Result<(), std::io::Error> {
    msime_path_trust::reject_symlinked_components(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::InvalidInput {
            std::io::Error::new(error.kind(), "state root contains a symbolic link")
        } else {
            error
        }
    })
}

/// 当前平台发布包可以不内置、改为按需下载的资源文件组：桌面、Android、鸿蒙和 iOS 是日文词典与它的两份 Mozc 许可文本、以及五笔码表。规则本身见 [`msime_client_core::resources::on_demand_artifacts`]，鸿蒙通过 `target_env = "ohos"` 选择规则。
#[cfg(target_env = "ohos")]
pub(crate) const ON_DEMAND_ARTIFACTS: &[&[&str]] =
    msime_client_core::resources::on_demand_artifacts("ohos");

#[cfg(not(target_env = "ohos"))]
pub(crate) const ON_DEMAND_ARTIFACTS: &[&[&str]] =
    msime_client_core::resources::on_demand_artifacts(std::env::consts::OS);

/// `resources` 实际按哪一份清单发货：`on_demand` 里每一组文件全部缺席时去掉这一组，否则这一组照锁文件要求。见 [`ResourceSet::as_shipped_in_groups`]。
pub(crate) fn shipped_specification(
    specification: &ResourceSet,
    resources: &Path,
    on_demand: &[&[&str]],
) -> ResourceSet {
    specification.as_shipped_in_groups(resources, on_demand)
}

fn verify_resources_once(
    resources: &std::path::Path,
    specification: &ResourceSet,
    state_root: &std::path::Path,
    on_demand: &[&[&str]],
) -> Result<(), Box<dyn std::error::Error>> {
    // 从内置日文词典的旧版本升级后，第一次启动时校验标记描述的清单变了，会重新哈希一次，这是有意的。
    let shipped = shipped_specification(specification, resources, on_demand);
    reject_symlinked_state_root(state_root)?;
    match std::fs::symlink_metadata(state_root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "state root is a symbolic link",
            )
            .into());
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "state root is not a directory",
            )
            .into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let marker_path = state_root.join("verified-resources.json");
    let current = VerifiedMarker::describe(resources, &shipped)?;
    if let (Some(current), Some(recorded)) = (&current, VerifiedMarker::read(&marker_path)) {
        if *current == recorded {
            return Ok(());
        }
    }
    ResourceStore::new(resources).verify(resources, &shipped)?;
    if let Some(current) = current {
        let _ = current.write(&marker_path);
    }
    Ok(())
}

/// 只读检查已准备配置所指向的资源；沿用宿主的版本锁、按需资源分组和校验标记，不创建输入会话。
pub fn runtime_resources_ready(document: &Value) -> bool {
    let check = || -> Option<bool> {
        let resources = Path::new(document.get("resources")?.as_str()?);
        let state = Path::new(document.get("preferences_directory")?.as_str()?);
        if !resources.is_absolute() || !state.is_absolute() {
            return Some(false);
        }
        let baseline = Edition::of_host_options(document)?.resource_set().ok()?;
        Some(runtime_resources_ready_with(
            resources,
            state,
            &baseline,
            ON_DEMAND_ARTIFACTS,
        ))
    };
    check().unwrap_or(false)
}

fn runtime_resources_ready_with(
    resources: &Path,
    state: &Path,
    specification: &ResourceSet,
    on_demand: &[&[&str]],
) -> bool {
    if reject_symlinked_state_root(state).is_err() {
        return false;
    }
    let shipped = shipped_specification(specification, resources, on_demand);
    if let (Ok(Some(current)), Some(recorded)) = (
        VerifiedMarker::describe(resources, &shipped),
        VerifiedMarker::read(&state.join("verified-resources.json")),
    ) {
        if current == recorded {
            return true;
        }
    }
    ResourceStore::new(resources)
        .verify(resources, &shipped)
        .is_ok()
}

pub fn prepare_host_configuration(
    resources: &std::path::Path,
    state_root: &std::path::Path,
) -> Result<String, Box<dyn std::error::Error>> {
    prepare_host_configuration_for_edition(resources, state_root, Edition::full())
}

/// 为 `edition` 准备宿主：[`prepare_host_configuration`] 就是 full 的这一个。
///
/// 不是 full 的版本在文档里记下 `edition`，之后的会话、[`refresh_host_options`] 和 `msime-mcp` 都从文档里读它；full 的文档不写这个键，与以前完全相同。状态目录也记下它属于哪个版本（[`Edition::record_in`]，full 不写）：各平台宿主经 C 接口、设置应用经自己的存储都只按目录读写偏好，并不知道版本，偏好文件不见了或被修复时靠这份记录回到本版本的默认偏好。状态目录里还没有偏好文件时，每个版本都会把此刻读到的偏好写成第一份偏好文件：不经偏好存储直接读文件的一方第一次读到的也是本版本的默认值；新装和从以前的版本升级也只在这一刻分得清（触屏键盘的默认方案不同，见 `PreferencesStore::write_first_document`），写下文件后结论就固定了。
pub fn prepare_host_configuration_for_edition(
    resources: &std::path::Path,
    state_root: &std::path::Path,
    edition: &'static Edition,
) -> Result<String, Box<dyn std::error::Error>> {
    let specification = edition.resource_set()?;
    prepare_host_with_updates(resources, state_root, edition, &specification)
}

/// 把还留在资源目录里的自定义辅助码表（`helpcodes/custom`）复制到状态根下的同名目录，Engine 和设置页都先读那里（`msime_engine::load_helpcode_keymap_in`）。资源目录是安装包或词库代次，升级和在线词库更新会换掉或删掉它；准备和刷新运行时配置时各搬一次，表还在就不会跟着旧目录一起丢。只复制，不删源文件，状态根里已有同一主干的表不覆盖。
fn adopt_custom_helpcodes(state_root: &Path, sources: &[&Path]) {
    let Ok(state_root) = std::path::absolute(state_root) else {
        return;
    };
    let adopted = msime_client_core::helpcode::adopt_custom_helpcodes(&state_root, sources);
    if adopted > 0 {
        crate::diagnostics::report(
            "custom helpcode tables adopted",
            &format!("copied {adopted} table(s) from the resource directory into the state root"),
        );
    }
}

/// 安装选择与随包基线共用准备流程。
fn prepare_host_with_updates(
    resources: &Path,
    state_root: &Path,
    edition: &'static Edition,
    specification: &ResourceSet,
) -> Result<String, Box<dyn std::error::Error>> {
    adopt_custom_helpcodes(state_root, &[resources]);
    prepare_shipped_host_configuration(
        resources,
        state_root,
        specification,
        ON_DEMAND_ARTIFACTS,
        edition,
    )
}

/// 状态目录里 `edition` 的偏好。还没有偏好文件时，先把此刻读到的偏好写成第一份文件（[`PreferencesStore::write_first_document`]），理由见 [`prepare_host_configuration_for_edition`]。必须在建用户词库代次之前调用：没有偏好文件时，状态目录有没有被准备过决定触屏键盘按新装还是按升级前的默认方案。
///
/// 已有的偏好文件读不了（更新的版本写的取值、本版本校验更严、写坏的字节）时不让准备失败，否则用户完全打不了字（#9802 那次 iOS 键盘、HarmonyOS 和 Windows 的 Server 都是这样）：改用在内存里救下的设置（[`PreferencesStore::load_for_runtime`]），文件不动，原因经诊断出口报告。只有存储本身出错时准备才失败。
fn edition_preferences(
    state_root: &Path,
    edition: &'static Edition,
) -> Result<Preferences, Box<dyn std::error::Error>> {
    let store = PreferencesStore::for_edition(state_root, edition);
    match store.write_first_document() {
        Ok(snapshot) => Ok(snapshot.preferences),
        Err(msime_client_core::preferences::PreferencesError::Io(error)) => Err(error.into()),
        Err(_) => {
            let runtime = store.load_for_runtime()?;
            report_unreadable_preferences(&runtime);
            Ok(runtime.snapshot.preferences)
        }
    }
}

/// [`prepare_host_configuration_for_edition`] 按给定的锁文件和按需下载清单准备；测试借它在各平台上检查 macOS 的发货规则。
fn prepare_shipped_host_configuration(
    resources: &std::path::Path,
    state_root: &std::path::Path,
    specification: &ResourceSet,
    on_demand: &[&[&str]],
    edition: &'static Edition,
) -> Result<String, Box<dyn std::error::Error>> {
    let resources = without_verbatim_prefix(std::fs::canonicalize(resources)?);
    let state_root = std::path::absolute(state_root)?;
    verify_resources_once(&resources, specification, &state_root, on_demand)?;
    // 偏好要在建 `user/dictionaries` 之前读：没有偏好文件时，以前的版本准备过的状态目录按升级前的默认方案，新装按现在的默认方案，建了代次目录就分不清了。
    let preferences = edition_preferences(&state_root, edition)?;
    // 用户词库日志读不出是数据库时，代次准备会把它改名挪开、从空日志开始，否则回放每次都失败、宿主永远起不来。在这里先挪，好经诊断出口报出来；准备代次时再查一遍只是空操作。
    let journal = state_root
        .join("user")
        .join(msime_engine::assets::USER_JOURNAL);
    if let Some(moved) = msime_engine::quarantine_corrupt_journal(&journal)? {
        crate::diagnostics::report(
            "user dictionary journal moved aside",
            &format!(
                "{} is not a database; learning starts again from an empty journal",
                moved.display()
            ),
        );
    }
    // 代次按本版本的方案准备：不含全拼、双拼、五笔的版本（日文、越南文、藏文）随包不带 msime-pinyin.db，代次里也没有它；full 的方案是全部，与以前相同。
    let prepared = msime_engine::host::prepare_options_for(
        resources.to_str().ok_or("non-UTF-8 resource path")?,
        state_root
            .join("user")
            .to_str()
            .ok_or("non-UTF-8 state path")?,
        state_root
            .join("cache")
            .to_str()
            .ok_or("non-UTF-8 cache path")?,
        // 代次按完整锁文件计算，不随发布包是否内置日文词典而变：user/dictionaries/<generation> 和 refreshed_host_options 都保持原样，升级后不会重新准备，仍在运行的旧版输入法也不会。
        &specification.generation()?,
        engine_schemes(edition),
    )?;
    // 两个租约锁文件在这里就建好：对状态目录没有写权限的宿主（iOS 键盘没有完全访问）只能只读打开已有的锁文件取租约，自己建不出来（#10945，见 `read_only_state`）。取到就放；正被维护占着时文件也已经在了。
    let _ = DictionaryAccess::try_session(
        Path::new(&prepared.user_data),
        Path::new(&prepared.dictionaries),
    );
    // 先记下状态目录属于哪个版本：之后只拿到这个目录的偏好存储（各平台的 C ABI、设置应用）靠它取本版本的默认偏好。full 什么也不写。
    edition.record_in(&state_root)?;
    Ok(serde_json::to_string_pretty(&prepared_host_options(
        prepared,
        &resources,
        &state_root,
        preferences,
        edition,
    )?)?)
}

/// 准备好的引擎路径上的宿主选项：可写的准备（[`prepare_shipped_host_configuration`]）和只读的准备（[`read_only_state`]）共用，`resources` 是校验过的资源目录。
fn prepared_host_options(
    prepared: msime_engine::host::EngineOptions,
    resources: &Path,
    state_root: &Path,
    preferences: Preferences,
    edition: &'static Edition,
) -> Result<HostOptions, Box<dyn std::error::Error>> {
    Ok(HostOptions {
        api_version: 1,
        resources: prepared.resources,
        helpcode_fallback_resources: None,
        user_data: prepared.user_data,
        cache: prepared.cache,
        dictionaries: prepared.dictionaries,
        preferences,
        preferences_directory: Some(
            state_root
                .to_str()
                .ok_or("non-UTF-8 state path")?
                .to_owned(),
        ),
        phrase_preedit: None,
        selection_statistics: None,
        keyboard_reranker: None,
        rerank_models: None,
        pause_rerank: None,
        quick_phrase_candidates: None,
        korean_romaja: None,
        clipboard_history_path: None,
        online_provider_socket: None,
        translation_provider_socket: None,
        cloud_dictionary_provider_socket: None,
        cloud_clipboard_provider_socket: None,
        voice_provider_socket: None,
        // Written only when a model has actually been installed as its own artifact; a host that
        // places one beside the dictionaries needs no configuration.
        sentence_model: None,
        settled_model: settled_model_beside(resources),
        sound_packs: None,
        plugin_packs: None,
        language_dictionaries: installed_language_dictionaries(resources),
        edition: (!edition.is_full()).then_some(edition),
        read_only_state: None,
    })
}

/// A runtime options refresh found that the recorded resource directory does not hold the resource set this build pins.
///
/// This is the state a user who downloaded the dictionaries (rather than getting them from the package) is left in after an upgrade that raised the dictionary version: the package replaced the lock but nothing replaced the files. It is told apart from every other refresh failure because only this one has a fix the user can run, `msime-linux-setup --update --download`. The C ABI passes errors through as their `Display` text, so the stable part of the contract is the `dictionary_outdated:` prefix; what follows it is diagnostic and may name private paths.
#[derive(Debug)]
pub struct DictionaryOutdated(msime_client_core::resources::ResourceError);

impl std::fmt::Display for DictionaryOutdated {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{DICTIONARY_OUTDATED_PREFIX} {}", self.0)
    }
}

impl std::error::Error for DictionaryOutdated {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// How a [`DictionaryOutdated`] error begins once it has crossed the C ABI as text.
pub const DICTIONARY_OUTDATED_PREFIX: &str = "dictionary_outdated:";

/// Turn a verification failure out of `prepare_host_configuration` into [`DictionaryOutdated`], leaving every other error as it was. Only a length, digest or directory-content mismatch counts: an unreadable file or an invalid compiled lock is not something a download fixes.
fn outdated_resources(error: Box<dyn std::error::Error>) -> Box<dyn std::error::Error> {
    use msime_client_core::resources::ResourceError;
    match error.downcast::<ResourceError>() {
        Ok(resource) => match *resource {
            outdated @ (ResourceError::Integrity | ResourceError::ExistingGeneration(_)) => {
                Box::new(DictionaryOutdated(outdated))
            }
            other => Box::new(other),
        },
        Err(error) => error,
    }
}

fn reject_symlinked_options_parent(path: &Path) -> std::io::Result<()> {
    let mut current = path.parent();
    while let Some(candidate) = current {
        match std::fs::symlink_metadata(candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if msime_path_trust::is_trusted_system_alias(candidate) {
                    current = candidate.parent();
                    continue;
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "options path has a symbolic-link parent",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        current = candidate.parent();
    }
    Ok(())
}

/// Bring a published HostOptions file up to the installed dictionary generation.
///
/// A package upgrade replaces the resource bundle in place but leaves each user's options pointing at working dictionaries copied from the previous bundle, so the new dictionary never reaches the Engine and the user-dictionary replay the Windows installer runs after an upgrade never happens. When the recorded dictionaries directory is not the generation the installed lock describes, this prepares that generation (the Engine copies the new dictionaries and replays the user journal into them) and rewrites only `resources` and `dictionaries`, keeping every other key a setup or the settings app wrote. A current file is only read.
///
/// Returns whether the file was rewritten. Run it before the caller's own sessions exist. The previous generation is never modified, so a host still using it keeps working until it restarts. A symlink, or a document whose paths do not follow the layout `prepare_host_configuration` produces, is left alone rather than guessed at. When the recorded resources do not match the compiled lock the error is [`DictionaryOutdated`] and the file is left as it was.
pub fn refresh_host_options(path: &std::path::Path) -> Result<bool, Box<dyn std::error::Error>> {
    refresh_options_file(path, false, None)
}

/// 给自带一份已校验资源的宿主用的 [`refresh_host_options`]（macOS 设置应用 bundle 里的 `EngineResources` 就是这样一份）：记录的资源目录与编译进来的词库锁不符（[`DictionaryOutdated`]）时，改用 `bundled` 准备新代次，此后 `resources` 指向它。
///
/// 记录的资源目录不一定是安装包会替换的那一个。手工暂存到 Application Support 的目录、在输入法「准备词库」里选的目录，都停在暂存时的代次上，之后每次升级都以 [`DictionaryOutdated`] 失败，用户既拿不到新词库，也拿不到安装包放在自带资源旁的粤语、注音与笔画词库。记录的目录仍是当前代次，或者失败是别的原因时，处理与 [`refresh_host_options`] 完全相同；这里同样不碰 `language_dictionaries`。
pub fn refresh_host_options_from(
    path: &std::path::Path,
    bundled: &std::path::Path,
) -> Result<bool, Box<dyn std::error::Error>> {
    refresh_options_file(path, false, Some(bundled))
}

/// 在 [`refresh_host_options`] 之外，不论代次是否变化，都让 `language_dictionaries` 跟上资源目录旁实际安装的粤语、注音与笔画词库；两项都不需要改时只读一次文件。代次准备失败也不会挡住这一项：仍按记录的资源目录更新这个键，然后再返回准备失败的错误，`resources` 与 `dictionaries` 保持原样。
///
/// 只有输入法进程自己在启动时、任何会话读取这份文件之前调用它。每个输入法会话都会重读这份文件，而 `HostOptions` 拒绝未知键，往一个仍被旧版输入法读取的文件里加键会让它再也开不了会话。设置应用升级后旧版输入法可能还在运行，所以设置应用自己的刷新是 [`refresh_host_options`]；运行这段代码的输入法认识它写入的键。
pub fn refresh_host_options_with_language_dictionaries(
    path: &std::path::Path,
) -> Result<bool, Box<dyn std::error::Error>> {
    refresh_options_file(path, true, None)
}

fn refresh_options_file(
    path: &std::path::Path,
    language_dictionaries: bool,
    bundled: Option<&std::path::Path>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("runtime options have no directory")?;
    let name = path
        .file_name()
        .ok_or("runtime options have no file name")?;
    let directory = msime_client_core::file_lock::open_private_directory(parent)?;
    reject_symlinked_options_parent(path)?;
    let locator = std::fs::symlink_metadata(path)?;
    if locator.file_type().is_symlink() {
        return Ok(false);
    }
    let file = msime_client_core::file_lock::open_private_file_at(&directory, name)?;
    let metadata = file.metadata()?;
    // The file can be replaced or grow after symlink_metadata returns. Read through a
    // limit-aware handle so that the size check remains effective across that race.
    let bytes =
        crate::bounded_file::read(file, HOST_OPTIONS_DOCUMENT_LIMIT as u64).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData {
                Box::<dyn std::error::Error>::from("runtime options exceed 1 MiB")
            } else {
                Box::<dyn std::error::Error>::from(error)
            }
        })?;
    let document: Value = serde_json::from_slice(&bytes)?;
    // 刷新可能把 `resources` 换成别的目录（新代次、应用包里的那份），换之前先把旧目录里的自定义辅助码表搬到状态根下。
    if let Some(state) = document["preferences_directory"]
        .as_str()
        .map(Path::new)
        .filter(|state| state.is_absolute())
    {
        let sources: Vec<&Path> = ["resources", "helpcode_fallback_resources"]
            .into_iter()
            .filter_map(|key| document[key].as_str().map(Path::new))
            .collect();
        adopt_custom_helpcodes(state, &sources);
    }
    // 文档记录的版本，缺省是 full。新代次按同一个版本准备；`refreshed_layout` 只替换 `resources` 和 `dictionaries`，`edition` 键原样保留。
    let edition =
        Edition::of_host_options(&document).ok_or("runtime options name an unknown edition")?;
    let baseline = edition.resource_set()?;
    let prepared = refreshed_host_options(
        &document,
        &baseline.generation()?,
        bundled,
        Path::is_dir,
        |resources, state| {
            Ok(serde_json::from_str(
                &prepare_host_configuration_for_edition(resources, state, edition)
                    .map_err(outdated_resources)?,
            )?)
        },
    );
    // 语言词库随安装包到来，与词库代次无关；代次准备失败（最常见的是没有任何安装包会升级的资源目录，见 `refresh_host_options_from`）不能让它们进不了配置。
    let languages = if language_dictionaries {
        let current = match &prepared {
            Ok(Some(prepared)) => prepared,
            _ => &document,
        };
        with_installed_language_dictionaries(current)?
    } else {
        None
    };
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            if let Some(languages) = languages {
                replace_options_file(&directory, name, &metadata, &languages)?;
            }
            return Err(error);
        }
    };
    let Some(refreshed) = languages.or(prepared) else {
        return Ok(false);
    };
    replace_options_file(&directory, name, &metadata, &refreshed)?;
    Ok(true)
}

/// 用已打开的父目录句柄原子替换配置文件，保留原有权限。
fn replace_options_file(
    directory: &msime_client_core::file_lock::PrivateDirectory,
    name: &std::ffi::OsStr,
    metadata: &std::fs::Metadata,
    document: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut serialized = serde_json::to_vec_pretty(document)?;
    serialized.push(b'\n');
    msime_client_core::file_lock::replace_private_file_at(
        directory,
        name,
        &serialized,
        &metadata.permissions(),
    )?;
    Ok(())
}

/// 把输入法进程这次启动准备好的词库代次写进已发布的 `runtime-options.json`，只改 `dictionaries` 一个键；文件已经是这个代次时只读不写。返回是否改写了文件。
///
/// Windows 的 Server 每次启动都重新准备宿主（`msime_client_prepare_host`），自己的会话用准备出的文档；`runtime-options.json` 却只在首次运行时写下（`FirstRun.h`）。升级换了词库锁、或者装上了在线词库更新之后，Server 的会话读新代次，而照这份文件工作的设置窗口、`msime-mcp` 和各应用进程里的 TIP 还在旧代次上：设置里新增的快捷短语和五笔词条写进旧代次，Server 要等下次启动回放日志才看得到，用户看到的就是「加了没用」（#9434）。
///
/// 只改 `dictionaries`，不动 `resources`：在线词库更新的资源目录在状态目录下，而 Server 启动时从这份文件读 `resources` 并要求它与状态目录不相交，改了会让 Server 下次起不来。词库新增、编辑和查找都只经过代次里的工作副本，`resources` 留在安装包目录不影响它们。两份文档都必须是准备布局（[`prepared_layout`]）且属于同一个状态目录，否则原样不动；写入的路径沿用文件自己的拼写，只换代次目录名。
pub fn publish_prepared_dictionaries(
    path: &Path,
    prepared: &Value,
) -> Result<bool, Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("runtime options have no directory")?;
    let name = path
        .file_name()
        .ok_or("runtime options have no file name")?;
    let directory = msime_client_core::file_lock::open_private_directory(parent)?;
    reject_symlinked_options_parent(path)?;
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(false);
    }
    let file = msime_client_core::file_lock::open_private_file_at(&directory, name)?;
    let metadata = file.metadata()?;
    let bytes = crate::bounded_file::read(file, HOST_OPTIONS_DOCUMENT_LIMIT as u64)?;
    let mut document: Value = serde_json::from_slice(&bytes)?;
    let (Some((_, published, state)), Some((_, generation, prepared_state))) =
        (prepared_layout(&document), prepared_layout(prepared))
    else {
        return Ok(false);
    };
    // 同一个状态目录可以写成不同的字面路径（Windows 的盘符大小写、短文件名），按规范化后的路径比较；文件记的状态目录已经不在，就不是这次准备的宿主的文件。
    let same_state = std::fs::canonicalize(state).is_ok_and(|state| {
        std::fs::canonicalize(prepared_state).is_ok_and(|prepared| state == prepared)
    });
    if !same_state {
        return Ok(false);
    }
    // 两份文档的拼写可能不同，只取代次目录名，接在文件自己的 `user_data/dictionaries` 下：照搬准备出的整条路径会让 `dictionaries` 与文件里的 `user_data` 拼写不一致，文件从此不再是准备布局，之后的发布和刷新都会跳过它。
    let (Some(current), Some(generation_name)) = (published.parent(), generation.file_name())
    else {
        return Ok(false);
    };
    if published.file_name() == Some(generation_name) {
        return Ok(false);
    }
    document["dictionaries"] = Value::String(
        current
            .join(generation_name)
            .to_str()
            .ok_or("dictionary generation path is not UTF-8")?
            .to_owned(),
    );
    replace_options_file(&directory, name, &metadata, &document)?;
    Ok(true)
}

/// 随包资源所在的目录：安装包里 Engine 资源目录的上一级，设置应用在这里找背单词的内置词书（`wordbooks/`）。
pub fn bundled_resource_root(document: &Value) -> Option<PathBuf> {
    document
        .get("resources")
        .and_then(Value::as_str)
        .map(Path::new)
        .filter(|resources| resources.is_absolute())?
        .parent()
        .map(Path::to_path_buf)
}

/// The `resources`, `dictionaries` and state directory of a document in the layout `prepare_host_configuration` produces, or `None` for any other document.
fn prepared_layout(document: &Value) -> Option<(&Path, &Path, &Path)> {
    let path = |key: &str| {
        document
            .get(key)
            .and_then(Value::as_str)
            .map(Path::new)
            .filter(|path| path.is_absolute())
    };
    let (resources, user_data, dictionaries, state) = (
        path("resources")?,
        path("user_data")?,
        path("dictionaries")?,
        path("preferences_directory")?,
    );
    (user_data == state.join("user")
        && dictionaries.parent() == Some(user_data.join("dictionaries").as_path()))
    .then_some((resources, dictionaries, state))
}

/// `refresh_host_options` 会发布的配置；文件已是当前代次或不符合准备布局时为 `None`。记录的资源目录是 [`DictionaryOutdated`]、而宿主自带 `bundled` 资源时，改用它准备代次。
fn refreshed_host_options(
    document: &Value,
    generation: &str,
    bundled: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
    mut prepare: impl FnMut(&Path, &Path) -> Result<Value, Box<dyn std::error::Error>>,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let Some((resources, dictionaries, state)) = prepared_layout(document) else {
        return Ok(None);
    };
    // 记录的资源目录已经不存在，最常见的是用户把设置应用挪了位置（例如从 /Applications 挪到 ~/Applications）：代次没变，只看代次就会让 `resources` 一直指向不存在的旧路径，下一次开会话就找不到词库。带着 bundle 刷新时改从 bundle 准备，不论代次是否变化。
    if let Some(bundled) = bundled.filter(|bundled| *bundled != resources && !exists(resources)) {
        return refreshed_layout(document, prepare(bundled, state)?).map(Some);
    }
    if dictionaries.file_name().and_then(|name| name.to_str()) == Some(generation) {
        return Ok(None);
    }
    let prepared = match (prepare(resources, state), bundled) {
        (Err(error), Some(bundled)) if error.is::<DictionaryOutdated>() && bundled != resources => {
            prepare(bundled, state)?
        }
        (prepared, _) => prepared?,
    };
    refreshed_layout(document, prepared).map(Some)
}

/// `document` with `resources` and `dictionaries` taken from what `prepare_host_configuration` returned.
fn refreshed_layout(
    document: &Value,
    prepared: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut refreshed = document.clone();
    for key in ["resources", "dictionaries"] {
        refreshed[key] = prepared
            .get(key)
            .filter(|value| value.is_string())
            .cloned()
            .ok_or("prepared options are incomplete")?;
    }
    if let Some(value) = prepared
        .get("language_dictionaries")
        .filter(|v| v.is_string())
    {
        refreshed["language_dictionaries"] = value.clone();
    } else if document["language_dictionaries"]
        .as_str()
        .is_some_and(|p| p.contains("dictionary-updates"))
    {
        refreshed
            .as_object_mut()
            .ok_or("invalid host options")?
            .remove("language_dictionaries");
    }
    for key in ["settled_model", "sound_packs", "plugin_packs"] {
        if refreshed.get(key).is_none() {
            if let Some(value) = prepared.get(key) {
                refreshed[key] = value.clone();
            }
        }
    }
    Ok(refreshed)
}

/// 为准备流程生成的文档补齐语言词库路径；无需变化或布局不匹配时返回 `None`。
///
/// 优先资源相邻目录，其次已校验的下载语言代次，最后是更新记录中的原安装包目录。
/// 主词库代次不变也需刷新；显式指定且仍存在的其他目录继续保留。
fn with_installed_language_dictionaries(
    document: &Value,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let Some((resources, _, state)) = prepared_layout(document) else {
        return Ok(None);
    };
    let beside = language_dictionaries_directory(resources)
        .and_then(|directory| directory.to_str().map(str::to_owned));
    let recorded = document
        .get("language_dictionaries")
        .and_then(Value::as_str);
    // 指向别处、而且那个目录还在的记录是有意为之，原样保留；已经不存在的记录（应用挪了位置后留下的旧路径）换成资源目录旁实际安装的那份。
    if recorded
        .is_some_and(|recorded| Some(recorded) != beside.as_deref() && Path::new(recorded).is_dir())
    {
        return Ok(None);
    }
    let _ = state;
    let installed = installed_language_dictionaries(resources);
    if recorded == installed.as_deref() {
        return Ok(None);
    }
    let mut refreshed = document.clone();
    let object = refreshed
        .as_object_mut()
        .ok_or("runtime options are not an object")?;
    match installed {
        Some(directory) => {
            object.insert("language_dictionaries".to_owned(), Value::String(directory));
        }
        None => {
            object.remove("language_dictionaries");
        }
    }
    Ok(Some(refreshed))
}

/// The reason prefix for an entry this layer or the Engine refuses on its own terms. Callers map it to one error code, so it stays stable while the part after the colon says which rule failed.
pub const INVALID_DICTIONARY_ENTRY: &str = "invalid dictionary entry";

pub(crate) fn invalid_dictionary_entry(reason: &str) -> String {
    format!("{INVALID_DICTIONARY_ENTRY}: {reason}")
}

/// 本版本没有中文词库时编辑拼音、五笔或快捷短语的拒绝原因。
pub(crate) const NO_CHINESE_DICTIONARY: &str =
    "this edition of the input method has no Chinese dictionary; only English words can be edited";

/// `kind` 的词能不能在 `options` 的版本里编辑。不含全拼、双拼、五笔的版本（日文、越南文、藏文）随包不带 msime-pinyin.db，拼音、五笔和快捷短语都无处可存，只有英文词和粤语词（粤语词只记在用户日志里）能编辑（`SchemeSet::reads_main_dictionary`）。在加锁之前就拒绝，把原因告诉调用方，而不是让 Engine 拒绝后报成笼统的「dictionary edit rejected」。
pub(crate) fn require_dictionary_kind(
    options: &EngineOptions,
    kind: msime_engine::host::DictionaryKind,
) -> Result<(), String> {
    if kind.stored_without_main_dictionary() || options.enabled_schemes.reads_main_dictionary() {
        Ok(())
    } else {
        Err(NO_CHINESE_DICTIONARY.into())
    }
}

/// Edit only after every participating host has destroyed its sessions.
/// Busy is retryable without cancelling any composition. The host must recreate
/// sessions after success; no native/Tauri management command is exposed yet.
///
/// An entry the Engine refuses is reported as `invalid dictionary entry: <reason>` with the Engine's reason, checked before anything is locked. Those reasons are fixed sentences in `validate_personal_dictionary_entry` that never repeat the submitted entry; collapsing them into one generic "rejected" left the settings page telling the user to retry an entry that could never be saved. Every other Engine diagnostic is still withheld, since it may include user text.
pub fn edit_personal_dictionary(
    options: &EngineOptions,
    previous: Option<&msime_engine::host::DictionaryEntry>,
    replacement: Option<&msime_engine::host::DictionaryEntry>,
    request_id: &str,
) -> Result<(), String> {
    validate_personal_dictionary_edit(options, previous, replacement, request_id)?;
    let access = DictionaryAccess::try_maintenance(
        std::path::Path::new(&options.user_data),
        std::path::Path::new(&options.dictionaries),
    )
    .map_err(|_| "dictionary access unavailable")?
    .ok_or("dictionary maintenance busy")?;
    edit_personal_dictionary_with_access(&access, options, previous, replacement, request_id)
}

/// 在调用方持有维护租约时校验并应用一个词条改动。
/// 租约按借用传入，让有界批次共用一个独占窗口，不必为每个待处理词条重新取锁。
pub(crate) fn edit_personal_dictionary_with_access(
    _access: &DictionaryAccess,
    options: &EngineOptions,
    previous: Option<&msime_engine::host::DictionaryEntry>,
    replacement: Option<&msime_engine::host::DictionaryEntry>,
    request_id: &str,
) -> Result<(), String> {
    validate_personal_dictionary_edit(options, previous, replacement, request_id)?;
    msime_engine::host::dictionary_edit(options, previous, replacement, request_id)
        .map_err(|_| "dictionary edit rejected".into())
}

fn validate_personal_dictionary_edit(
    options: &EngineOptions,
    previous: Option<&msime_engine::host::DictionaryEntry>,
    replacement: Option<&msime_engine::host::DictionaryEntry>,
    request_id: &str,
) -> Result<(), String> {
    for entry in previous.iter().chain(replacement.iter()) {
        require_dictionary_kind(options, entry.kind)?;
    }
    // The previous row is one the list returned, whose weight learning may have lifted past the ceiling a new entry is held to.
    for entry in previous.iter() {
        msime_engine::host::dictionary_validate_previous(entry)
            .map_err(|error| invalid_dictionary_entry(&error.to_string()))?;
    }
    for entry in replacement.iter() {
        msime_engine::host::dictionary_validate(entry)
            .map_err(|error| invalid_dictionary_entry(&error.to_string()))?;
    }
    if request_id.is_empty() {
        return Err("dictionary request id required".into());
    }
    Ok(())
}

/// A host-owned view of one entry in the verified local Emoji catalog.
#[derive(Clone, Debug, Serialize)]
pub struct LocalEmojiCatalogItem {
    pub text: String,
    pub annotation: String,
    pub group: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct LocalEmojiCatalogSlice {
    pub items: Vec<LocalEmojiCatalogItem>,
    pub next_offset: usize,
    pub complete: bool,
}

/// Read catalog rows without collapsing equal text from distinct categories.
// Not unix-gated: the bodies only call the engine, which builds on Windows too. The gate was a porting gap, and it left the Windows desktop falling back to the compact built-in catalog - 97 emoji against the several thousand rows in msime-others.db - behind a permanent "catalog failed to load" banner.
pub fn local_emoji_catalog_slice(
    resources: &str,
    category: &str,
    offset: usize,
    limit: u16,
) -> Result<LocalEmojiCatalogSlice, &'static str> {
    if !std::path::Path::new(resources).is_absolute() {
        return Err("resources path must be absolute");
    }
    if limit == 0 || limit > 4096 {
        return Err("invalid local emoji page size");
    }
    msime_engine::host::emoji_catalog_slice(resources, "", category, "", offset, limit, "")
        .map(|page| LocalEmojiCatalogSlice {
            items: page
                .items
                .into_iter()
                .map(|item| LocalEmojiCatalogItem {
                    text: item.text,
                    annotation: item.annotation,
                    group: item.group,
                })
                .collect(),
            next_offset: page.next_offset,
            complete: page.complete,
        })
        .map_err(|_| "local emoji catalog unavailable")
}

#[derive(Clone, Debug, Serialize)]
pub struct LocalSymbolCatalogGroup {
    pub parent: String,
    pub title: String,
    pub items: Vec<LocalEmojiCatalogItem>,
}

/// Preserve Engine-owned symbol parent categories and subgroup order.
pub fn local_symbol_catalog(resources: &str) -> Result<Vec<LocalSymbolCatalogGroup>, &'static str> {
    if !std::path::Path::new(resources).is_absolute() {
        return Err("resources path must be absolute");
    }
    let groups = msime_engine::host::emoji_symbol_groups(resources)
        .map_err(|_| "local symbol catalog unavailable")?;
    let mut result = Vec::with_capacity(groups.len());
    let mut remaining_pages = 256usize;
    for group in groups {
        let mut items = Vec::new();
        let mut first_page = true;
        let mut offset = 0usize;
        loop {
            if remaining_pages == 0 {
                return Err("local symbol catalog exceeds limit");
            }
            remaining_pages -= 1;
            let page = msime_engine::host::emoji_catalog_slice(
                resources,
                "",
                "symbols",
                &group.title,
                offset,
                512,
                &group.parent,
            )
            .map_err(|_| "local symbol catalog unavailable")?;
            if first_page {
                // The engine exposes pages rather than a total count. Use the first
                // page's actual item count as the only reliable initial capacity.
                items = Vec::with_capacity(page.items.len());
                first_page = false;
            }
            let complete = page.complete;
            let next_offset = page.next_offset;
            items.extend(page.items.into_iter().map(|item| LocalEmojiCatalogItem {
                text: item.text,
                annotation: item.annotation,
                group: item.group,
            }));
            if complete {
                break;
            }
            if next_offset <= offset {
                return Err("local symbol catalog cursor did not advance");
            }
            offset = next_offset;
        }
        result.push(LocalSymbolCatalogGroup {
            parent: group.parent,
            title: group.title,
            items,
        });
    }
    Ok(result)
}

/// 列出设置页可选的自定义辅助码表：状态根 `state_root` 下的 `helpcodes/custom`（用户自己的表放在这里，升级不会动它）和资源目录里的 `helpcodes/custom`（以前的位置），同一主干取状态根那份，与 Engine 读表的顺序一致。
///
/// 码表文件归 Engine 解析，显示名属于设置界面。路径检查留在宿主边界上，原生调用方和 Tauri 外壳都不能让 client-core 去看一个相对路径。目录不在或读不了时是空列表。`state_root` 为 `None` 时只看资源目录。
pub fn list_custom_helpcode_schemas(
    resources: &str,
    state_root: Option<&str>,
) -> Result<Vec<msime_client_core::helpcode::CustomHelpcodeSchema>, &'static str> {
    let path = std::path::Path::new(resources);
    if !path.is_absolute() {
        return Err("resources path must be absolute");
    }
    let state_root = state_root.map(std::path::Path::new);
    if state_root.is_some_and(|state_root| !state_root.is_absolute()) {
        return Err("state root must be absolute");
    }
    Ok(msime_client_core::helpcode::list_custom_helpcode_schemas_in(path, state_root))
}

fn response(operation: impl FnOnce() -> Result<Value, String>) -> *mut c_char {
    let value = match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => json!({ "ok": true, "value": value }),
        Ok(Err(error)) => json!({ "ok": false, "error": error }),
        Err(_) => json!({ "ok": false, "error": "internal runtime failure" }),
    };
    // JSON escapes embedded NUL bytes, so this cannot contain an interior NUL.
    CString::new(value.to_string())
        .expect("JSON contains no NUL")
        .into_raw()
}

fn with_session<T>(
    handle: u64,
    action: impl FnOnce(&mut HostSession) -> Result<T, String>,
) -> Result<T, String> {
    SESSIONS.with(|sessions| {
        let mut sessions = sessions
            .try_borrow_mut()
            .map_err(|_| "reentrant host call")?;
        let runtime = sessions
            .get_mut(&handle)
            .ok_or("unknown session or wrong thread")?;
        action(runtime)
    })
}

fn dispatch(handle: u64, action: Action) -> *mut c_char {
    dispatch_call(handle, action, None)
}

/// 派发一个动作。`host` 只对命令 19、20 有值：动作同 Cancel、Finish，生命周期里的类别是 `HostCommit`（结束原因记 HostCommit）或 `FocusLoss`（上屏来源记 FocusLoss，不出声不画）。
///
/// 选择加入生命周期的会话按这个顺序：调用前采样并发 `input` → `runtime.dispatch`（脚本的 `candidates` 在里面的 refresh 和展开之后）→ 统计 → 读派发事实 → `complete_transition`（全角转换、再中英文之间加空格都在这里，之后才是宿主拿到的字节，宿主再做简繁转换）→ `before_commit`（脚本可以改写 body）→ 其余事件和内置消费者 → 序列化，FEEDBACK 开着时插入 `feedback`。没选择加入的会话只走原来的几步。
fn dispatch_call(handle: u64, action: Action, host: Option<lifecycle::InputClass>) -> *mut c_char {
    response(|| dispatch_value(handle, action, host))
}

/// [`dispatch_call`] 的本体，返回值不经 C 字符串，`candidate_preview` 在本进程里打字时直接用它。
fn dispatch_value(
    handle: u64,
    action: Action,
    host: Option<lifecycle::InputClass>,
) -> Result<Value, String> {
    with_session(handle, |session| {
        // 大写锁定改道先于其余处理，后面看到的都是改道后的动作。
        let action = session.caps_lock_punctuation(action);
        let action = session.runtime.candidate_shortcut(action);
        // 只读会话不改词库（#10945）：在快捷键改道之后拦，组字原样留着，见 `read_only_state::edits_dictionary`。
        if session.read_only_state && read_only_state::edits_dictionary(&action) {
            return Err(read_only_state::CANDIDATE_EDITING_UNAVAILABLE.to_owned());
        }
        let currency_punctuation = matches!(
            action,
            Action::Character { value: b'$', .. } | Action::Punctuation(b'$')
        );
        let pending = session.lifecycle_input(&action, host);
        let result = session.runtime.dispatch(action).map_err(|e| e.to_string());
        // 报错前 runtime 可能已经改了组字，生命周期照样收尾。
        let mut result = session.lifecycle_on_error(pending, result)?;
        let facts = session.runtime.facts();
        // 固定标点表返回全角人民币；半角只改本次 `$` 产生的尾巴，候选和短语里的符号保持原文。
        if currency_punctuation
            && result.view.character_width == CharacterWidth::Halfwidth
            && facts.tail_chars == 1
            && result.commit.as_deref().is_some_and(|c| c.ends_with('￥'))
        {
            // 光标后缀带着同一个单字尾巴，正文以 `￥` 结尾时它也以 `￥` 结尾；两者一起改写，`complete_transition` 核对时后缀仍是正文的真后缀。
            for text in result
                .commit
                .iter_mut()
                .chain(result.commit_cursor_suffix.iter_mut())
            {
                if text.ends_with('￥') {
                    text.truncate(text.len() - '￥'.len_utf8());
                    text.push('¥');
                }
            }
        }
        let mut result = session.complete_transition(result);
        // 脚本插件可能在这里改写上屏的 body（`before_commit`），宿主拿到的是改写之后的文本；引擎学习和 AI 上文已经按原文记下。
        let feedback = session.lifecycle_after(pending, facts, &mut result);
        let mut value = serde_json::to_value(result).map_err(|e| e.to_string())?;
        if let (Some(effect), Value::Object(fields)) = (feedback, &mut value) {
            fields.insert("feedback".into(), json!({ "typing_effect": effect }));
        }
        Ok(value)
    })
}

