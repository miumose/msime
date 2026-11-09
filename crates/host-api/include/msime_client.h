#ifndef MSIME_CLIENT_H
#define MSIME_CLIENT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

/* Host-neutral focus lease and key event contract. Values are opaque to the ABI. */
typedef struct msime_client_focus_lease {
  uint64_t client;
  uint64_t epoch;
  uint64_t token;
} msime_client_focus_lease;

typedef struct msime_client_key_event {
  msime_client_focus_lease lease;
  uint32_t virtual_key;
  uint32_t scan_code;
  uint32_t modifiers;
  uint32_t character;
  bool ui_less;
} msime_client_key_event;

/* Outcome for platform key-router writes. Only definitely-not-sent may use a
 * local fallback; ambiguous delivery must wait for lease recovery. */
typedef enum msime_client_key_dispatch_result {
  MSIME_CLIENT_KEY_SENT = 0,
  MSIME_CLIENT_KEY_DEFINITELY_NOT_SENT = 1,
  MSIME_CLIENT_KEY_DELIVERY_AMBIGUOUS = 2,
} msime_client_key_dispatch_result;

static inline bool msime_client_key_dispatch_allows_fallback(
    msime_client_key_dispatch_result result) {
  return result == MSIME_CLIENT_KEY_DEFINITELY_NOT_SENT;
}

static inline bool msime_client_key_event_valid(const msime_client_key_event *event) {
  return event != NULL && event->lease.client != 0 && event->lease.epoch != 0 &&
         event->lease.token != 0 && event->virtual_key <= 0xff &&
         (event->modifiers & ~UINT32_C(0x0f)) == 0;
}

/* ABI 3 (3 replaced msime_client_builtin_skins with the global theme functions). All functions return owned, NUL-terminated UTF-8 JSON. Free exactly once using msime_client_string_free, including error responses. Never use free(). Responses: {"ok":true,"value":...} or {"ok":false,"error":"..."}. Creation returns a View; its session field is the handle. Handles are confined to their creating thread. Dispatch, focus, view and destroy on that thread. Text and candidate values are copied; no Engine pointers escape. */
uint32_t msime_client_abi_version(void);

/* Worker-thread bootstrap: {resources: absolute path, state_root: absolute path}.
 * Verifies pinned resources, delegates working data preparation to Engine and
 * returns HostOptions. Maximum 16384 bytes; no session may use state_root during
 * preparation. Caller publishes the returned config atomically after success.
 * 可选的 edition 是版本 id（shared/contracts/editions.json），缺省为 full；不是 full 时返回的 HostOptions 带 edition 键，未知 id 返回错误。
 * 可选的 runtime_options 是已发布的 runtime-options.json 的绝对路径：准备成功后，文件里的 dictionaries 不是这次准备的代次时改写成它，其余键不动；改写失败只记诊断，不影响返回值。Windows Server 每次启动都重新准备、会话用返回的文档，靠它让照这份文件工作的设置窗口和 TIP 跟上同一个代次。
 * read_only_state 为 true 时宿主对 state_root 没有写权限（iOS 键盘没有完全访问）：只读地准备，state_root 里一个字节也不写，必须同时给 private_root（宿主自己能写、不在 state_root 里的绝对路径），不能给 runtime_options。偏好从 state_root 读；别的进程准备好了这个版本的代次、两把租约锁文件也在时用它，否则用 resources 的只读布局（没有用户词库）。返回的 HostOptions 带 read_only_state: true，只放在内存里，建出的会话不学习、租约只读打开已有的锁文件，msime_client_pin_candidate、msime_client_remove_candidate、msime_client_fix_candidate_position、msime_client_clear_candidate_position 一律以 "candidate editing needs write access to the shared state" 失败，组字不变。
 */
char *msime_client_prepare_host(const uint8_t *options, size_t length);

/* path is an absolute UTF-8 runtime options file path of length bytes; maximum 4096. When its dictionaries directory is not the installed resource generation (after a package upgrade), prepares that generation, replays the user dictionary into it and atomically rewrites resources/dictionaries, keeping every other key. It also keeps language_dictionaries in step with the Cantonese, Zhuyin and Stroke dictionaries installed beside the resources, whatever the generation; only an input method host may call it, because a host older than this library rejects that key. Value is true when the file was rewritten. Call before creating any session from the file. When the recorded resource directory does not match the compiled dictionary lock (downloaded dictionaries an upgrade did not replace) the error text begins with "dictionary_outdated:" and the file is left unchanged; the rest of that text may name private paths. */
char *msime_client_refresh_host(const uint8_t *path, size_t length);

/* options is a readable UTF-8 buffer of length bytes; maximum 1 MiB.
 * Object: api_version=1, resources/user_data/cache/dictionaries (absolute paths),
 * preferences={scheme, candidate_page_size, learning, chinese_punctuation,
 *              shuangpin_profile?, shuangpin_custom_profile?, wubi_profile?}。缺省 shuangpin_profile 为 xiaohe，
 * 可选 xiaohe、ziranma、shoudao、microsoft、smart_abc、custom。custom 读 shuangpin_custom_profile（{initials, finals, zero_initials}，各是单位到键的对象），表缺失或不合法时按 xiaohe 建会话。缺省 wubi_profile 为 wubi86，可选 wubi86、wubi98、wubixsj（新世纪五笔，词库里要有 wubixsj 表才有候选）。不认识的取值一律拒绝。
 * Optional preferences_directory is bootstrap metadata for host file monitoring;
 * session creation itself does not monitor or load it.
 * Optional phrase_preedit=true asks for a half-composed phrase to stay in the
 * composition: picking a candidate that covers only part of the input leaves the
 * chosen piece in view.phrase_prefix instead of committing it, and the whole
 * phrase commits at once when the composition ends. The host must draw that
 * field ahead of editing_text - it is separate because caret_position is an
 * offset into editing_text in the host's own string unit. Omitted means each
 * piece is committed as it is picked, as before.
 * Optional selection_statistics=false stops this session from recording candidate positions and commit efficiency in <preferences_directory>/typing-statistics.json. It is for a mirror session that repeats selections another session already made: the Windows TIP's in-process session follows the Server's choice, and only the Server's session records it. Omitted means the session records them.
 * The host must prepare and validate its dictionary generation before creation.
 * Creation acquires cooperative shared access to user_data and dictionaries until
 * destroy. It fails immediately while a participating maintenance writer holds
 * exclusive access. Existing sessions are never cancelled for maintenance.
 * Linux hosts may provide absolute online_provider_socket and
 * translation_provider_socket paths for user-managed Unix-socket services;
 * translation may reuse the online socket when omitted.
 * preferences_directory also names the plugins root, <preferences_directory>/plugins: the
 * installed plugin packs, the enabled command tables the "/" mode reads and the "@" mode's
 * mentions.json. They are read at creation, on preference updates and when a field gains
 * focus, and only while the mode that uses them is on. Optional sound_packs is the absolute
 * path of the bundle's built-in sound packs; absent means sound-packs beside resources.
 * Optional plugin_packs is the absolute path of the official plugin packs the desktop
 * installers ship (<kind>/<id>/ folders: the uuid script plugin, the latex-symbols phrase
 * table); absent means plugin-packs beside the built-in sound packs, and nothing when that
 * directory is not there (mobile, development builds without them).
 * 可选 helpcode_fallback_resources 是随包资源的绝对根目录。仅主资源缺少内置辅助码表时才从这里读取 helpcodes/；已有表、自定义表和插件表继续优先。resources 换成下载的词库代次时，msime_client_prepare_host 自己写这个键；文档没有它时，建会话会从词库更新记录里找随包目录，宿主一般不必设置。宿主自己注入时可仅放在内存配置中，避免旧版宿主读取不认识的持久化键。
 * Do not delete .msime-dictionary-access.lock files. Legacy/external writers do
 * not participate; preparation/upgrades still require stopped sessions.
 */
char *msime_client_create(const uint8_t *options, size_t length);

