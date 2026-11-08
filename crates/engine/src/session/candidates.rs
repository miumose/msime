//! The candidate merge pipeline and candidate actions (core-session.md §7.2-§7.3): personal rerank, mixed rows, fixed positions, pin, removal and position fixing.

use std::borrow::Cow;

use super::input::InputSession;
use crate::assets;
use crate::diagnostics;
use crate::ime::personal_rerank::personal_context_rerank_order;
use crate::ime::queries::{keep_expressive_rows_off_the_lead, MODE_ENGLISH_LIMIT};
use crate::local::date_time::{inline_date_time_keyword, insert_inline_date_time, LocalDateTime};
use crate::local::jianpin::jianpin_ranking_context;
use crate::offensive_placement::{
    evict_after_positions, keep_off_first_page, spells_whole_input, typed_demotion,
};
use crate::pinyin::segment::{cut_pinyin_by_mode, join_segments, CutMode};
use crate::text::{count_utf8_chars, is_han_phrase};
use crate::types::{
    CandidateSource, FrequencyAdjustmentMode, FrequencyAdjustmentOptions, KeyResult,
    LocalInputMode, PersonalDictionaryKind, SchemeType, WordItem,
};
use crate::user_dictionary::journal::has_matching_user_weight;
use crate::user_dictionary::positions::{
    apply_fixed_positions_with_state, clear_fixed_position, is_pinned_candidate,
    record_pinned_candidate, set_fixed_position,
};
use crate::user_dictionary::ranking::candidate_dictionary_key;
use crate::user_dictionary::removal::delete_dictionary_candidate;

fn english_position_context(input: &str) -> String {
    let mut context = String::with_capacity("english:".len() + input.len());
    context.push_str("english:");
    context.extend(
        input
            .chars()
            .map(|character| character.to_ascii_lowercase()),
    );
    context
}

fn plain_position_context(context: &str) -> String {
    let mut plain = String::with_capacity(context.len());
    plain.extend(context.chars().filter(|character| *character != '\''));
    plain
}

pub(super) fn clone_candidate_rows(source: &[WordItem], destination: &mut Vec<WordItem>) {
    let common = source.len().min(destination.len());
    for (target, item) in destination.iter_mut().take(common).zip(source.iter()) {
        target.pinyin.clone_from(&item.pinyin);
        target.canonical_pinyin.clone_from(&item.canonical_pinyin);
        target.word.clone_from(&item.word);
        target.weight = item.weight;
        target.source = item.source;
        target.scheme = item.scheme;
        target.fixed_position = item.fixed_position;
        target.fuzzy = item.fuzzy;
        target.corrected_from.clone_from(&item.corrected_from);
        target.sentence_association = item.sentence_association;
        target.sentence_words.clone_from(&item.sentence_words);
        target.touch_supported = item.touch_supported;
        target.confident_correction = item.confident_correction;
    }
    if destination.len() > source.len() {
        destination.truncate(source.len());
    } else {
        destination.extend(source[common..].iter().cloned());
    }
}

