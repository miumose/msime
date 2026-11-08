//! 延后到停顿的逐键打分器（`Runtime::set_deferred_scorer`）：按键时不挂，引擎照没有模型时那样解码；停顿时 `settle_deferred_scorer` 挂上它重解一遍，一次只花一片模型时间，没解完的那一片不改屏幕上的候选，打过的分留在引擎的备忘里接着用。

use super::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// ta 三个字、shuo 三个字：九种读法按两字词频之积排，没有模型时第九条 `它朔` 不在候选里（见 `sentence_pool.rs`）。
const POOL_DICTIONARY: &str = "CREATE TABLE tbl_1_t(key TEXT,jp TEXT,value TEXT,weight INTEGER);
INSERT INTO tbl_1_t VALUES('ta','t','他',1000);
INSERT INTO tbl_1_t VALUES('ta','t','她',900);
INSERT INTO tbl_1_t VALUES('ta','t','它',800);
CREATE TABLE tbl_1_s(key TEXT,jp TEXT,value TEXT,weight INTEGER);
INSERT INTO tbl_1_s VALUES('shuo','s','说',1000);
INSERT INTO tbl_1_s VALUES('shuo','s','硕',100);
INSERT INTO tbl_1_s VALUES('shuo','s','朔',50);";

const NINTH: &str = "它朔";

/// 束搜索里的模型：只给 `NINTH` 和它的前缀高分；记下每一次被要求打分的文字，每次调用先睡 `delay`，好让一片的时限在测试里也会用完。
struct Recording {
    delay: Duration,
    scored: Mutex<Vec<String>>,
}

impl msime_engine::ordering::SentenceScorer for Recording {
    fn log_probabilities(&self, _context: &str, candidates: &[&str]) -> Vec<f32> {
        std::thread::sleep(self.delay);
        self.scored
            .lock()
            .unwrap()
            .extend(candidates.iter().map(|text| (*text).to_owned()));
        candidates
            .iter()
            .map(|text| if NINTH.starts_with(text) { 0.0 } else { -5.0 })
            .collect()
    }
}

/// 把 `Recording` 交给引擎的逐键打分器；自己的重排分对谁都一样，顺序只由束搜索决定。
struct Lends(Arc<Recording>);

impl CandidateScorer for Lends {
    fn context_length(&self) -> usize {
        64
    }

    fn log_probabilities(&mut self, _context: &str, candidates: &[&str]) -> Vec<f32> {
        vec![-5.0; candidates.len()]
    }

    fn sentence_scorer(&self) -> Option<Arc<dyn msime_engine::ordering::SentenceScorer>> {
        Some(self.0.clone())
    }
}

fn type_tashuo(runtime: &mut Runtime) {
    for value in b"tashuo" {
        runtime
            .dispatch(Action::Character {
                value: *value,
                shift: false,
            })
            .unwrap();
    }
}

fn texts(runtime: &mut Runtime) -> Vec<String> {
    runtime
        .all_candidates()
        .candidates
        .into_iter()
        .map(|row| row.text)
        .collect()
}

fn first(runtime: &mut Runtime) -> String {
    runtime.all_candidates().candidates[0].text.clone()
}

/// 先给若干次有效评分，之后模拟模型运行失败；恢复开关用于核对空分没有被当成有效分记住。
struct EmptyAfter {
    recording: Arc<Recording>,
    valid_calls: usize,
    calls: std::sync::atomic::AtomicUsize,
    recovered: std::sync::atomic::AtomicBool,
}

impl msime_engine::ordering::SentenceScorer for EmptyAfter {
    fn log_probabilities(&self, context: &str, candidates: &[&str]) -> Vec<f32> {
        let call = self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if call < self.valid_calls || self.recovered.load(std::sync::atomic::Ordering::Relaxed) {
            self.recording.log_probabilities(context, candidates)
        } else {
            Vec::new()
        }
    }
}

struct LendsEmpty(Arc<EmptyAfter>);

impl CandidateScorer for LendsEmpty {
    fn context_length(&self) -> usize {
        64
    }

    fn log_probabilities(&mut self, _context: &str, candidates: &[&str]) -> Vec<f32> {
        vec![-5.0; candidates.len()]
    }

    fn sentence_scorer(&self) -> Option<Arc<dyn msime_engine::ordering::SentenceScorer>> {
        Some(self.0.clone())
    }
}

fn empty_scorer(recording: Arc<Recording>, valid_calls: usize) -> Arc<EmptyAfter> {
    Arc::new(EmptyAfter {
        recording,
        valid_calls,
        calls: std::sync::atomic::AtomicUsize::new(0),
        recovered: std::sync::atomic::AtomicBool::new(false),
    })
}

