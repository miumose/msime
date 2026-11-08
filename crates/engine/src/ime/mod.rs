//! `ImeSession`：当前方案、provider 和候选状态。精简版只处理全拼和双拼。

pub mod online_batch;
pub mod personal_rerank;
pub mod queries;
pub mod registry;
pub mod scheme;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::assets;
use crate::error::Result;
use crate::helpcode::SharedKeymap;
use crate::paths::RuntimePaths;
use crate::pinyin::autocorrect::autocorrect_suppression_key;
use crate::quanpin::QuanpinScheme;
use crate::shuangpin::query::{
    apply_segmentation_cases_into, detect_active_double_helpcode_length, remove_manual_delimiters,
    segment_input, to_quanpin_segmentation, trim_trailing_letters_preserve_delimiters,
};
use crate::shuangpin::ShuangpinProfile;
use crate::shuangpin::ShuangpinScheme;
use crate::types::{
    autocorrect_type, CandidateSource, FirstPageRows, FuzzyPinyinOptions, QueryRequest, SchemeKey,
    SchemeSet, SchemeType, SentenceAssociationOptions, ShuangpinProfileKind, WordItem,
};
use crate::user_dictionary::typo_profile::PersonalTypoProfile;

use registry::ProviderRegistry;
use scheme::Scheme;

/// What the providers answered for the current composition.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompositionState {
    pub preedit: String,
    pub request: QueryRequest,
    pub candidates: Vec<WordItem>,
}

pub struct ImeSession {
    scheme: Scheme,
    registry: ProviderRegistry,
    state: CompositionState,
    /// 光标前缀查询复用的临时请求存储，避免每次分页重新分配请求字段。
    scratch_request: QueryRequest,
    profile: &'static ShuangpinProfile,
    autocorrect_types: u32,
    quanpin_helpcode: bool,
    strict_quanpin_helpcode: bool,
    shuangpin_helpcode: bool,
    shuangpin_helpcode_modes: crate::types::ShuangpinHelpcodeModes,
    fuzzy: FuzzyPinyinOptions,
    sentence_alternatives: bool,
    /// 26 键全拼的整句读法条数下限，随每个请求交给全拼引擎（`QueryRequest::sentence_pool`）。
    sentence_pool: usize,
    /// 26 键全拼拼整句时给前缀打分的模型，随每个请求交给全拼引擎（`QueryRequest::sentence_scorer`）。
    sentence_scorer: crate::ordering::SentenceScorerHandle,
    /// 首页行数随每个请求交给各方案引擎；只影响词库要不要并入补全词。
    first_page_rows: FirstPageRows,
    sentence_association: SentenceAssociationOptions,
    rescoring_context: String,
    typo_profile: Arc<PersonalTypoProfile>,
}

impl ImeSession {
    /// 当前方案用的词库的粗俗词表，候选排位读它，和拼整句用同一张。
    pub(crate) fn offensive_words(&self) -> crate::lattice::offensive::OffensiveWords {
        self.registry.offensive_words(self.scheme.scheme_type())
    }

    /// `enabled` 是会话允许运行的方案，`scheme` 不在其中时报 `INPUT_SCHEME_NOT_ENABLED`。
    pub fn new(
        scheme: SchemeType,
        enabled: SchemeSet,
        profile: &'static ShuangpinProfile,
        paths: &RuntimePaths,
    ) -> Result<Self> {
        let mut registry = ProviderRegistry::new(enabled, profile, paths);
        registry.activate(scheme)?;
        Ok(Self {
            scheme: Scheme::new(scheme, profile),
            registry,
            state: CompositionState::default(),
            scratch_request: QueryRequest::default(),
            profile,
            autocorrect_types: 0,
            quanpin_helpcode: false,
            strict_quanpin_helpcode: false,
            shuangpin_helpcode: false,
            shuangpin_helpcode_modes: crate::types::ShuangpinHelpcodeModes::default(),
            fuzzy: FuzzyPinyinOptions::default(),
            sentence_alternatives: false,
            sentence_pool: 0,
            sentence_scorer: Default::default(),
            first_page_rows: FirstPageRows::default(),
            sentence_association: SentenceAssociationOptions::default(),
            rescoring_context: String::new(),
            typo_profile: PersonalTypoProfile::shared(&paths.user(assets::USER_JOURNAL)),
        })
    }