/* Smart punctuation follow-up gestures. The host holds the snapshots: they belong
 * to its editor, not to Engine, and a session rebuilt while the keyboard was away
 * must not carry a gesture across the gap. The switches that gate them live in the
 * applied preferences, so the session answers rather than the host keeping a copy.
 * arm: {ascii, commit, timestamp_ms, editor_generation, auto_closed_pair} ->
 *      {repeat: {ascii, committed, timestamp_ms, editor_generation}|null,
 *       space: {chinese, ascii, editor_generation}|null}
 * decide: {character, preceding, timestamp_ms, editor_generation, repeat, space} ->
 *      {replace_with: "，"|null, space_ascii: 46|null}
 * `preceding` is what the editor holds before the caret at the moment of the press;
 * both decisions re-read it and decline when it disagrees with the arming, so a
 * stale snapshot can never rewrite the wrong character. A non-null space_ascii means
 * replace the preceding mark with it and swallow the space. Maximum 4096 bytes. */
char *msime_client_smart_punctuation_arm(uint64_t handle, const uint8_t *request, size_t length);

char *msime_client_smart_punctuation_decide(uint64_t handle, const uint8_t *request, size_t length);

/* The shared preference defaults as a JSON document. A host patching one key of
 * a nested preference object needs that object's other fields: the object itself
 * is optional, its members are not.
 */
char *msime_client_default_preferences(void);

/* 触控板设置元数据：defaults 为共享默认值，horizontal_dp 和 vertical_dp 各含 minimum / maximum。纯计算，返回标准 JSON 结果，用 string_free 释放。 */
char *msime_client_text_trackpad_options(void);

/* 「重置所有设置」：directory 是绝对的偏好目录（UTF-8，<=16384 字节）。在偏好锁里读出当前文档，换成本目录所属版本的默认偏好（服务凭据与 fuzzy_pinyin.seeded 保留），再按 expected_revision 比较并交换写回，返回新的 PreferencesSnapshot。修订号不符时失败且不写入。词库、统计不受影响；默认关闭剪贴板历史，因此与 msime_client_save_preferences 一样清空已存的历史。会读写文件，请在工作线程调用。 */
char *msime_client_restore_default_preferences(const uint8_t *directory, size_t length,
                                               uint64_t expected_revision);

/* 「导出设置」：directory 是绝对的偏好目录（UTF-8，<=16384 字节）。返回值 value 是设置文件全文（JSON 字符串，format 为 app.msime.client.preferences），不含语音、AI 辅助和翻译服务的配置与密钥、诊断日志、使用统计和剪贴板历史开关。宿主负责让用户选位置并写盘。会读文件，请在工作线程调用。 */
char *msime_client_export_settings(const uint8_t *directory, size_t length);

/* 「导入设置」：document 是用户选的设置文件内容（<=1048576 字节）。换算成偏好（本机的服务配置与密钥、诊断日志、使用统计和剪贴板历史开关保留，按本目录所属的版本收窄输入方案；不会清空剪贴板历史）后按 expected_revision 比较并交换写回，返回新的 PreferencesSnapshot。失败时 error 为 settings_document_invalid（不是设置文件）、settings_document_macos（macOS 原生设置窗口导出的文件）、settings_document_unsupported（更新版本的文件或读不懂的设置）、settings_conflict（设置已在别处更新）之一，或存储错误的说明；失败时不写入。会读写文件，请在工作线程调用。 */
char *msime_client_import_settings(const uint8_t *directory, size_t directory_length,
                                   uint64_t expected_revision, const uint8_t *document,
                                   size_t document_length);

/* resolve_theme 的 value 可选带 touch_selected_text：仅自定义主题的合法显式选中文字取色器，#RRGGBB；触屏候选条以它覆盖首选文字，缺字段仍用键盘配色，其余候选与底色不变。旧宿主可忽略该字段。 */
/* 候选选中文字色的纯更新补丁，不读写文件。请求 <=1024 字节、拒绝未知键：{current?:全局主题 id|null,value:#RRGGBB|null}。current 缺失/null 沿用既有缺省 system；value 必须明确给出，null 表示清除。返回 {selected_text:#RRGGBB|null,global_theme?:"custom",base?:主题 id,reset_skins:bool}。宿主在刚读到的文档上改 selected_text，应用给出的主题/base；reset_skins 为真时清两个候选皮肤槽，再按原修订号 CAS 保存。清除不修改主题、底和皮肤槽。响应由 msime_client_string_free 释放。 */
char *msime_client_candidate_color_update(const uint8_t *request, size_t length);

/* Per-key double-pinyin hint text for one profile name, as a JSON object mapping an uppercase key (or ";") to "initials / finals" - or to whichever side that key carries. Read out of the Engine's own profile tables so a keyboard face never carries a second copy of the keymap. The reserved name "cantonese" gives the Cantonese shuangpin face. An unknown profile name yields an empty object rather than the default profile's hints. */
char *msime_client_shuangpin_key_hints(const uint8_t *profile, size_t length);

/* The double-pinyin codes of the whole zero-initial syllables for one profile name, as a JSON object mapping each syllable to its two-key code, e.g. {"a":"aa","ang":"ah",...}. Read out of the Engine's own profile tables. An unknown profile name yields an empty object. */
char *msime_client_shuangpin_zero_initials(const uint8_t *profile, size_t length);

/* Load PreferencesStore from an absolute UTF-8 directory, without a session.
 * May block on disk/file lock: use a worker thread. Returns PreferencesSnapshot.
 * Missing file returns shared defaults; malformed/future files return errors.
 * Creates the directory/lock file if absent, never overwrites preference contents.
 */
char *msime_client_load_preferences(const uint8_t *directory, size_t length);

/* Android 本机旧布局迁移：{directory:绝对路径,revision:u64,long_space:bool,number_row:bool,nine_key_symbols?:[string],nine_key_digit_symbols?:[string],nine_key_swipe?:string}。
 * 两张九键左侧符号表是本机旧键里不等于出厂表的自定义表，缺省或空表表示不迁；共享偏好里这一项缺键或是空数组时才写入。
 * nine_key_swipe 是本机旧设置 platform.android.nine_key_swipe 的原值（off/up/down），本机没有时不带。
 * 在偏好锁内按原始字段决定迁移并校验修订号；未知样式原样保留。返回 PreferencesSnapshot。
 * 请求最多 64 KiB，工作线程调用，结果用 msime_client_string_free 释放。 */
char *msime_client_migrate_android_key_layout(const uint8_t *request, size_t length);

/* 可重绑切换热键（keybindings.toggle_english_candidates、toggle_full_width、toggle_punctuation、toggle_voice_input）按共享 KeyChord 契约解码。请求是 JSON {"keybindings": object}（<=65536 字节）；keybindings 缺省或为 null 时按出厂值，不是 object 时返回错误。value 是 {"english_candidates","full_width","punctuation","voice_input"}，每项是 null（写法为空或不合法：不绑定）或 {"key": 主键规范名, "ctrl": bool, "alt": bool, "shift": bool, "meta": bool}；规范名是 client-core KeyChord 的 Display 输出（KeyA-KeyZ、Digit0-Digit9、F1-F12、Space、Semicolon、Equal、Comma、Minus、Period、Slash、Backquote、BracketLeft、Backslash、BracketRight、Quote），宿主按规范名认键，写法的解析只在 Rust 一侧。结果用 msime_client_string_free 释放。 */
char *msime_client_toggle_hotkey_chords(const uint8_t *request, size_t length);

// 录键试写：{"field": 槽位, "chord": 写法, "keybindings": 偏好, "voice_ctrl_f9": bool}。
// value 为 {"ok": true, "chord": 规范写法}，或 {"ok": false, "kind": "invalid"|"conflict", "owner"?: 占用者}。
// 请求最多 64 KiB，不落盘；返回值通过 msime_client_string_free 释放。
char *msime_client_check_toggle_hotkey(const uint8_t *request, size_t length);

/* 列出自定义辅助码表。resources 是绝对的 Engine 资源目录；state_root 是绝对的状态根（HostOptions 的 preferences_directory），没有时传 NULL 和 0。两处的 helpcodes/custom 合起来按主干排序，同一主干取状态根那份（用户自己的表放在状态根下，升级不会动它；资源目录是安装包或词库代次，是以前的位置），与 Engine 读表的顺序一致。value 是 {schema,file_stem,name,name_en} 的数组，目录不在或读不了时是空数组。显示名取开头的 # name: 和 # name_en: 注释，码表本身仍由 Engine 解析。 */
char *msime_client_helpcode_schemas(const uint8_t *resources, size_t length, const uint8_t *state_root, size_t state_root_length);

/* Same validation as load_preferences; ok:true,value:null means lock busy.
 * Does not wait for the writer lock. Disk I/O may still block: use a worker.
 * Busy is not missing/corrupt and must not reset preferences to defaults. */