impl InputSession {
    /// 仅改显示或上屏副本，内部词条保留词库拼写供调频、置顶和删除定位。
    pub(super) fn apply_english_word_case(&self, source: CandidateSource, word: &mut String) {
        if source != CandidateSource::EnglishDictionary
            || self.local_mode != LocalInputMode::None
            || self.dedicated_english
            || (self.has_active_helpcode() && !self.has_literal_input())
        {
            return;
        }
        let raw = &self.engine.request().raw_input_with_cases;
        if !raw.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return;
        }
        if raw.len() > 1 && raw.bytes().all(|byte| byte.is_ascii_uppercase()) {
            word.make_ascii_uppercase();
        } else if word
            .get(..raw.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(raw))
        {
            word.replace_range(..raw.len(), raw);
        } else if raw.as_bytes().first().is_some_and(u8::is_ascii_uppercase) {
            if let Some(first) = word.get_mut(..1) {
                first.make_ascii_uppercase();
            }
        }
    }

    /// 前缀或引擎候选经过个人上下文重排，再混入英文、表情、颜文字并应用固定位置。
    pub(super) fn update_mixed_candidates(&mut self) {
        if !self.has_composition() {
            self.glide_selection_pending = false;
        }
        if let Some((spelling, _)) = &self.glide_candidates {
            if self.local_mode == LocalInputMode::None
                && !self.dedicated_english
                && self.raw_with_cases() == spelling
                && self.caret_position() == spelling.len()
            {
                return;
            }
        }
        self.glide_candidates = None;
        if self.update_literal_candidates() {
            return;
        }
        self.refresh_prefix_candidates();
        let mut decoded = self
            .ranking_candidates
            .take()
            .unwrap_or_else(|| std::mem::take(&mut self.mixed_candidates));
        let mut reranked = std::mem::take(&mut self.mixed_candidates);
        if self.prefix_active {
            clone_candidate_rows(&self.prefix_candidates, &mut decoded);
        } else {
            clone_candidate_rows(self.engine.candidates(), &mut decoded);
            // 快捷短语的独立开关同时约束普通候选入口，联系人在最终混排后单独加入。五笔 14 键的组码不是用户打出的字母，不拿它去对快捷短语的码。光标后编辑不混入快捷短语：选中短语会结束整段组字。
            if self.quick_phrase_candidates
                && !self.caret_edit.active
                && self.mixed_candidates_enabled
                && self.local_mode_options.quick_phrase
            {
                let code = self.engine.request().raw_input.clone();
                // 调频开着时整组按学下来的槽位放；没有记录（或调频关掉）时落在首页尾部，普通候选不满一页时接在它们后面。
                let slot = (self.quick_phrase_frequency
                    && self.frequency.mode != FrequencyAdjustmentMode::Disabled)
                    .then(|| {
                        crate::user_dictionary::ranking::quick_phrase_slot(
                            self.journal_path(),
                            &code,
                        )
                    })
                    .flatten();
                self.queries.insert_quick_phrase_candidates(
                    &code,
                    &mut decoded,
                    slot,
                    self.first_page_rows,
                );
            }
        }
        self.personal_reranked = false;
        // 提交链起点没有上下文，候选偏好由词频设置负责。
        let reordered = match self.chain.previous.as_deref() {
            Some(previous) if self.personal_context_applies() => {
                let journal = self.journal_path();
                let context = self.pinyin_ranking_context();
                let model = self.personal_context.model();
                personal_context_rerank_order(
                    &decoded,
                    &model,
                    self.chain.earlier.as_deref(),
                    previous,
                    self.scheme(),
                    &mut |leader: &str| {
                        is_pinned_candidate(journal, &context, leader)
                            || (self.frequency.mode != FrequencyAdjustmentMode::Disabled
                                && decoded.first().is_some_and(|row| {
                                    if !row.source.is_dictionary() || !row.scheme.is_pinyin() {
                                        return false;
                                    }
                                    // 用调频写入的真实读音匹配，单码和双拼不能拿显示编码查日志。
                                    let key = if row.canonical_pinyin.is_empty() {
                                        Cow::Owned(candidate_dictionary_key(row, &context))
                                    } else {
                                        Cow::Borrowed(row.canonical_pinyin.as_ref())
                                    };
                                    has_matching_user_weight(
                                        journal,
                                        PersonalDictionaryKind::Pinyin,
                                        &key,
                                        &row.word,
                                        row.weight,
                                    )
                                }))
                    },
                )
            }
            _ => None,
        };
        match reordered {
            Some(order) => {
                // 先复制原始行到可复用的显示缓冲，再按索引原地重排，避免重新分配每个字符串。
                clone_candidate_rows(&decoded, &mut reranked);
                order.reorder(&mut reranked);
                self.mixed_candidates = self.mixed_from(reranked);
                // 学习使用字典给出的顺序，因此另存未重排的混排列表。
                self.ranking_candidates = Some(self.mixed_from(decoded));
                self.personal_reranked = true;
            }
            None => self.mixed_candidates = self.mixed_from(decoded),
        }
    }

    /// The candidates learning ranks against: the list before the personal rerank.
    pub(super) fn ranking_list(&self) -> &[WordItem] {
        self.ranking_candidates
            .as_deref()
            .unwrap_or_else(|| self.candidates())
    }

    pub(super) fn update_local_candidates(&mut self) -> Option<String> {
        // Only the date/time and command modes read the wall clock; the other modes would pay a local-offset lookup per key for nothing.
        let now = if matches!(
            self.local_mode,
            LocalInputMode::DateTime | LocalInputMode::Command
        ) {
            self.local_now()
        } else {
            LocalDateTime::default()
        };
        let scheme = self.scheme();
        let result = self.queries.local(
            self.local_mode,
            &self.local_preedit,
            scheme,
            &now,
            self.engine.candidates(),
        );
        let mut candidates = result.candidates;
        // 超级简拼查出来的是共享词库的行，和 26 键的简拼一样：粗俗词不进首屏（`offensive_placement`）。简拼不是完整拼写，所以没有「只让出首选」的情形。
        let jianpin_vulgar = (self.local_mode == LocalInputMode::SuperJianpin)
            .then(|| self.engine.offensive_words());
        let page = self.engine.request().first_page_rows.get();
        if let Some(vulgar) = &jianpin_vulgar {
            keep_off_first_page(&mut candidates, page, |item| {
                typed_demotion(item, vulgar, |_| false)
            });
        }
        self.apply_candidate_positions(&mut candidates);
        if let Some(vulgar) = &jianpin_vulgar {
            evict_after_positions(&mut candidates, page, |item| {
                typed_demotion(item, vulgar, |_| false)
            });
        }
        self.local_candidates = candidates;
        self.insert_cached_plugin_rows();
        // After the positions, so the synthetic row is never moved.
        self.add_local_fallback_candidate();
        if self.local_mode == LocalInputMode::Url {
            self.add_url_completions();
        }
        if self.local_mode == LocalInputMode::TemporaryEnglish {
            let mut candidates = std::mem::take(&mut self.local_candidates);
            self.append_english_online_rows(&mut candidates);
            self.local_candidates = candidates;
        }
        result.diagnostic
    }

    /// 缓存的插件行还属于这份预编辑串时把它们放回列表前面，同文的后面那一行去掉；预编辑串已经变了就清掉缓存，下一次派发重新问插件。
    fn insert_cached_plugin_rows(&mut self) {
        let Some((preedit, rows)) = &self.plugin_rows else {
            return;
        };
        if *preedit != self.local_preedit {
            self.plugin_rows = None;
            return;
        }
        if rows.is_empty() {
            return;
        }
        self.local_candidates
            .retain(|item| !rows.iter().any(|row| row.word == item.word));
        self.local_candidates.splice(0..0, rows.iter().cloned());
    }

    pub(super) fn add_local_fallback_candidate(&mut self) {
        // 未完成或未匹配的特殊模式把原文作为候选，让空格仍走普通选词路径（overlays.md §8.1）。手动进入时保留前导字母；组字后按 `/` 转出的超级简拼不带 `J`，因为它不是用户输入的字符（`local_raw_text`）。
        if self.local_candidates.is_empty() && !self.local_preedit.is_empty() {
            let text = self.local_raw_text().to_owned();
            self.local_candidates.push(WordItem::new(
                text.clone(),
                text,
                1,
                CandidateSource::Fallback,
                "",
            ));
        }
    }

    pub(super) fn update_dedicated_english_candidates(&mut self) {
        if self.dedicated_english_preedit.is_empty() {
            self.dedicated_english_candidates.clear();
            return;
        }
        let prefix = crate::ime::queries::lowercase_prefix(&self.dedicated_english_preedit);
        self.queries.english_dictionary().query_prefix_into(
            &prefix,
            MODE_ENGLISH_LIMIT,
            &mut self.dedicated_english_candidates,
        );
        let mut candidates = std::mem::take(&mut self.dedicated_english_candidates);
        self.apply_candidate_positions(&mut candidates);
        if candidates.is_empty() {
            // A word the dictionary does not know is still what the user is typing.
            candidates.push(WordItem::new(
                "",
                self.dedicated_english_preedit.clone(),
                0,
                CandidateSource::Generated,
                "",
            ));
        }
        self.append_english_online_rows(&mut candidates);
        self.dedicated_english_candidates = candidates;
    }

    /// input_session_candidates.cpp:19-61 with the per-producer split of overlays.md §3.3.
    pub(super) fn apply_candidate_positions(&mut self, items: &mut Vec<WordItem>) {
        if items.is_empty() {
            return;
        }
        let regular = self.local_mode == LocalInputMode::None && !self.dedicated_english;
        let has_english = if !regular && self.local_mode != LocalInputMode::SuperJianpin {
            if !items
                .iter()
                .any(|item| item.source == CandidateSource::EnglishDictionary)
            {
                return;
            }
            true
        } else {
            false
        };
        let journal = self.journal_path();
        let journal_exists = journal.try_exists().unwrap_or(false);
        if !journal_exists && !items.iter().any(|item| item.source.takes_online_seat()) {
            return;
        }
        let include_missing = self.engine.request().raw_input.len() == 1;
        let keep_dynamic = self.has_active_helpcode();
        let engine = &self.engine;
        if regular {
            let context = self.position_context(false);
            let scheme = self.scheme();
            let mut finder = |key: &str, word: &str| engine.find_candidate(scheme, key, word);
            apply_fixed_positions_with_state(
                journal,
                context.as_ref(),
                items,
                include_missing,
                Some(&mut finder),
                keep_dynamic,
                journal_exists,
            );
        } else if self.local_mode == LocalInputMode::SuperJianpin {
            let context = self.position_context(false);
            apply_fixed_positions_with_state(
                journal,
                context.as_ref(),
                items,
                false,
                None,
                false,
                journal_exists,
            );
        }
        if has_english
            || items
                .iter()
                .any(|item| item.source == CandidateSource::EnglishDictionary)
        {
            let context = self.position_context(true);
            apply_fixed_positions_with_state(
                journal,
                context.as_ref(),
                items,
                false,
                None,
                true,
                journal_exists,
            );
        }
    }

    /// input_session_candidates.cpp:21-44.
    pub(super) fn position_context(&self, english: bool) -> Cow<'_, str> {
        let request = self.engine.request();
        if english {
            let input = if self.dedicated_english {
                &self.dedicated_english_preedit
            } else if self.local_mode == LocalInputMode::TemporaryEnglish {
                &self.local_preedit[1..]
            } else {
                &request.raw_input_with_cases
            };
            return Cow::Owned(english_position_context(input));
        }
        if self.local_mode == LocalInputMode::SuperJianpin {
            return Cow::Owned(jianpin_ranking_context(
                &self.local_preedit[1..],
                self.scheme(),
                self.shuangpin_profile(),
            ));
        }
        let context: Cow<'_, str> = if request.normalized_input.is_empty() {
            self.pinyin_segmentation()
        } else {
            Cow::Borrowed(&request.normalized_input)
        };
        if request.raw_input.len() == 1 {
            return context;
        }
        // Positions are stored under one canonical cut, so the same letters typed with or without apostrophes share them.
        let plain = if context.contains('\'') {
            Cow::Owned(plain_position_context(context.as_ref()))
        } else {
            Cow::Borrowed(context.as_ref())
        };
        match cut_pinyin_by_mode(&plain, CutMode::Correction).first() {
            Some(cut) => Cow::Owned(join_segments(cut)),
            None => context,
        }
    }

    /// The pre-rerank index of a displayed candidate.
    pub(super) fn ranking_index(&self, displayed: usize) -> Option<usize> {
        let shown = self.candidates().get(displayed)?;
        let Some(ordered) = self.ranking_candidates.as_deref() else {
            return Some(displayed);
        };
        let found = ordered.iter().position(|item| {
            item.word == shown.word
                && item.source == shown.source
                && item.pinyin == shown.pinyin
                && item.canonical_pinyin == shown.canonical_pinyin
        });
        Some(found.unwrap_or(displayed))
    }

    /// 0 clears.
    pub(super) fn set_candidate_position(&mut self, index: usize, position: i32) -> KeyResult {
        // 改字的候选是光标处一段的替换，不是这串拼音的候选，不能固定、置顶或删除。
        if self.conversion.is_some() || self.date_time_menu_page.is_some() {
            return KeyResult::unhandled();
        }
        if !(0..=5).contains(&position) {
            return KeyResult::unhandled();
        }
        let Some(selected) = self.candidates().get(index) else {
            return KeyResult::unhandled();
        };
        if !self.is_editable_source(selected) {
            return KeyResult::unhandled();
        }
        let english = selected.source == CandidateSource::EnglishDictionary;
        let context = self.position_context(english);
        let key = if english || selected.canonical_pinyin.is_empty() {
            &selected.pinyin
        } else {
            &selected.canonical_pinyin
        };
        if context.is_empty() || key.is_empty() {
            return KeyResult::unhandled();
        }
        let journal = self.journal_path();
        let written = if position == 0 {
            clear_fixed_position(journal, context.as_ref(), key, &selected.word)
        } else {
            set_fixed_position(journal, context.as_ref(), key, &selected.word, position)
        };
        if written.is_err() {
            return KeyResult::handled()
                .with_diagnostic(Some(diagnostics::POSITION_NOT_PERSISTED.to_owned()));
        }
        KeyResult::handled().with_diagnostic(self.refresh_after_candidate_change())
    }

    pub(super) fn pin_candidate(&mut self, index: usize) -> KeyResult {
        // 「📅日期」页里的行按日期格式索引，pin 会落到真实列表里同下标的另一行。
        if self.conversion.is_some() || self.date_time_menu_page.is_some() {
            return KeyResult::unhandled();
        }
        let Some(index) = self.ranking_index(index) else {
            return KeyResult::unhandled();
        };
        let Some(selected) = self.ranking_list().get(index) else {
            return KeyResult::unhandled();
        };
        if !self.is_editable_source(selected) {
            return KeyResult::unhandled();
        }
        let selected_source = selected.source;
        let selected_word = selected.word.clone();
        // Manual pinning is independent of the automatic learning preferences and never selects text.
        let pin = FrequencyAdjustmentOptions {
            mode: FrequencyAdjustmentMode::Pin,
            trigger_count: 1,
            linear_step: 1,
        };
        if let Some(diagnostic) = self.adjust_candidate_frequency(index, pin, true) {
            return KeyResult::handled().with_diagnostic(Some(diagnostic));
        }
        // Remembered so that the personal context rerank does not move the pinned word off the top it was pinned to.
        if selected_source != CandidateSource::EnglishDictionary
            && self.local_mode == LocalInputMode::None
            && !self.dedicated_english
            && self.scheme().is_pinyin()
            && record_pinned_candidate(
                self.journal_path(),
                &self.pinyin_ranking_context(),
                &selected_word,
            )
            .is_err()
        {
            return KeyResult::handled()
                .with_diagnostic(Some(diagnostics::PIN_NOT_PERSISTED.to_owned()));
        }
        KeyResult::handled().with_diagnostic(self.refresh_after_candidate_change())
    }

    pub(super) fn remove_candidate(&mut self, index: usize) -> KeyResult {
        if self.conversion.is_some() || self.date_time_menu_page.is_some() {
            return KeyResult::unhandled();
        }
        let Some(selected) = self.candidates().get(index) else {
            return KeyResult::unhandled();
        };
        let english = selected.source == CandidateSource::EnglishDictionary;
        // A single character is protected: removing it would leave its reading unanswerable.
        if !self.is_editable_source(selected) || (!english && count_utf8_chars(&selected.word) <= 1)
        {
            return KeyResult::unhandled();
        }
        let kind = if english {
            PersonalDictionaryKind::English
        } else {
            PersonalDictionaryKind::Pinyin
        };
        // A displayed pinyin row already carries its exact dictionary key; re-segmenting it could delete another pronunciation of the same word.
        let key = if english {
            &selected.pinyin
        } else {
            &selected.canonical_pinyin
        };
        if key.is_empty() {
            return KeyResult::unhandled();
        }
        let dictionary = self.paths.dictionary(if english {
            assets::ENGLISH_DICTIONARY
        } else {
            assets::MAIN_DICTIONARY
        });
        if delete_dictionary_candidate(&dictionary, self.journal_path(), kind, key, &selected.word)
            .is_err()
        {
            return KeyResult::handled()
                .with_diagnostic(Some(diagnostics::REMOVAL_NOT_PERSISTED.to_owned()));
        }
        KeyResult::handled().with_diagnostic(self.refresh_after_candidate_change())
    }

    /// Caches reset and the active view rebuilt after a dictionary edit (input_session.cpp:1221-1235).
    fn refresh_after_candidate_change(&mut self) -> Option<String> {
        self.reset_cache();
        if self.dedicated_english {
            self.update_dedicated_english_candidates();
            return None;
        }
        if self.local_mode != LocalInputMode::None {
            return self.update_local_candidates();
        }
        self.recompute_candidates();
        None
    }

    fn placement_input(&self) -> &str {
        if self.prefix_active {
            self.prefix_query_input.as_str()
        } else {
            self.engine.request().raw_input.as_str()
        }
    }

    /// 对一份解码结果混排并应用固定位置。
    fn mixed_from(&mut self, mut decoded: Vec<WordItem>) -> Vec<WordItem> {
        // 粗俗词不当首选，前缀、部分候选和简拼查出来的不进首屏（`offensive_placement`）。在混入英文、日文和固定位置之前挪：混入的行坐的是首选之后的位置，挪动不能把它们推上首选；用户固定的位置照旧生效。英文粗俗词由 `CandidateQueries::mixed` 接在末尾。
        let scheme = self.scheme();
        let page = self.engine.request().first_page_rows.get();
        let vulgar = (scheme.is_pinyin() && self.local_mode == LocalInputMode::None)
            .then(|| self.engine.offensive_words());
        if let Some(vulgar) = &vulgar {
            let (input, profile) = (self.placement_input(), self.profile);
            keep_off_first_page(&mut decoded, page, |item| {
                typed_demotion(item, vulgar, |item| {
                    spells_whole_input(item, input, scheme, profile)
                })
            });
        }
        // 拼音拆不开时完全匹配的英文可以占首位，但有两种情况不行：光标前缀解码时选英文会扔掉光标后的输入（`commit_selection` 整段结束组字）；辅助码激活时大写的辅助码被转成小写拼进了查英文的输入，那串字母不是用户要的英文词。
        let english_may_lead = !self.prefix_active && !self.has_active_helpcode();
        // 光标后编辑里选中英文、emoji、颜文字、符号这些混排行会结束整段组字，扔掉光标前固定的那段或光标后剩下的拼写，所以一概不混入。
        let mixed_enabled = self.mixed_candidates_enabled && !self.caret_edit.active;
        // 符号行要整段组字正好是符号的名称，条件和上面相同但语义不同：光标前缀解码时查的是光标前那段，辅助码激活时输入里混着辅助码字母，两种情况下整段都不是用户打的名称。
        let symbols_allowed = !self.prefix_active && !self.has_active_helpcode();
        let association_input = if self.prefix_active {
            self.prefix_query_input.as_str()
        } else {
            self.engine.request().raw_input.as_str()
        };
        let mut mixed = self.queries.mixed(
            decoded,
            association_input,
            scheme,
            self.english_options,
            self.expressive_options,
            self.dedicated_english,
            self.local_mode,
            english_may_lead,
            symbols_allowed,
            self.engine.request().first_page_rows.get(),
            mixed_enabled,
        );
        self.insert_inline_date_time(&mut mixed);
        self.apply_candidate_positions(&mut mixed);
        if let Some(vulgar) = &vulgar {
            let (input, profile) = (self.placement_input(), self.profile);
            evict_after_positions(&mut mixed, page, |item| {
                typed_demotion(item, vulgar, |item| {
                    spells_whole_input(item, input, scheme, profile)
                })
            });
        }
        // 云候选异步到达；先在它的固定席位放一个云图标，回包时原位替换，避免本地候选整体后移。
        if self.cloud_candidates_enabled
            && self
                .online_query()
                .is_some_and(|query| query.cloud_eligible)
            && !mixed.iter().any(|item| {
                matches!(
                    item.source,
                    CandidateSource::CloudSuggestion | CandidateSource::CloudPending
                )
            })
        {
            let position = 1.min(mixed.len());
            mixed.insert(
                position,
                WordItem::new(
                    self.engine.request().raw_input.as_str(),
                    "☁",
                    0,
                    CandidateSource::CloudPending,
                    "",
                ),
            );
        }
        // 放在固定位置之后：单字母输入时固定位置会补回列表里没有的词。
        if self.single_character_only && scheme.filters_to_single_characters() {
            mixed.retain(|item| !is_han_phrase(&item.word));
        }
        // 本地模式不混排，列表里的 emoji、颜文字是模式自己查的，可以领头。
        if self.local_mode == LocalInputMode::None {
            keep_expressive_rows_off_the_lead(&mut mixed);
        }
        // 放在最后：网址补全不参与固定位置，也不被挪开。光标后编辑里选中网址补全会进入网址模式，同样不加。
        if !self.caret_edit.active {
            self.insert_url_scheme_completions(&mut mixed);
        }
        // 联系人优先级高于固定位置和云候选；两行相邻，不能让云占位插进姓名与组合文本之间。
        // 光标前缀只解码局部，联系人选择却会结束整段组字，因此只在完整普通拼音组字中加入。
        if !self.prefix_active
            && !self.caret_edit.active
            && !self.dedicated_english
            && self.local_mode == LocalInputMode::None
            && !self.has_active_helpcode()
            && !self.single_character_only
        {
            let input = match scheme {
                SchemeType::Quanpin => {
                    Some(Cow::Borrowed(self.engine.request().raw_input.as_str()))
                }
                SchemeType::Shuangpin => Some(self.pinyin_ranking_context()),
                _ => None,
            };
            if let Some(input) = input {
                self.queries.append_contact_candidates(&input, &mut mixed);
            }
        }
        mixed
    }

    /// 全拼组字正好是 `riqi`、`sj` 这类关键词时，把当前日期、时间、星期或农历接在对应的词后面（#5952）。与日期时间模式（T 模式）是两个开关：这里只看 `date_time_inline`；光标前缀解码、只出单字、专用英文和本地模式里不加。
    fn insert_inline_date_time(&self, items: &mut Vec<WordItem>) {
        if !self.local_mode_options.date_time_inline
            || !self.mixed_candidates_enabled
            || self.prefix_active
            || self.caret_edit.active
            || self.single_character_only
            || self.dedicated_english
            || self.local_mode != LocalInputMode::None
            || self.scheme() != SchemeType::Quanpin
        {
            return;
        }
        let raw = self.engine.request().raw_input.as_str();
        let Some((keyword, anchor, kind)) = inline_date_time_keyword(|keyword| {
            raw.bytes()
                .filter(|&byte| byte != b'\'')
                .eq(keyword.bytes())
        }) else {
            return;
        };
        let menu = self.local_mode_options.date_time_menu.then_some(keyword);
        insert_inline_date_time(items, anchor, kind, raw, || self.local_now(), menu);
    }
}