    pub fn candidates(&self) -> &[WordItem] {
        &self.state.candidates
    }

    pub fn request(&self) -> &QueryRequest {
        &self.state.request
    }

    pub fn preedit(&self) -> &str {
        &self.state.preedit
    }

    pub fn current_scheme_type(&self) -> SchemeType {
        self.scheme.scheme_type()
    }

    /// 会话允许运行的方案。
    pub fn enabled_schemes(&self) -> SchemeSet {
        self.registry.enabled()
    }

    /// `scheme.handle_key` then `refresh_candidates`; `SchemeKey::Requery` only refreshes.
    pub fn handle_key(&mut self, key: SchemeKey) -> bool {
        let changed = if key != SchemeKey::Requery {
            self.scheme.handle_key(key)
        } else {
            false
        };
        self.refresh_candidates();
        changed
    }

    pub fn activate(&mut self, scheme: SchemeType) -> Result<()> {
        self.registry.activate(scheme)
    }

    /// A new scheme and an empty state. 不在 `enabled_schemes` 里的方案不可用（`INPUT_SCHEME_NOT_ENABLED`），当前方案和组合保持不变。
    pub fn switch_scheme(&mut self, scheme: SchemeType) -> Result<()> {
        self.activate(scheme)?;
        self.scheme = Scheme::new(scheme, self.profile);
        self.bind_shuangpin_scheme();
        self.state = CompositionState::default();
        Ok(())
    }

    pub fn reset(&mut self) {
        self.scheme.reset();
        self.state = CompositionState::default();
    }

    /// Host editing for whichever scheme is active.
    pub fn replace_active_raw_input(&mut self, raw: &str, raw_with_cases: &str) {
        self.scheme.set_raw_input(raw, raw_with_cases);
        self.refresh_candidates();
    }

    /// 全拼词典里每个键（以 `'` 连接的完整音节）最好那一行的权重，供滑行输入使用。
    pub fn quanpin_best_weights(&self, keys: &[String]) -> HashMap<String, i64> {
        self.registry.quanpin_best_weights(keys)
    }

    /// Candidates for a raw prefix through a scratch scheme of the current type, leaving the live composition alone.
    pub fn query_raw_candidates(&mut self, raw: &str, raw_with_cases: &str) -> Vec<WordItem> {
        self.with_raw_request(
            |session, request| session.build_raw_request(raw, raw_with_cases, request),
            |session, request| {
                if request.valid {
                    session.decode(request)
                } else {
                    Vec::new()
                }
            },
        )
    }

    /// 与 [`Self::query_raw_candidates`] 相同，另外交回查询用的请求。
    pub fn query_raw_candidates_with_request(
        &mut self,
        raw: &str,
        raw_with_cases: &str,
    ) -> (Vec<WordItem>, QueryRequest) {
        self.with_raw_request(
            |session, request| session.build_raw_request(raw, raw_with_cases, request),
            |session, request| {
                let candidates = if request.valid {
                    session.decode(request)
                } else {
                    Vec::new()
                };
                (candidates, request.clone())
            },
        )
    }

    /// 扩展当前请求的光标前缀，直接借用请求里的带大小写原文。
    pub fn expand_current_raw_prefix_initial_candidates(
        &mut self,
        raw: &str,
        prefix_end: usize,
        candidates: &mut Vec<WordItem>,
    ) -> bool {
        self.expand_current_raw_range_initial_candidates(raw, 0..prefix_end, candidates)
    }