char *msime_client_try_load_preferences(const uint8_t *directory, size_t length);

/* Repair a preferences.json that is not well-formed JSON (truncated, empty, overwritten). May block on disk and the writer lock: use a worker. Backup first: the damaged bytes are copied verbatim to <directory>/preferences.json.corrupt-YYYYMMDD-HHMMSS (UTC, -N suffix when taken) and nothing is rewritten if that copy fails. Then every setting and service key the schema still accepts is carried onto the defaults. A valid or missing document is a no-op: {recovered:false, snapshot}. A repair returns {recovered:true, snapshot, backup_path, backup_name, salvaged}. A well-formed document the schema rejects (unknown fields, newer format_version) is most likely a newer build's and returns the load error unchanged; the settings page repairs it explicitly with msime_client_repair_preferences. Storage errors are returned and never lead to a rewrite. */
char *msime_client_recover_preferences(const uint8_t *directory, size_t length);

/* The settings page's explicit 修复配置文件: the same repair for any document load_preferences rejects, a newer build's (unknown fields, newer format_version) included. Same backup, salvage, result shape and worker requirement as msime_client_recover_preferences. Call it only when the user asks: what this schema does not accept takes its default. */
char *msime_client_repair_preferences(const uint8_t *directory, size_t length);

/* Compare-and-swap save of PreferencesSnapshot.preferences. The snapshot's
 * format_version is validated; expected_revision must match the store.
 * A disabled clipboard-history save clears the default history file if history
 * is still disabled. Cleanup errors may be returned after preferences are saved.
 * Directory <=16384 bytes; snapshot <=1048576 bytes, enough for a custom skin photo. */
char *msime_client_save_preferences(const uint8_t *directory, size_t directory_length,
                                    uint64_t expected_revision,
                                    const uint8_t *snapshot, size_t snapshot_length);

/* 在会话线程传入 PreferencesSnapshot JSON（最多 1048576 字节）：{format_version:1, revision, preferences:{...}}。修订号按会话递增；相同快照可重试，旧版或冲突快照会被拒绝。返回 {revision, deferred, view, diagnostic?, missing_resource_pack?}；仅首选方案无法运行并回退时带 diagnostic。首选方案因本机缺它的词库而回退时另带 missing_resource_pack（"wubi" 或 "language-dictionaries"），宿主可经 msime_client_resource_pack_install 下载，装好后下一次焦点刷新即换回首选方案。组字期间的 Engine 偏好应用延后到派发或焦点调用使会话空闲，但关闭云候选会立即清除现有云行和旧查询，AI 配置或令牌变化也会立即清除旧 AI 行和查询；释义开关、目标语言或翻译服务变化会清掉旧释义并推进候选代次，旧释义回包将被拒绝。宿主应使用返回的新 view。较新快照替换待应用快照。构建失败时保留旧会话和待应用快照供重试，派发通过 diagnostic 报告重试错误，不丢失已完成输入。本接口不读写偏好文件，宿主提供已加载的快照。 */
char *msime_client_update_preferences(uint64_t session, const uint8_t *snapshot, size_t length);

/* 获得焦点（true）只重置旧组字，包括 phrase_preedit 暂存的词段，不返回旧组字的提交；失焦（false）按当前方案结束组字，并返回应送出的提交，包括已经选好的暂存词段。 */
char *msime_client_focus(uint64_t session, bool focused);

/* Clear the current session's Engine candidate cache and refresh its view. */
char *msime_client_reset_cache(uint64_t session);

/* Release process-level model caches without a session handle (memory pressure): the Japanese dictionary, personal context models no session uses, and the plugin tables kept for the next session. Also releases the calling thread's candidate-gloss caches, as msime_client_release_gloss_caches does. */
void msime_client_release_process_cache(void);

/* 关掉调用线程缓存的候选释义词库连接（发布词库、学到的释义库、离线释义库），丢掉读好的自定义译文；下一次查询重新打开，结果不变。不动日文词典这类重新加载很贵的模型，宿主可以在每次键盘收起时调用。只清调用线程自己的那份，要在查询释义的那个线程上调。 */
void msime_client_release_gloss_caches(void);

// Live per-session mode, not a persisted preference. Preserves composition and
// candidate generation; remains authoritative across preference replacement.
char *msime_client_set_chinese_punctuation(uint64_t session, bool enabled);

char *msime_client_set_character_width(uint64_t session, bool fullwidth);

char *msime_client_set_english_mode(uint64_t session, bool enabled);

/* 标出隐私会话（隐私模式、不允许学习的输入框）：不记录选词位置和上屏效率，并立即禁止、清除上屏后联想；宿主须重新读取 View，以丢弃旧候选代次。用户单独关掉学习不算隐私会话；学习本身仍由 learning 偏好决定。Value 是设下的布尔值。 */
char *msime_client_set_private_session(uint64_t session, bool enabled);

/* next_word_prediction 偏好开启、而且宿主声明能显示联想（HostCapabilities.next_word_prediction，目前是 Android、Linux、macOS、Windows、iOS 与 HarmonyOS）时，View.prediction=true 标记上屏后的空闲联想：editing_text、preedit 和 reading 为空；联想和普通候选一样选：点选、数字键选本页对应的一条、空格选高亮的一条（九键、字母网格和拿数字拼写的方案里数字仍是输入，越过本页的数字照常输入）。回车、标点、退格、导航和失焦不提交联想，引擎先清掉联想再照常处理；普通状态省略 prediction 字段，宿主缺省按 false 读取。 */
/* 中日英循环切换的状态机（`crates/client-core/src/language_cycle.rs`），一个键在 中 → 日 → 英 → 中 之间转圈。顺序来自旧版 Windows 正式版 `engine/contracts/trilingual_input.h` 的 `Next()`：中文 → 日文 → 英文 → 中文；参与循环的是「切换中英文」那一组快捷键，不是 Ctrl+Shift+J。
 * 参数：enabled 是 keybindings.trilingual_cycle；mode 是按下循环键那一刻的模式，0 中文、1 日文、2 英文；scheme 是实际在跑的方案，编号同 View.scheme；last_chinese_scheme 是要回去的中文方案（0/1/2/5/6/9），-1 表示还没记下；has_composition 是宿主会话里还有组字（已选词段或未上屏的拼音）。
 * Value 是 {mode, scheme, commit_raw}：mode 同上编号；scheme 是进入中文时要切到的方案，null 表示只换中英模式；commit_raw 为真时先原样上屏正在组的内容（已选词段保留，剩下没选的拼音串照原样出去），再换模式。value 为 null 表示这一键不循环（开关关着，这一组快捷键仍是二态切换），宿主照旧自己处理这个键。非法编号返回错误。
 * 有组字时上的屏是原始拼音串，不是高亮的候选：旧版在改名或缩写时按住循环键是为了保住已经打出来的字母，而不是替用户选一个他没要的词。 */
char *msime_client_language_cycle(bool enabled, uint8_t mode, uint8_t scheme, int8_t last_chinese_scheme,
                                  bool has_composition);

/* 报告大写锁定状态：偏好 caps_lock_ascii_punctuation 打开时，大写锁定期间没有组字的标点键（经 msime_client_character、msime_client_punctuation 或 msime_client_punctuation_with_context 送来）改走字面 ASCII 路线，和英文模式一样；punctuation_lock 为 chinese 时仍是中文标点。会话状态而非偏好，会话重建后由宿主重新报告。Value 是设下的布尔值。接上这个调用的宿主在 HostCapabilities.caps_lock_punctuation 里声明。 */
char *msime_client_set_caps_lock(uint64_t session, bool enabled);

char *msime_client_set_paired_punctuation(uint64_t session, bool enabled);

char *msime_client_set_punctuation_lock(uint64_t session, uint8_t lock);

/* 每页候选数的会话内覆盖，size 为 1–10；本平台排不下那么多时（Windows、iOS、鸿蒙最多 9）按平台上限截断，view.page_size 是实际值。组字中调用时等组字结束才生效。 */
char *msime_client_set_candidate_page_size(uint64_t session, uint8_t size);

char *msime_client_character(uint64_t session, uint8_t ascii, bool shift);

// Explicit native punctuation: finish the highlighted composition, then translate.
// Invalid non-punctuation bytes fail without modifying the session.
char *msime_client_punctuation(uint64_t session, uint8_t ascii);