#[cfg(test)]
mod tests {
    use super::{clone_candidate_rows, english_position_context, plain_position_context};
    use crate::types::{CandidateSource, WordItem};

    #[test]
    fn english_position_context_preserves_non_ascii_while_lowercasing_ascii() {
        assert_eq!(english_position_context("HeLLo 世界"), "english:hello 世界");
    }

    #[test]
    fn plain_position_context_reserves_source_capacity() {
        let source: String = (0..100)
            .map(|index| if index % 5 == 0 { '\'' } else { 'a' })
            .collect();
        let plain = plain_position_context(&source);
        assert_eq!(plain, source.replace('\'', ""));
        assert_eq!(plain.capacity(), source.len());
    }

    #[test]
    fn cloning_candidate_rows_reuses_existing_storage() {
        let source = vec![
            WordItem::new("ni", "你", 1, CandidateSource::Database, "ni"),
            WordItem::new("hao", "好", 1, CandidateSource::Database, "hao"),
        ];
        let mut destination = source.clone();

        let ((), allocations) = crate::ime::personal_rerank::allocations::count(|| {
            clone_candidate_rows(&source, &mut destination);
        });

        assert_eq!(destination, source);
        assert_eq!(allocations, 0);
    }

    #[test]
    fn cloning_candidate_rows_replaces_metadata_and_handles_length_changes() {
        let marked = WordItem {
            pinyin: "ni'hao".into(),
            canonical_pinyin: "li'hao".into(),
            word: "合成词".into(),
            weight: 42,
            source: CandidateSource::Generated,
            scheme: crate::types::SchemeType::Shuangpin,
            fixed_position: 2,
            fuzzy: true,
            corrected_from: "lihao".into(),
            sentence_association: true,
            sentence_words: vec!["合".to_owned(), "成词".to_owned()],
            touch_supported: true,
            confident_correction: true,
        };
        let plain = WordItem::new("ni", "你", 1, CandidateSource::Database, "ni");
        let mut destination = vec![marked.clone(), marked.clone(), marked.clone()];
        let source = vec![plain.clone(), marked];

        clone_candidate_rows(&source, &mut destination);
        assert_eq!(destination, source);

        let larger = vec![plain; 4];
        clone_candidate_rows(&larger, &mut destination);
        assert_eq!(destination, larger);
        clone_candidate_rows(&[], &mut destination);
        assert!(destination.is_empty());
    }

}