    /// 扩展当前请求原文里 `range` 那一段的首字母候选。
    pub fn expand_current_raw_range_initial_candidates(
        &mut self,
        raw: &str,
        range: std::ops::Range<usize>,
        candidates: &mut Vec<WordItem>,
    ) -> bool {
        let raw_with_cases = if self.state.request.raw_input_with_cases.is_empty() {
            &self.state.request.raw_input
        } else {
            &self.state.request.raw_input_with_cases
        };
        if raw_with_cases.get(range.clone()).is_none() {
            return false;
        }
        self.with_raw_request(
            |session, request| {
                let raw_with_cases = if session.state.request.raw_input_with_cases.is_empty() {
                    &session.state.request.raw_input
                } else {
                    &session.state.request.raw_input_with_cases
                };
                let raw_with_cases = &raw_with_cases[range];
                session.build_raw_request(raw, raw_with_cases, request);
            },
            |session, request| {
                request.valid
                    && session
                        .registry
                        .expand_initial_candidates(request, candidates)
            },
        )
    }

    /// The request a scratch scheme of the current type builds for `raw`, with the session's switches applied.
    fn build_raw_request(&self, raw: &str, raw_with_cases: &str, request: &mut QueryRequest) {
        match self.current_scheme_type() {
            SchemeType::Quanpin => {
                QuanpinScheme::build_request_from_raw_into(raw, raw_with_cases, request)
            }
            SchemeType::Shuangpin => ShuangpinScheme::build_request_from_raw_into(
                self.profile,
                raw,
                raw_with_cases,
                self.helpcode_uppercase_triggers(),
                request,
            ),
        }
        self.apply_request_options(request);
        self.apply_autocorrect_suppression(request);
        apply_shuangpin_helpcode_segmentation(request, self.profile);
    }

    fn with_raw_request<R>(
        &mut self,
        build: impl FnOnce(&Self, &mut QueryRequest),
        use_request: impl FnOnce(&mut Self, &QueryRequest) -> R,
    ) -> R {
        let mut request = std::mem::take(&mut self.scratch_request);
        build(self, &mut request);
        let result = use_request(self, &request);
        self.scratch_request = request;
        result
    }

    pub fn set_helpcode_keymap(&mut self, keymap: Option<SharedKeymap>) {
        self.registry.set_helpcode_keymap(keymap);
        self.refresh_candidates();
    }

    pub fn set_autocorrect_types(&mut self, types: u32) {
        self.autocorrect_types = types;
    }

    pub fn set_quanpin_helpcode_enabled(&mut self, enabled: bool) {
        self.quanpin_helpcode = enabled;
    }

    pub fn set_strict_quanpin_helpcode(&mut self, enabled: bool) {
        self.strict_quanpin_helpcode = enabled;
    }

    /// 一码严格筛选，全拼和双拼共用。
    pub(crate) fn strict_quanpin_helpcode(&self) -> bool {
        self.strict_quanpin_helpcode
    }

    pub fn set_shuangpin_helpcode_enabled(&mut self, enabled: bool) {
        self.shuangpin_helpcode = enabled;
        self.bind_shuangpin_scheme();
    }

    /// 双拼辅助码的触发方式；请求每次构建都带上，改动即时生效。
    pub fn set_shuangpin_helpcode_modes(&mut self, modes: crate::types::ShuangpinHelpcodeModes) {
        self.shuangpin_helpcode_modes = if modes.manual_tab {
            crate::types::ShuangpinHelpcodeModes {
                manual_tab: true,
                direct: false,
                trigger_backtick: false,
                trigger_semicolon: false,
                trigger_uppercase: false,
                mid_sentence: false,
                mid_sentence_backtick: false,
                mid_sentence_semicolon: false,
                mid_sentence_uppercase: false,
                direct_helpcode: false,
                direct_helpcode_slash: false,
                direct_helpcode_uppercase: false,
            }
        } else {
            modes
        };
        self.bind_shuangpin_scheme();
    }

    /// 大写字母开哪种辅助码段。
    pub(super) fn helpcode_uppercase_triggers(
        &self,
    ) -> crate::shuangpin::mid_sentence::UppercaseTriggers {
        crate::shuangpin::mid_sentence::uppercase_triggers(
            &self.shuangpin_helpcode_modes,
            self.shuangpin_helpcode,
        )
    }