/* Resolve native punctuation using the immediately preceding Unicode scalar
 * supplied by the platform editor. Zero means unavailable. Only the scalar is
 * inspected and no document text is retained. */
char *msime_client_punctuation_with_context(uint64_t session, uint8_t ascii,
                                            uint32_t preceding);

/* 无会话、无 I/O 的跨次空格策略。请求含 config（enabled/excluded_apps）、application、context（scheme/local_mode），可带 text、preceding Unicode 标量、secure、full_width。返回 {enabled,text}；先询问 enabled，再读取必要的前文。最多 65536 字节；结果按通常的 ok/value 信封返回，由 msime_client_string_free 释放。 */
char *msime_client_cross_commit_spacing(const uint8_t *request, size_t length);

/* Balance Engine nesting after the host emitted a paired closing mark. Only
 * the ASCII book-title opening '<' is accepted. */
char *msime_client_balance_paired_punctuation_after_auto_close(uint64_t session,
                                                               uint8_t opening);

// Finish the highlighted composition, then append the literal ASCII mark.
// Hosts use this for platform smart-punctuation decisions based on editor
// context; invalid non-punctuation bytes fail without modifying the session.
char *msime_client_punctuation_ascii(uint64_t session, uint8_t ascii);

/* View.scheme and commit_context.scheme: 0 quanpin, 1 shuangpin, 2 wubi, 3 japanese, 4 korean (preferences scheme "korean"). Korean is a Dubeolsik Hangul automaton: send every letter through msime_client_character with its case (Shift+Q/W/E/R/T/O/P type ㅃ ㅉ ㄸ ㄲ ㅆ ㅒ ㅖ); View.reading and View.preedit hold the composing Hangul to mark inline with the caret at its end, while editing_text holds the key letters of the open syllable and is non-empty exactly while composing. A transition may carry a commit together with a new composition (the previous syllable finished when a new one started) and a commit with handled=false (Space, Enter, a caret key, Delete or a digit ended the syllable): always insert the commit first, then let an unhandled key do its normal work in the application. Punctuation is always half-width ASCII and never converted to full width. With no Hanja list open there are no candidates: MSIME_BACKSPACE removes one jamo; MSIME_CANCEL discards the open syllable; msime_client_focus(false) commits it, while msime_client_focus(true) discards it so a syllable left open in one client never reaches the next.
 * Hanja: candidates appear only after MSIME_CONVERT_HANJA while a syllable is composing, so a host reads scheme 4 with a non-empty candidate list as "the Hanja list is open". The list holds the Hanja of the composing syllable (a syllable already committed is not converted), each candidate's annotation is its 훈음 when it has one, and its code is the key letters, which a host should not draw. MSIME_CONVERT_HANJA answers handled=false when the composition has no Hanja (a lone jamo) or nothing is composing; a host should then swallow its trigger key while composing rather than pass it on. Sending it again closes the list. While the list is open the candidate commands work as for any list (MSIME_COMMIT_CANDIDATE, digits 1-9 on the visible page, paging and MSIME_NEXT/PREVIOUS_CANDIDATE), and choosing commits the Hanja with handled=true; send MSIME_COMMIT_CANDIDATE for Return as well, since only the session knows the highlight. MSIME_CANCEL and MSIME_BACKSPACE only close the list and keep the syllable composing. A letter closes the list and composes as usual. Punctuation, MSIME_FINISH_COMPOSITION and msime_client_focus(false) close the list and commit the Hangul, never a Hanja, whatever is highlighted; msime_client_focus(true) still discards the syllable. */
/* View.scheme 和 commit_context.scheme 续：8 tibetan（藏文，偏好方案 "tibetan"），在拉丁字母键盘上按 EWTS（扩展威利转写）输入，不需要词库，所有宿主都提供，和越南文一样默认不启用。威利转写区分大小写，大写字母（T D N Sh A I U M H 等）是拼写：带大小写原样送进 msime_client_character，不要当成 Shift 命令。组字保存当前音节串的威利原文，View.preedit 和 editing_text 显示转换后的藏文，候选列表始终为空。View.spelling_symbols 空闲时为 "'/"，组字时为 "'+-./"，这些键要作为字符交给引擎，尤其 '/' 不能由宿主自己插入 ASCII 斜杠。组字时空格（字符 ' ' 或 MSIME_COMMIT_CANDIDATE）上屏藏文加音节点 U+0F0B，'/' 上屏藏文加垂符 U+0F0D，两者都是 handled=true；没有组字时 '/' 单独上屏垂符，空格和数字交回宿主。MSIME_COMMIT_RAW（回车）只上屏藏文、不加音节点，handled=true。MSIME_BACKSPACE 删掉一个原文按键；第一次 MSIME_CANCEL 把显示退回威利原文，第二次丢弃组字。数字、光标键、Delete、Tab 和 MSIME_COMMIT_READING 按显示上屏后 handled=false：先插入 commit，再让按键在应用里照常生效。其他标点先上屏藏文再跟半角 ASCII 标点，从不转成中文或全角标点。msime_client_focus(false) 上屏组字。数字保持原样，不转成藏文数字。 */
/* View.scheme and commit_context.scheme, continued: 5 cantonese (Jyutping), 6 zhuyin (Dachen bopomofo), 7 vietnamese (Telex or VNI), 9 stroke (笔画: five-stroke order 横竖撇点折 typed as the letters h s p n z, with x matching any one stroke), each the preferences scheme of the same name. Every host offers them, and Cantonese, Zhuyin and Stroke only with their language dictionaries installed; where a scheme cannot run, a preferences document naming it runs the last Chinese scheme or quanpin instead and msime_client_update_preferences says so in its diagnostic.
 * Stroke: only h s p n z start a composition; x and other letters return handled=false while idle, so a host inserts them as usual. While composing x appends a wildcard, other letters are swallowed with handled=true, and digits 1-9 pick candidates. View.reading and View.preedit hold the stroke glyphs 一丨丿丶乛＊, one per typed letter, which a host draws inline; editing_text and caret_position count the ASCII letters. MSIME_COMMIT_RAW commits the typed letters, MSIME_COMMIT_CANDIDATE the highlighted character (the letters when there is none), and focus loss does not commit.
 * MSIME_OPEN_CANDIDATE_LIST is MSIME_CONVERT_HANJA under the name that says what it does in every scheme: open the active scheme's candidate list. Korean opens and closes its Hanja list as described above; Zhuyin opens the candidate list of its composition; every other scheme answers handled=false. */
