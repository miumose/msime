// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! The input engine: composition state, input schemes, dictionary queries, ranking and learning.
//!
//! This crate replaces the C++ MSIME-Engine that used to be fetched from `engine-lock.json` and patched by overlay scripts. It owns the input algorithms; `msime-input-runtime` keeps orchestrating hosts and does not duplicate any of it.
//!
//! The documented API is the core layer at the crate root: `Session`, its options and snapshot, and the functions and types beside them (they mirror the C++ `metasequoia::` API the golden fixtures were recorded against), plus the public modules such as `assets`, `diagnostics`, `touch` and `multilingual`. `host`, `ordering`, `backend` and `web` are msime's own product glue (the session facade and settings-page helpers its hosts call, its candidate ordering, its msime-cloud protocol, its browser storage): they stay `pub` only so msime's crates in the same repository can reach them, are hidden from the documentation, are not part of the library API and change with msime's hosts; other programs should not use them. msime's product crates import everything the core layer takes or returns from the root, like a third party would; the boundary is described in `docs/ARCHITECTURE.md` and `.agents/notes/implemented/architecture/2026-10-29-engine-host-dogfood.md`. The crate's README says which dictionary files a session reads, what to set in a downstream workspace, and the licences of the data compiled in.
//!
//! 引擎在运行时读写的每个文件（词库、模型、学习日志、代次目录）的格式、版本检查、产出方与数据许可证见仓库里的 `docs/data-formats.md`；代码注释里的 `data-formats.md §N` 指它的章节。

// 非测试代码的日志一律走 `logging::warn`，嵌入方才换得掉出口（`set_log_sink`）；测试和基准测量照旧可以直接打印。
#![cfg_attr(not(test), deny(clippy::print_stderr))]

pub mod access;
pub mod assets;
mod cache;
pub mod diagnostics;
mod dictionary;
mod error;
pub mod format;
mod helpcode;
#[doc(hidden)]
pub mod host;
mod ime;
mod lattice;
mod local;
pub mod logging;
mod offensive_placement;
#[doc(hidden)]
pub mod ordering;
mod paths;
mod pinyin;
mod punctuation;
mod quanpin;
mod session;
mod shared_string;
mod shuangpin;
mod text;
pub mod time;
pub mod touch;
mod types;
mod user_dictionary;

pub use access::DictionaryAccess;
pub use error::{EngineError, ErrorCode, Result};
pub use logging::{set_log_sink, LogSink};
// 按需下载的 `.gram` 语法模型由宿主侧资源包管理，删除包时需要释放
// `GramDb::shared` 缓存的内存映射，经这个窄出口暴露给 host-api。
pub use lattice::octagram;
// 不拿来拼整句的粗俗词表：dict-builder 用同一个解析器读 msime-dictionary 的 `curation/offensive-words.txt`，用同一套查法出清洗报告。
pub use lattice::offensive::{
    parse_offensive_words, OffensiveEntry, OffensiveWords, EMBEDDED_OFFENSIVE_WORDS,
};
pub use paths::RuntimePaths;
pub use pinyin::glide::{GlideKeyboard, GlidePoint};
pub use punctuation::{BracketMarks, PunctuationMarks, QuoteMarks};
pub use session::{Clock, HelpcodeTrigger, Session, SessionOptions, SessionSnapshot};
pub use types::{
    autocorrect_type, fuzzy_rule, request_autocorrect_mask, CandidateEdge, CandidateSource,
    Command, CommandTableEntry, CommandTranslationQuery, EnglishInputOptions, FirstPageRows,
    FrequencyAdjustmentMode, FrequencyAdjustmentOptions, FuzzyPinyinOptions, KeyResult,
    LocalInputMode, LocalModeOptions, MentionEntry, MixedExpressiveOptions,
    OnlineInputMode, OnlineQuery, PersonalDictionaryEntry,
    PersonalDictionaryKind, PluginCommand, PluginCommandQuery, QuickPhraseEntry, SchemeSet,
    SchemeType, SentenceAssociationOptions, ShuangpinCustomTable, ShuangpinHelpcodeModes,
    ShuangpinProfileKind, SyllableHelpcode, WordItem,
    DEFAULT_FIRST_PAGE_ROWS,
};
// 辅助码表：`SessionOptions::helpcode_table` 与 `Session::set_helpcode_table` 收的就是它，宿主自己读表（辅助码表插件、没有文件系统的网页）时用这几项。
pub use helpcode::{load_helpcode_keymap, load_helpcode_keymap_in, HelpcodeKeymap, SharedKeymap};
// 用户自定义的 26 键双拼表（`SessionOptions::shuangpin_custom_profile`）的校验：宿主在把表交给会话之前先校验，不合法就不要传。设置页编辑表用的辅助函数（内置表作起点、不留存的校验）是产品胶水，在 `host` 里。
pub use shuangpin::custom::{
    validate_custom_profile as validate_shuangpin_custom_profile,
    CustomProfileError as ShuangpinCustomProfileError, CustomTablePart as ShuangpinCustomTablePart,
};
pub use user_dictionary::bundled::{
    dictionary_table_entries, edit_bundled_dictionary_entry, DictionaryTableEntry,
    DictionaryTablePage,
};
pub use user_dictionary::generation::{prepare_runtime_paths, prepare_runtime_paths_for};
pub use user_dictionary::journal::quarantine_corrupt_journal;
pub use user_dictionary::personal::{
    edit_personal_dictionary, personal_dictionary_entries, validate_personal_dictionary_entry,
    PersonalDictionaryPage,
};
pub use user_dictionary::replay::{replay as replay_user_dictionary, ReplayResult};
pub use user_dictionary::reset::{reset_learned_data, reset_learned_data_for};
pub use user_dictionary::state::{
    dictionary_state_revision, stage_dictionary_state, stream_dictionary_state,
    DictionaryStateRecord, DEFAULT_MAXIMUM_RECORDS,
};

pub use local::command::{valid_plugin_argument, EXTENDED_ARGUMENT_SYMBOLS};
pub use local::date_time::LocalDateTime;
pub use local::quick_phrase::{expand_quick_phrase, QuickPhraseExpansion};
pub use local::url;

/// Create or migrate an `msime-english.db` to the schema the engine reads. Fixtures without one need it; `prepare_runtime_paths` copies both dictionaries.
pub fn ensure_english_schema(path: &std::path::Path) -> Result<()> {
    dictionary::english::ensure_english_schema(path)
}

/// Write every queued personal-context transition now. Those writes are otherwise delayed about 2 s; call this before reading the journal directly or re-preparing a generation.
pub fn flush_personal_learning() {
    user_dictionary::ngram_store::flush_all()
}

/// Write every queued personal-context transition and stop the background thread that writes them, waiting for it to end. Call before unloading the engine or exiting the process; sessions still work afterwards, and the next transition they record starts the thread again.
pub fn shutdown() {
    user_dictionary::ngram_store::shutdown()
}

/// Drop every cached journal, personal-context and local-mode connection, on every thread (a connection an operation is using closes when that operation ends). Call before deleting or replacing a data directory.
pub fn close_cached_databases() {
    user_dictionary::journal::close_cached_journals()
}

/// 放掉进程里为下一个会话留着、没有会话在用的整句联想 n-gram store（连同它的模型和日志连接）。正在用的会话不受影响；之后第一个会话重新读日志。由宿主的清缓存、内存告警动作调用。
pub fn release_idle_personal_models() {
    user_dictionary::ngram_store::release_idle();
}