    /// 大写段按当时的触发方式解析，方案层要拿到和敲键时一致的设置。
    fn bind_shuangpin_scheme(&mut self) {
        let triggers = self.helpcode_uppercase_triggers();
        if let Some(shuangpin) = self.scheme.as_shuangpin_mut() {
            shuangpin.set_helpcode_uppercase_triggers(triggers);
        }
    }

    pub fn set_fuzzy_pinyin_options(&mut self, options: FuzzyPinyinOptions) {
        self.fuzzy = options;
    }

    pub fn set_sentence_alternatives(&mut self, enabled: bool) {
        self.sentence_alternatives = enabled;
    }

    pub fn set_sentence_pool(&mut self, paths: usize) {
        self.sentence_pool = paths;
    }

    pub fn set_sentence_scorer(
        &mut self,
        scorer: Option<std::sync::Arc<dyn crate::ordering::SentenceScorer>>,
    ) {
        self.sentence_scorer = crate::ordering::SentenceScorerHandle(scorer);
    }

    /// Carried on every request; the engines reset their caches when it changes.
    pub fn set_sentence_association(&mut self, options: SentenceAssociationOptions) {
        self.sentence_association = options;
    }

    /// 首页行数随每个请求交给各方案引擎。
    pub fn set_first_page_rows(&mut self, rows: FirstPageRows) {
        self.first_page_rows = rows;
    }

    pub fn set_rescoring_context(&mut self, context: &str) {
        context.clone_into(&mut self.rescoring_context);
    }

    pub fn find_candidate(&self, scheme: SchemeType, key: &str, value: &str) -> Option<WordItem> {
        self.registry.find_candidate(scheme, key, value)
    }

    /// Resets the current scheme's provider.
    pub fn reset_cache(&mut self) {
        let scheme = self.current_scheme_type();
        self.registry.reset_cache(scheme);
        self.refresh_candidates();
    }

    pub fn expand_initial_candidates(&mut self) -> bool {
        self.registry
            .expand_initial_candidates(&self.state.request, &mut self.state.candidates)
    }

    /// Insert online rows for the current request and refresh; false when the provider could not take them.
    pub fn apply_dynamic_candidates(&mut self, words: &[String], source: CandidateSource) -> bool {
        if !self
            .registry
            .cache_dynamic_candidates_for_request(&self.state.request, words, source)
        {
            return false;
        }
        self.refresh_candidates();
        true
    }

    /// Remove one provider's rows from all provider caches and rebuild the live candidate list.
    pub fn clear_online_candidates(&mut self, source: CandidateSource) {
        if !source.is_online() {
            return;
        }
        self.registry.clear_online_candidates(source);
        self.refresh_candidates();
    }

    fn refresh_candidates(&mut self) {
        let mut request = std::mem::take(&mut self.state.request);
        self.scheme.build_request_into(&mut request);
        self.apply_request_options(&mut request);
        self.apply_autocorrect_suppression(&mut request);
        apply_shuangpin_helpcode_segmentation(&mut request, self.profile);
        reuse_request_preedit(&request, &mut self.state.preedit);
        if !request.valid {
            self.state.request = request;
            self.state.candidates.clear();
            return;
        }
        let candidates = self.decode(&request);
        self.state.request = request;
        self.state.candidates = candidates;
    }

    /// The scheme's request with the session's switches, autocorrect suppression and the shuangpin double-helpcode segmentation applied.
    #[allow(dead_code)]
    fn prepare_request(&self, scheme: &Scheme) -> QueryRequest {
        let mut request = scheme.build_request();
        self.apply_request_options(&mut request);
        self.apply_autocorrect_suppression(&mut request);
        apply_shuangpin_helpcode_segmentation(&mut request, self.profile);
        request
    }