/* View.display_reading 是宿主画在行内的读音。日文罗马字（scheme 3，非本地模式、非英文）组字时末尾单个 n 还没定成ん，这里留作字母（kan 是 かn，kann 才是 かん），与微软、Google 日文输入法一致；其余方案和状态下与 View.reading 相同。View.reading 仍是 MSIME_COMMIT_READING 上屏的读音，末尾的 n 在那里已是ん。宿主不经引擎结束组字区时：光标被移走、编辑器仍报告着组字区，应把组字区换成 View.reading 再结束；确认不了组字区还在（编辑器报告没有组字区、会话停下）时只结束组字，不要把 View.reading 写进可能已被应用清空或改写的输入框，这时文档里可能留下画出的读音。只读 View.reading 的宿主行为不变。 */
/* View.preedit_caret 是组字光标在 View.preedit 里的位置，按 Unicode 标量计，宿主换算成自己的字符串单位。View.caret_position 是 View.editing_text（按键原文）里的字节偏移；preedit 是分词或展开过的拼写时（全拼的 ni'hao，双拼选「拼音分词」时展开的全拼 shi'shi），宿主画 preedit 就用 preedit_caret 放光标，不要拿 caret_position 去数 preedit 的字母——双拼的按键和全拼的字母数对不上，那样光标会落进音节中间。光标在按键中间时按音节换：前面有几个完整的音节，就落在第几个显示音节后面（音节边界上落在隔音符前面，紧跟用户打的隔音符时落在它后面）。两段文字没有可靠对应（韩文音节、假名、笔画字形等）时等于 preedit 的长度。宿主画 editing_text 时仍用 caret_position。 */
/* View.helpcode_trigger 只在辅助码前导键（偏好 helpcode.trigger_key）按下后、辅助码还没输满两位时出现，形如 {"key": ";", "position": 2}。前导键不进 View.editing_text，View.preedit 已经把它紧挨着插在辅助码前面（ni;，全拼 ni';F），画 preedit 的宿主不用另做什么；画 editing_text（行内「原始按键」样式）的宿主把 key 插在 editing_text 的第 position 个字节前，caret_position 不小于 position 时光标后移 key 的长度。preedit 里只插不删：拿 preedit 当拼写记东西的宿主（macOS 按它记置顶）要去掉前导键时，已输入的码有 editing_text 长度减 position 位，前导键就在 preedit 末尾这几位之前。前导键只在光标位于末尾时生效，按键原文是 ASCII，所以字节偏移就是各宿主自己的字符单位。两位码齐了、退出前导状态或别的状态下省略这个字段。 */
/* 整句改字（全拼、双拼）：桌面宿主把不带修饰键的左右方向键发成 MSIME_CONVERSION_LEFT / MSIME_CONVERSION_RIGHT（macOS 不进改字，发 MSIME_MOVE_LEFT / MSIME_MOVE_RIGHT）；Shift+左右和 Ctrl+左右在所有方案里都发 MSIME_MOVE_LEFT_SEGMENT / MSIME_MOVE_RIGHT_SEGMENT，按音节移动拼音光标。偏好 navigation.left_right 打开时，横排候选窗里不带修饰键的左右键可能被换成移动候选（高亮不在首选，或首选上光标已在末尾时按右键），分段命令从不被换，所以高亮在哪个候选上它们都编辑拼音。首选是一个字对一个完整音节、覆盖整个组字、至少两个字的句子时，MSIME_CONVERSION_LEFT 进入改字：View.conversion 是整句，光标在 View.conversion_focus_start 个字（Unicode 标量）之前，conversion_focus_start..conversion_focus_end 是候选所替换的那一段，候选换成这一段（先是光标所在的词，再是更短、更长的）的字和词。之后左右键每次移一个字，最远到句末（focus_start 等于字数，没有候选）。选中候选（数字、MSIME_COMMIT_CANDIDATE、msime_client_select）把这段钉住，句子其余部分围绕它重新转换，光标移到这段后面，不上屏；光标在句末时 MSIME_COMMIT_CANDIDATE 上屏整句。MSIME_COMMIT_RAW、MSIME_FINISH_COMPOSITION、标点和失去焦点上屏改好的整句，不是拼音字母。MSIME_CANCEL 和 MSIME_BACKSPACE 只退出改字回到拼音（组字保留，钉住的段丢掉）；键入字母或撇号、Shift/Ctrl+左右、Home/End 先退出改字再照常处理。进不了改字时（读音不完整、单字、光标在拼音中间、其他方案、本地模式）这两条命令就是 MSIME_MOVE_LEFT / MSIME_MOVE_RIGHT。改字期间宿主把 View.phrase_prefix 加 View.conversion 画在行内代替拼音，不管预编辑样式选的是什么；改字的候选不能置顶、删除或固定位置。 */
/* 移动光标后编辑光标后的部分（偏好 caret_edit_suffix，默认关，只在三个桌面宿主上生效）：全拼、双拼的组字光标停在拼音中间（MSIME_MOVE_LEFT 等字母光标命令、分段命令）时，光标前那段原文固定为它当前的最佳转换，候选换成光标后那段的候选。此时 View.caret_edit_suffix 为 true，View.caret_edit_prefix 是固定的那段文字，首选（id.index 为 0）的 text 已经以它开头，是整句；其余候选是光标后那段开头的词。宿主在候选窗右侧显示「候选编辑：光标后」，能分段画候选的把首选开头 caret_edit_prefix 的那几个字（按 Unicode 标量数）画淡。选中一条只消耗它自己的音节：固定的那段和这一条进 View.phrase_prefix 暂存（需要 phrase_preedit），剩下的拼写接着在本模式里转换，整段完成时一起上屏。光标在开头或末尾、其他方案、本地模式和专用英文里两项都不出现，行为与关着时相同。模式里不混入英文、emoji、符号、日文、快捷短语等行，也不发云候选和 AI 查询。 */
enum MsimeCommand {
    MSIME_BACKSPACE = 0, MSIME_COMMIT_CANDIDATE = 1, MSIME_COMMIT_RAW = 2,
    MSIME_CANCEL = 3, MSIME_MOVE_LEFT = 4, MSIME_MOVE_RIGHT = 5,
    MSIME_MOVE_HOME = 6, MSIME_MOVE_END = 7, MSIME_DELETE_FORWARD = 8,
    MSIME_FINISH_COMPOSITION = 9,
    MSIME_CYCLE_KANA_VARIANT = 10, MSIME_COMMIT_READING = 11,
    MSIME_BACKSPACE_SEGMENT = 12, MSIME_MOVE_LEFT_SEGMENT = 13, MSIME_MOVE_RIGHT_SEGMENT = 14,
    MSIME_COMMIT_RAW_WITHOUT_LEARNING = 15,
    MSIME_CONVERT_HANJA = 16,
    MSIME_OPEN_CANDIDATE_LIST = 16,
    MSIME_CONVERSION_LEFT = 17, MSIME_CONVERSION_RIGHT = 18,
    /* 宿主已经自己把文本写进了文档，再丢弃组字：对会话的作用同 MSIME_CANCEL（一次，组字没结束时照旧再发一次），区别只在生命周期里结束原因记 HostCommit、不出取消音。 */
    MSIME_CANCEL_AFTER_HOST_COMMIT = 19,
    /* 焦点即将离开或会话即将关闭，宿主替用户结束组字（macOS IMK 的 commitComposition:，宿主为词库维护关掉会话之前）：对会话的作用同 MSIME_FINISH_COMPOSITION（只有一处不同：高亮的是脚本插件在 / 指令里给的行，即来源 14 时，上屏用户打的原文，例如 /uuid，因为这时用户没在看候选窗），生命周期里上屏来源记 FocusLoss，同 msime_client_focus(false) 的上屏：不可改写、不出提交音、不画上屏特效，回复里 feedback 的 typing_effect 为 0。用户按键或点选引起的结束组字仍发 MSIME_FINISH_COMPOSITION。 */
    MSIME_FINISH_ON_FOCUS_LOSS = 20,
    MSIME_NEXT_PAGE = 100, MSIME_PREVIOUS_PAGE = 101,
    MSIME_NEXT_CANDIDATE = 102, MSIME_PREVIOUS_CANDIDATE = 103,
    MSIME_FIRST_CANDIDATE = 104, MSIME_LAST_CANDIDATE = 105,
    /* 候选窗显示时的 Home 键：高亮不在首选时同 MSIME_FIRST_CANDIDATE；已经在首选、输入串非空、没有打开的候选列表（韩文汉字、注音）且不在整句改字时同 MSIME_MOVE_HOME；整句改字时一直同 MSIME_FIRST_CANDIDATE，不退出改字。宿主只在 Home 键上发它；需要「无条件回到首选」的地方（日文转换循环候选）仍发 MSIME_FIRST_CANDIDATE。 */
    MSIME_FIRST_CANDIDATE_OR_HOME = 106
};

char *msime_client_command(uint64_t session, uint32_t command);

/* Re-rank the visible candidates with the settled model, once the host's typing pause elapses.
 * The host owns the clock: it is the only side that knows whether a keystroke arrived while the
 * pass was being decided. Answers {"moved": false} when the order did not change, or
 * {"moved": true, "view": ...} after a reorder. The false case lets the host leave the candidate
 * window alone without serializing a view it will discard. Inert, and immediately false, when no
 * settled model is installed. */
char *msime_client_rerank_settled(uint64_t session);

/* 宿主等待输入停顿后，在后台启动模型评分。无需改动或只做呈现整理时返回 {"status":"done","moved":false,"current":true}；呈现整理改变顺序时返回 {"status":"done","moved":true,"current":true,"view":...}。后台评分返回 {"status":"pending","token":N}，经 msime_client_poll_settled_rerank 轮询；每会话只运行一个任务，重复启动返回相同令牌，评分期间可以继续处理按键。宿主在会话选项里写了 "pause_rerank": true、而这台设备跑不进逐键预算时，整句联想「增强」的模型延后到这里：当场在会话线程上带着它重解当前组字的一片（约 8 ms 的模型时间），解完返回 "done"，没解完返回 {"status":"continue","moved":false,"current":true}，宿主先处理排队的输入，没有新按键就再调一次；每片若推进了候选代次，就附带新的 view，即使 moved 为假；宿主收到 view 必须更新候选身份。 */
char *msime_client_start_settled_rerank(uint64_t session);

