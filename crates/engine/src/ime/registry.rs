//! 拼音方案的 provider：全拼和双拼。精简版只有这两种方案。

use std::collections::HashMap;

use crate::diagnostics;
use crate::error::{EngineError, Result};
use crate::helpcode::SharedKeymap;
use crate::paths::RuntimePaths;
use crate::quanpin::QuanpinEngine;
use crate::shuangpin::{ShuangpinEngine, ShuangpinProfile};
use crate::types::{CandidateSource, QueryRequest, SchemeSet, SchemeType, WordItem};

pub struct ProviderRegistry {
    /// 会话允许运行的方案；不在其中的方案，`activate` 拒绝。
    enabled: SchemeSet,
    /// 全拼在 `enabled` 里时才有。
    quanpin: Option<QuanpinEngine>,
    /// 双拼在 `enabled` 里时才有。
    shuangpin: Option<ShuangpinEngine>,
    keymap: Option<SharedKeymap>,
}

impl ProviderRegistry {
    pub fn new(
        enabled: SchemeSet,
        profile: &'static ShuangpinProfile,
        paths: &RuntimePaths,
    ) -> Self {
        Self {
            enabled,
            quanpin: enabled
                .contains(SchemeType::Quanpin)
                .then(|| QuanpinEngine::new(paths)),
            shuangpin: enabled
                .contains(SchemeType::Shuangpin)
                .then(|| ShuangpinEngine::new(profile, paths)),
            keymap: None,
        }
    }

    /// 不在 `enabled` 里的方案报 `INPUT_SCHEME_NOT_ENABLED`。拼音方案打开词库在构造 provider 时完成。
    pub fn activate(&mut self, scheme: SchemeType) -> Result<()> {
        if !self.enabled.contains(scheme) {
            return Err(EngineError::invalid(diagnostics::INPUT_SCHEME_NOT_ENABLED));
        }
        Ok(())
    }

    /// 会话允许运行的方案。
    pub fn enabled(&self) -> SchemeSet {
        self.enabled
    }

    pub fn set_helpcode_keymap(&mut self, keymap: Option<SharedKeymap>) {
        self.keymap = keymap;
        self.reset_pinyin_caches();
    }

    fn reset_pinyin_caches(&mut self) {
        if let Some(quanpin) = &mut self.quanpin {
            quanpin.reset_cache();
        }
        if let Some(shuangpin) = &mut self.shuangpin {
            shuangpin.reset_cache();
        }
    }

    /// 拼音行带上请求的方案。方案没有 provider（不在 `enabled` 里）时答空。
    pub fn query(&mut self, request: &QueryRequest) -> Vec<WordItem> {
        if !request.valid {
            return Vec::new();
        }
        let keymap = self.keymap.as_deref();
        let mut candidates = match request.scheme {
            SchemeType::Quanpin => match &mut self.quanpin {
                Some(quanpin) => quanpin.query(request, keymap),
                None => return Vec::new(),
            },
            SchemeType::Shuangpin => match &mut self.shuangpin {
                Some(shuangpin) => shuangpin.query(request, keymap),
                None => return Vec::new(),
            },
        };
        for item in &mut candidates {
            item.scheme = request.scheme;
        }
        candidates
    }

    pub fn find_candidate(&self, scheme: SchemeType, key: &str, value: &str) -> Option<WordItem> {
        match scheme {
            SchemeType::Quanpin => self.quanpin.as_ref()?.find_candidate(key, value),
            SchemeType::Shuangpin => self.shuangpin.as_ref()?.find_candidate(key, value),
        }
    }

    /// `scheme` 用的词库的粗俗词表（复制只是复制一个 `Arc`）。
    pub(crate) fn offensive_words(
        &self,
        scheme: SchemeType,
    ) -> crate::lattice::offensive::OffensiveWords {
        let table = if scheme == SchemeType::Shuangpin {
            self.shuangpin
                .as_ref()
                .map(ShuangpinEngine::offensive_words)
        } else {
            self.quanpin.as_ref().map(QuanpinEngine::offensive_words)
        };
        table.cloned().unwrap_or_default()
    }

    /// 全拼词典里每个键最好那一行的权重，供滑行输入使用；会话没有全拼 provider 时为空。
    pub fn quanpin_best_weights(&self, keys: &[String]) -> HashMap<String, i64> {
        self.quanpin
            .as_ref()
            .map_or_else(HashMap::new, |quanpin| quanpin.best_weights(keys))
    }

    /// 两种拼音方案都重置两个拼音引擎。
    pub fn reset_cache(&mut self, _scheme: SchemeType) {
        self.reset_pinyin_caches();
    }

    /// 在线候选只进全拼和双拼的序列缓存。
    pub fn cache_dynamic_candidates_for_request(
        &mut self,
        request: &QueryRequest,
        words: &[String],
        source: CandidateSource,
    ) -> bool {
        match request.scheme {
            SchemeType::Quanpin => self
                .quanpin
                .as_mut()
                .is_some_and(|quanpin| quanpin.insert_online_words(request, words, source)),
            SchemeType::Shuangpin => self
                .shuangpin
                .as_mut()
                .is_some_and(|shuangpin| shuangpin.insert_online_words(request, words, source)),
        }
    }

    /// Remove one online provider's rows from the pinyin caches.
    pub fn clear_online_candidates(&mut self, source: CandidateSource) {
        if let Some(quanpin) = &mut self.quanpin {
            quanpin.clear_online_candidates(source);
        }
        if let Some(shuangpin) = &mut self.shuangpin {
            shuangpin.clear_online_candidates(source);
        }
    }

    /// 拼音 provider 只答全拼和双拼请求。
    pub fn expand_initial_candidates(
        &mut self,
        request: &QueryRequest,
        candidates: &mut Vec<WordItem>,
    ) -> bool {
        match request.scheme {
            SchemeType::Quanpin => self
                .quanpin
                .as_mut()
                .is_some_and(|quanpin| quanpin.expand_initial_candidates(request, candidates)),
            SchemeType::Shuangpin => self
                .shuangpin
                .as_mut()
                .is_some_and(|shuangpin| shuangpin.expand_initial_candidates(request, candidates)),
        }
    }
}