    fn apply_request_options(&self, request: &mut QueryRequest) {
        request.enable_shuangpin_helpcode = self.shuangpin_helpcode;
        request.shuangpin_helpcode_modes = self.shuangpin_helpcode_modes;
        if !(self.shuangpin_helpcode
            && !request.shuangpin_helpcode_modes.direct
            && !request.shuangpin_helpcode_modes.direct_helpcode)
        {
            request.end_helpcodes.clear();
        }
        request.enable_quanpin_helpcode = self.quanpin_helpcode;
        request.strict_quanpin_helpcode = self.strict_quanpin_helpcode;
        request.sentence_alternatives = self.sentence_alternatives;
        request.sentence_pool = self.sentence_pool;
        request.sentence_scorer = self.sentence_scorer.clone();
        request.enable_quanpin_autocorrect_transposition =
            self.autocorrect_types & autocorrect_type::TRANSPOSITION != 0;
        request.enable_quanpin_autocorrect_neighbor =
            self.autocorrect_types & autocorrect_type::NEIGHBOR != 0;
        request.fuzzy_pinyin = self.fuzzy;
        request.sentence_association = self.sentence_association.clone();
        request.first_page_rows = self.first_page_rows;
        request
            .rescoring_context
            .clone_from(&self.rescoring_context);
    }

    /// An input the user committed raw while a correction was offered is answered as typed.
    fn apply_autocorrect_suppression(&self, request: &mut QueryRequest) {
        if request.scheme != SchemeType::Quanpin || self.autocorrect_types == 0 {
            return;
        }
        self.typo_profile.refresh_if_changed();
        let cases = if request.enable_quanpin_helpcode {
            &request.raw_input_with_cases
        } else {
            &request.raw_input
        };
        let key = autocorrect_suppression_key(&request.raw_input, cases);
        if !self.typo_profile.suppressed(&key) {
            return;
        }
        request.enable_quanpin_autocorrect_transposition = false;
        request.enable_quanpin_autocorrect_neighbor = false;
        QuanpinScheme::apply_literal_segmentation(request);
    }

    fn decode(&mut self, request: &QueryRequest) -> Vec<WordItem> {
        self.registry.query(request)
    }
}

/// 从已构造的请求复用方案显示文本。
fn reuse_request_preedit(request: &QueryRequest, destination: &mut String) {
    let source = if !request.raw_input_with_helpcode_marks.is_empty() {
        &request.raw_input_with_helpcode_marks
    } else {
        &request.raw_input_with_cases
    };
    source.clone_into(destination);
}

/// With a double helpcode after a complete shuangpin base, the segmentations become the base's plus `'` and the two help letters.
fn apply_shuangpin_helpcode_segmentation(request: &mut QueryRequest, profile: &ShuangpinProfile) {
    const HELPCODE_LENGTH: usize = 2;
    if request.scheme != SchemeType::Shuangpin
        || !request.enable_shuangpin_helpcode
        || !request.shuangpin_helpcode_modes.direct
        || detect_active_double_helpcode_length(
            &request.raw_input,
            &request.raw_input_with_cases,
            profile,
        ) != HELPCODE_LENGTH
    {
        return;
    }
    let base_raw = trim_trailing_letters_preserve_delimiters(&request.raw_input, HELPCODE_LENGTH);
    let base_raw_with_cases =
        trim_trailing_letters_preserve_delimiters(&request.raw_input_with_cases, HELPCODE_LENGTH);
    let base_segmentation = segment_input(&base_raw, profile);
    let effective_with_cases = remove_manual_delimiters(&request.raw_input_with_cases);
    let help_codes = &effective_with_cases[effective_with_cases.len() - HELPCODE_LENGTH..];

    apply_segmentation_cases_into(
        &base_segmentation,
        &base_raw_with_cases,
        &mut request.raw_segmentation,
    );
    append_help_codes(&mut request.raw_segmentation, help_codes);
    request
        .normalized_segmentation
        .clone_from(&to_quanpin_segmentation(&base_segmentation, profile));
    append_help_codes(&mut request.normalized_segmentation, help_codes);
    request
        .segmentation
        .clone_from(&request.normalized_segmentation);
}

fn append_help_codes(segmentation: &mut String, help_codes: &str) {
    segmentation.push('\'');
    segmentation.push_str(help_codes);
}