/* token 来自启动接口；评分未完成时返回 {"status":"pending","token":N}，
 * 完成后返回 {"status":"done","moved":...,"current":...}。
 * current 为真表示当前组字及配置已评分；按键、开关或模型改变，或 accept=false 时为假。
 * 未知令牌返回 {"status":"done","moved":false,"current":false}。
 * 宿主已离开原组字时传 accept=false，只归还仍适用的模型。必须在会话线程调用。 */
char *msime_client_poll_settled_rerank(uint64_t session, uint64_t token, bool accept);

/* Pass the generation and global index from the displayed candidate's id. */
char *msime_client_select(uint64_t session, uint64_t generation, size_t index);

/* Select any entry copied by msime_client_all_candidates for this exact
 * generation. Normal select remains restricted to the current View page. */
char *msime_client_select_any_candidate(uint64_t session, uint64_t generation, size_t index);

char *msime_client_pin_candidate(uint64_t session, uint64_t generation, size_t index);

char *msime_client_remove_candidate(uint64_t session, uint64_t generation, size_t index);

/* Fix a dictionary candidate to slot 1..5 for the current input context. */
char *msime_client_fix_candidate_position(uint64_t session, uint64_t generation, size_t index,
                                          uint8_t position);

/* Clear a previously fixed dictionary candidate position. */
char *msime_client_clear_candidate_position(uint64_t session, uint64_t generation, size_t index);

enum MsimeCandidateEdge { MSIME_FIRST_HAN = 0, MSIME_LAST_HAN = 1 };

/* Engine selects one Han character and clears composition on success.
 * A candidate without Han text is unhandled and keeps composition; no fallback
 * punctuation or candidate commit is manufactured. Uses the same current-page
 * identity checks as select. Invalid edge values fail before state changes.
 */
char *msime_client_select_edge(uint64_t session, uint64_t generation, size_t index, uint8_t edge);

/* On-demand {session,generation,preedit,candidates:[Candidate...]}. Unlike View,
 * candidates contains the complete cached Engine generation with global IDs. */
char *msime_client_all_candidates(uint64_t session);

/* {session,generation,preedit,reading,total,candidates:[Candidate...]}：座位 start 起最多 count 行，到 total（这一代已放出的候选数）为止，身份与 msime_client_all_candidates 同一座位的那一行相同，可交给 msime_client_select_any_candidate。每键重画的横向候选条只取它这次建得出来的一批，不必整代复制。引擎暂扣的尾部（首字母只先给 24 行）只在 start 到了 total 时才放出；放出来会重排座位并换一代，宿主看到 generation 变了要按新的一代从头取。同一代里 candidates 为空表示已经没有更多。 */
char *msime_client_candidate_range(uint64_t session, size_t start, size_t count);

/* 宿主按自己的排版显示了与固定分页不同的一组候选（macOS 横排按宽度自适应）时，报出这一代里这些候选的全局序号
 * （来自 all_candidates 的 id.index，<=256 个）。二三候选键、五笔自定义选词键和翻译查询都按看得见的这一组；
 * count 为 0 时清掉。只对 generation 这一代有效，组字推进后自动失效。
 * Value {"applied": bool}：代次过期或序号越界时为 false，原有显示页不变。 */
char *msime_client_set_display_page(uint64_t session, uint64_t generation, const size_t *indices, size_t count);

/* Return read-only English completions for a bounded ASCII prefix. */
char *msime_client_english_completions(uint64_t session, const uint8_t *prefix,
                                       size_t prefix_length, size_t limit);

/* View.local_mode 是 Engine 自己的模式，不是从预编辑前缀猜出来的；View.microsoft_shuangpin 报告 Engine 已应用的配置，不是尚未生效的新偏好；它为真表示当前是双拼且韵母或零声母编码用 `;` 作第二键，宿主要把 `;` 当字母键交给 msime_client_character。宿主结合模式、编辑文本和光标使用它。取值为 none、unicode、date_time、quick_phrase、emoji、kaomoji、super_jianpin、temporary_english、temporary_japanese、expression、command、mention、url、chaizi（拆字反查，Shift+C 进入；候选注释是字的带调读音，由 Engine 给出）、symbol（符号模式，Shift+S 进入；输入分组码 xl / sx / szq / jt 或符号名 alpha、infinity），未知取值视为不可用状态。
 * View.spelling_symbols 列出 Engine 在当前状态下当作输入的非字母键：当前模式的拼写（expression 的数字和运算符、unicode 的数字、url 的数字和网址符号）；没有组字时打开模式的键（`/` 和 `@`）；全拼、双拼、五笔组字中原文恰好是 www、http、https、ftp 时打开网址模式的键（`.` 或 `:`）。宿主要把它们当字符发送；列在里面的数字是输入，不是选候选的快捷键。转换结果的 commit_context.typing_statistics 对 expression、command、mention 模式生成的文本为 false，这些文本不计入打字统计。
 * 中文标点下 '\'' 与 '"' 出哪种引号、'[' ']' 出哪种方头括号由偏好 quote_style、bracket_style 决定（契约 shared/contracts/punctuation 的 styles 段）。View.punctuation_marks 报告引擎已应用的样式：{"quotes": "curly"|"corner", "brackets": "solid"|"hollow", "enumeration_comma_backslash": bool, "enumeration_comma_slash": bool}，corner_when_traditional 已按繁体输出开关解析成 curly 或 corner；两个顿号字段报告契约 enumerationComma 段里 '\\' 和 '/' 哪个键出顿号（至少一个为真）。宿主做成对补全、跨过后半个、空格转换、英文模式下的中文标点和画触屏键面时读它，不自己解析偏好；改了偏好要等空闲重建后它才变，和引擎出字同步。
 */
char *msime_client_view(uint64_t session);

/* 共享解析器处理宿主取得的云响应，返回 {applied,view}；空或无效响应不改变组字。
 * 查询最多 128 KiB，响应最多 256 KiB。过期、关闭或切换来源的查询不应用候选。 */
char *msime_client_apply_cloud_response(uint64_t session,
                                      const uint8_t *query, size_t query_length,
                                      const uint8_t *body, size_t body_length);

/* Apply a UTF-8 cloud (source=0) or AI (source=1) result for a copied query. */
char *msime_client_apply_online_candidate(uint64_t session,
                                           const uint8_t *query,
                                           size_t query_length,
                                           const uint8_t *candidate,
                                           size_t candidate_length,
                                           uint8_t source);

/* Apply a JSON array of UTF-8 strings for one source (cloud=0, AI=1).
 * Buffers are borrowed for the call; at most 16384 bytes each. */
char *msime_client_apply_online_candidates(uint64_t session,
                                          const uint8_t *query,
                                          size_t query_length,
                                          const uint8_t *candidates,
                                          size_t candidates_length,
                                          uint8_t source);

/* Remove cached and visible rows for one source (source=0 cloud, 1 AI). */
char *msime_client_clear_online_candidates(uint64_t session, uint8_t source);

/* AI 回答为空、失败，或 apply_online_candidates 返回 applied:false 时宿主会调用它（source 只认 0 和 1，其余报错）。共享内核不再放 AI 占位行（候选 source 12），AI 回答到达时第一条直接插在第一页最后一格；这个入口为已发布的宿主留着，合法的调用一律返回 {applied:false, view}，不改候选、不换代。 */
char *msime_client_resolve_online_candidates(uint64_t session,
                                             const uint8_t *query,
                                             size_t query_length,
                                             uint8_t source);

/* Resolve a shared surface route ("settings", "settings:voice", "emoji", ...)
 * so a host launches the shared shell by name. Returns the canonical route and,
 * for panel surfaces, the window label, query and geometry. */
char *msime_client_parse_surface_route(const uint8_t *value, size_t length);

/* Convert UTF-8 Simplified Chinese to Traditional Chinese with the shared
 * OpenCC s2t tables (phrase-level, so 头发 -> 頭髮 and 发展 -> 發展). Unlike
 * the JSON calls, returns the converted NUL-terminated text directly because
 * hosts call it per candidate. Returns NULL for NULL, inputs over 1 MiB,
 * invalid UTF-8 or an embedded NUL; keep the original text then. Free with
 * string_free. The first call parses the tables (a few milliseconds in
 * release builds). */
char *msime_client_simplified_to_traditional(const uint8_t *text, size_t length);

/* 同上，但按 variant 选 OpenCC 配置：0 s2t、1 s2tw（臺灣字形）、2 s2twp
 * （臺灣字形加惯用词），与偏好 traditional_chinese_variant 的 "s2t"、"s2tw"、
 * "s2twp" 一一对应。未知的 variant 与其它非法输入一样返回 NULL。臺灣配置的
 * 附加词表在第一次用到时解析。Free with string_free. */
char *msime_client_convert_to_traditional(const uint8_t *text, size_t length,
                                          uint32_t variant);

/* Display-only font aliases. Input: JSON array, at most 33 names / 32 KiB.
 * Returns the standard response with an array value. Free with string_free.
 * Resolution failure retains the corresponding original family name. */
char *msime_client_resolve_font_families(const uint8_t *value, size_t length);

/* Installed font family names, the catalog the shared app's 刷新字体列表 reads.
 * Returns the standard response with a sorted array of distinct names, each
 * non-empty, at most 128 UTF-8 bytes and free of control characters; error
 * "unsupported" on a host without a catalog, "font_catalog" or
 * "font_catalog_limit" (over 16384 families) when enumeration fails.
 * Enumerates the system's fonts: call it off the UI thread. Free with
 * string_free. */
char *msime_client_list_font_families(void);

/* Describe what the named host ("windows"/"macos"/"linux"/"android"/"ios") can
 * do, so the shared UI renders from capabilities rather than the user agent. */
char *msime_client_host_capabilities(const uint8_t *platform, size_t length);

/* Where this library reports a failure it recovers from by itself, such as a sound or music pack that does not load, an audio device that does not open, or a helpcode pack the Engine falls back from. Without a sink each report is one line on stderr prefixed "msime: "; with one, sink receives the same line without the prefix, as NUL-terminated UTF-8 valid only during the call. Each line is "<category>: <detail>": the category is a fixed English phrase saying what failed, such as "sound pack not loaded", and holds no colon, so the text before the first colon names no pack, file or cause; the detail names packs, files and error causes. Neither holds typed text, candidates or credentials; a host whose log must not keep the detail keeps only the category. Pass NULL to go back to stderr; a later call replaces the sink. A report already under way on another thread may still call the sink it read after the call that replaced or cleared it has returned, so every sink ever registered must stay callable for the rest of the process: do not unload or free its code. sink may be called on any thread, including the audio player's own and several at once, and from inside other msime_client calls: it must not block, must not call any msime_client function, and must return normally. Register it once at startup, before the first session. */
typedef void (*msime_client_diagnostic_sink)(const char *line);

void msime_client_set_diagnostic_sink(msime_client_diagnostic_sink sink);

/* 生命周期核心（桌面宿主选择加入；移动端宿主不调用，行为和 transition JSON 都不变）。以下调用都在会话所在线程上进行，不落盘，不分配要释放的内存；未知会话、错线程、重入调用或内部 panic 时返回 0 或 false。
 * msime_client_set_lifecycle：让会话选择加入，或换一组位；返回实际启用的位，其余位被忽略。没调用过它的会话不推导任何事件。
 *   MSIME_LIFECYCLE_FEEDBACK：按键音、上屏音、打字特效和背景音乐改由核心驱动。按键音和按键特效来自 msime_client_key；上屏音和上屏特效（typing_effect 事件 4）来自核心的上屏事件，派发类调用的回复里多一个 "feedback":{"typing_effect":N} 键，N 的位布局同 msime_client_typing_effect，没有上屏时为 0；音乐由 msime_client_set_active 仲裁所有者。本会话的 msime_client_key_sound、msime_client_commit_sound、msime_client_typing_effect（事件 0-4）和 msime_client_music_set_active 随之变为空操作（返回 false 或 0）。typing_effect 事件 5（绕过按键音的删除）不经过 msime_client_key，核心看不到，仍由宿主调用。失焦上屏（msime_client_focus(false) 和 MSIME_FINISH_ON_FOCUS_LOSS）不出声也不画。字段属性带 SECURE 时没有任何声音和特效，音乐关闭；带 FULLSCREEN 时声音静音、特效照画（同 typing_effect 的位 9）。
 *   MSIME_LIFECYCLE_COMMIT_OBSERVE_ONLY：之后的上屏只能观察、不能改写，可随时切换。TIP 已经从自己的会话把文字写进文档的路径（Windows），Server 在派发前设上、派发后清掉。设着它时脚本插件的候选变换也停下：派发里不运行 candidates，旁路调用也不重放之前的重排、隐藏和注释，列表保持引擎和内置排序的顺序。TIP 在自己的会话里选词的列表（注音等 AlwaysInlinePreedit 方案）按那个会话的顺序选，Server 一侧重排或隐藏就会让显示的顺序和数字选中的对不上，所以这类列表的每一次派发都要设着它。
 *   MSIME_LIFECYCLE_HOST_HOLDS_PIECES：宿主自己暂存部分选词（Windows），补齐整个词时才把各段拼好写进文档。这样的会话里每次部分选词都是一次上屏，commit 事件按段投递，插件读到的 commit 只是这一段；这个词的各段，包括补齐它的那一段，before_commit 都不可改写。部分选词上屏的那一段还在宿主手里、没进文档，FEEDBACK 不为它出上屏音、不画上屏特效（transition 的 feedback 为 0），整段送出的那次上屏再出；生命周期事件照常发。
 * 顺序契约。轮次：宿主每把一个物理按下交给输入法，都要在为这个键调用任何派发之前先调一次 msime_client_key，它开启新的一轮；到下一次 msime_client_key 之前的所有派发和 focus 调用都属于这一轮，同一个键可能触发多次派发（如 Windows 对同一个键再发一次 MSIME_CANCEL）。指针点选和翻页先调 msime_client_key(session, MSIME_KEY_POINTER)。带 MSIME_KEY_PASSTHROUGH 的键（Windows 交给应用、由 TIP 经 Aux 管道异步上报的键）不开新的一轮，也不承诺先于应用处理。所以"打字前"只承诺先于本轮的第一次派发。
 *   过渡期的例外：msime_client_key 还没有"开一轮但不出声"的位，所以 Linux 的 IBus 和 Fcitx5 宿主只为会出按键音的打字键调用它。松开、单独的修饰键、带 Ctrl/Alt/Super 的组合键、录音中的键和 IBus 屏幕键盘面板送来的键都不开新的一轮，它们引起的派发（如组字中按 Ctrl 组合键发出的 MSIME_FINISH_COMPOSITION 或 MSIME_CANCEL、单按 Shift 切换中英文时的 MSIME_COMMIT_RAW）算在上一轮里。第 3 阶段按轮计 fuel 的脚本运行时不会因为这类没有先调 msime_client_key 的派发熔断；补上这个位之后，宿主再为这些按下也调用它，让它们各自成一轮。按住不放产生的自动重复不出声、不画（与 macOS 宿主一样），但每次重复都会派发，所以两个 Linux 宿主为它借 msime_client_key(session, MSIME_KEY_POINTER) 开新的一轮；macOS 宿主目前不为自动重复调用 msime_client_key，自动重复引起的派发算在第一次按下那一轮里。
 * 同一次调用内的事件顺序：input（派发前）→ selection_retracted → candidate_selected → before_commit → commit → composition_end 或 composition_start → composition_update → candidates_shown 或 candidates_hidden，每次调用最多一个候选事件。focus 调用没有 input，focus_in/focus_out 排在它自己的上屏和组字结束之后；失焦上屏一律不可改写。destroy 先发 composition_end{Destroyed}，最后是会话结束。组字状态只在调用边界前后各采样一次。一次 transition 同时带着上屏和新组字时（韩文音节延续）只发 commit{continues}，不发 end 和 start。apply_online_*、resolve_online_candidates、clear_online_candidates、apply_cloud_response、rerank_settled、apply_translations 这些旁路调用，以及 set_english_mode、set_chinese_punctuation、set_punctuation_lock、update_preferences、set_ai_credential 这些可能在组字中途改动组字或候选的设置调用，只给核心发视图类事件（composition_end{Other}、candidates_shown/hidden 等）。派发、focus 和上面这些调用报错时照样推导事件：runtime 可能在报错前已经结束或改动了组字，start 和 end 始终成对。事件不带文本，也不写进任何逐键记录。
 * Windows TIP 会话的约束：TIP 在应用进程里自己的会话只给 Server 转述，不调用以下任何一个函数；字段属性和激活由 Server 的会话报告。
 * msime_client_key：key 的位 0-6 是按键类别 0-3（同 key_sound），位 7（MSIME_KEY_POINTER）是指针调用，类别被忽略，不出声不画；位 8（MSIME_KEY_REPEAT）是自动重复，画但不计连击；位 10（MSIME_KEY_PASSTHROUGH）见上。其余位忽略，类别超出 0-3 时什么都不做。返回值是 FEEDBACK 下这个键的打字特效，位布局同 msime_client_typing_effect；没开 FEEDBACK 时为 0。
 * msime_client_set_field_traits：宿主在每个键之前比较一次字段属性，变了就先调它，再调 msime_client_key。没有 MSIME_TRAIT_KNOWN 位时属性按未知处理：上屏不可改写；但 SECURE 和 FULLSCREEN 位本身仍然生效，声音、特效和音乐照样按上面的规则关闭或静音。返回现在生效的属性（不认识的位被丢掉）；会话没选择加入时返回 0。
 * msime_client_set_active：输入法在这个会话里激活（true）或停用（false）。FEEDBACK 下，true 让调用者成为背景音乐的所有者，只有所有者能开关音乐，所有者被销毁时音乐自动关闭；实际的音乐状态是"是所有者、处于激活、字段既不是 SECURE 也不是 FULLSCREEN"。会话已选择加入时返回 true。 */
#define MSIME_LIFECYCLE_FEEDBACK 0x1u
#define MSIME_LIFECYCLE_COMMIT_OBSERVE_ONLY 0x4u
#define MSIME_LIFECYCLE_HOST_HOLDS_PIECES 0x8u
#define MSIME_KEY_POINTER 0x80u
#define MSIME_KEY_REPEAT 0x100u
#define MSIME_KEY_PASSTHROUGH 0x400u
#define MSIME_TRAIT_SECURE 0x01u
#define MSIME_TRAIT_PRIVATE 0x02u
#define MSIME_TRAIT_NO_SPELLCHECK 0x04u
#define MSIME_TRAIT_FULLSCREEN 0x08u
#define MSIME_TRAIT_VOICE_ACTIVE 0x10u
#define MSIME_TRAIT_HOST_ENGLISH 0x20u
#define MSIME_TRAIT_ELEVATED 0x40u
#define MSIME_TRAIT_KNOWN 0x80000000u
uint32_t msime_client_set_lifecycle(uint64_t session, uint32_t flags);

uint32_t msime_client_key(uint64_t session, uint32_t key);

uint32_t msime_client_set_field_traits(uint64_t session, uint32_t traits);

bool msime_client_set_active(uint64_t session, bool active);

/* 脚本插件（桌面）。msime_client_scripts_enable 是进程级开关，返回开关的实际状态；任何线程都可以调用，不分配要释放的内存。只由 4 个桌面入口在启动时、第一个会话之前调用 msime_client_scripts_enable(true)：macOS 输入法进程、Windows Server、IBus 引擎进程和 Fcitx5 addon。Windows TIP（与 Server 共用同一份库，被注入每个应用进程）、设置应用和 MCP 从不调用，所以解释器的代码在它们的进程里，但从不执行。iOS、Android 和 HarmonyOS 上恒返回 false。
 * 授权文件 <state root>/plugins/script-grants.json 在用户可写的目录下，不签名：同一用户下的进程能伪造授权。三个桌面平台的开发构建和 release 构建都运行脚本，这一残余风险写在授权页上：macOS 和 Linux 是共享设置页，Windows 是原生设置应用（经 msime_client_plugins 的 script_page 和三项授权操作）；Windows 的提权窗口（MSIME_TRAIT_ELEVATED）里从不运行脚本。
 * 脚本运行要同时满足：开关开着；会话经 msime_client_set_lifecycle 选择加入；字段属性带 MSIME_TRAIT_KNOWN，且不带 SECURE、PRIVATE、ELEVATED。属性变得不满足时正在跑的实例当场丢弃。每次 msime_client_focus(true) 之后属性都要重新确认：宿主在聚焦之前调过 msime_client_set_field_traits（属性没变也算），或者聚焦之后按了第一个不带 MSIME_KEY_PASSTHROUGH 的键（msime_client_key；交给应用的键在 Windows 上经 Aux 管道异步上报，和属性没有先后保证，不算确认）；确认之前不实例化、不投递，所以想让脚本收到 focus_in 的宿主要在 msime_client_focus(true) 之前报告属性。观察类事件（focus_in、focus_out、input、selection_retracted、candidate_selected、commit、composition_start/update/end、candidates_shown/hidden）来自派发类调用、focus 和 destroy；msime_client_key 的按键、字段属性、激活和旁路调用的视图事件不投递。修改类钩子只在派发类调用里运行：candidates 在候选刷新和展开之后，插件可以重排、隐藏、注释候选，视图和选择 ID 按重排之后的顺序，被隐藏的行不出现在视图、分页和 msime_client_all_candidates 里、任何选择都选不到，插件的注释放在候选的 "script_annotation" 键里（没有时不出现，不改 "annotation"，由宿主另行绘制）；设着 MSIME_LIFECYCLE_COMMIT_OBSERVE_ONLY 时 candidates 不运行也不重放；before_commit 在上屏之前，可改写时（选中候选、原样上屏或暂存段送出，本地模式 none 且算打字，不是以词定字，没设 MSIME_LIFECYCLE_COMMIT_OBSERVE_ONLY，不是 MSIME_LIFECYCLE_HOST_HOLDS_PIECES 下宿主暂存的一段）插件可以改写标点和顶字尾巴之前的那一段，回复里的 commit 就是改写之后的文本，引擎学习和 AI 上文仍按原文。旁路调用不运行脚本，只按候选身份重放最近一次的重排、隐藏和注释。command 钩子给插件自己的 / 指令出候选：选择加入时和每次 msime_client_focus(true) 时，会话把启用、授予了 provide_commands 的插件在清单里声明的触发词交给引擎（用户指令表赢、插件按执行顺序先到先得）；用户在 / 指令里打完某个触发词时，派发类调用只投递给第一个声明它的插件一次，事件带触发词、后面的字母和 64 字节系统随机数，插件给的行是来源 14 的候选（"script_annotation" 是「标题 注释」），由引擎按预编辑串缓存，旁路调用、移动高亮和翻页都不再投递；运行条件不满足或设着 MSIME_LIFECYCLE_COMMIT_OBSERVE_ONLY 时只有原样的那一行。脚本只能经授权的 import 在钩子调用期间读文本，读不到宿主的任何别的状态，也不能回调宿主。每次 msime_client_focus（两个方向）和 destroy 都丢弃这个会话的全部脚本实例。fuel 按 msime_client_key 开启的轮次给：没有先调 msime_client_key 的派发（修饰键组合、快捷键、失焦命令）和上一次按键共用那一轮的配额；本轮配额用完时插件跳过这次投递，不算失败；运行到一半被本轮配额截断的插件在这次聚焦里不再运行，这一次用掉的不到单个钩子自己上限的一半时同样不算失败，达到一半就和超出单个钩子自己的上限一样算一次失败（candidates 的单次上限和每轮配额相等，同一轮里 input 先花过 fuel 时它的死循环先撞上每轮配额，这样也照样熔断）。脚本的失败不影响上屏：一次失败只丢弃它这次的结果，trap 之后这个插件在这次聚焦里不再运行，连续失败按插件熔断；被停用的插件和停用的类别（invalid、init、failed、slow，不带次数、具体原因和任何文本）在失焦时写进 <user_data>/script-health.json。 */
bool msime_client_scripts_enable(bool on);

char *msime_client_destroy(uint64_t session);

/* Write the selection counts held by every session on the calling thread and all queued personal-context learning, without ending any session. Call from the host's will-terminate hook (e.g. NSApplicationWillTerminateNotification) on the thread that owns the sessions; the C++ Engine did this from atexit. Returns null on success. */
char *msime_client_flush_all(void);

/* value must be NULL or a still-owned pointer returned by this library. */
void msime_client_string_free(char *value);

#ifdef __cplusplus

}
#endif
#endif
