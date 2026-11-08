//! 上屏后联想经逐键打分器排序：只在挂了排一池仍在逐键预算之内的打分器（`scores_prediction_pool_within_keystroke_budget`）时多取候选池、排过之后仍是 8 行，座位就是引擎的下标，选中哪一行上屏的就是哪一行；不报这一条的打分器（随包模型、退到 6x256 的 ONNX 后端）挂着时联想与没挂时逐行相同。

use super::*;
use std::sync::Mutex;

/// 「吃」之后 11 条词库续写，按词频是 饭 好 喝 药 苦 亏 惊 力 素 醋 东西。
const POOL_DICTIONARY: &str = "CREATE TABLE tbl_1_c(key TEXT,jp TEXT,value TEXT,weight INTEGER);
INSERT INTO tbl_1_c VALUES('chi','c','吃',1000);
CREATE TABLE tbl_2_c(key TEXT,jp TEXT,value TEXT,weight INTEGER);
INSERT INTO tbl_2_c VALUES('chi''fan','cf','吃饭',100);
INSERT INTO tbl_2_c VALUES('chi''hao','ch','吃好',95);
INSERT INTO tbl_2_c VALUES('chi''he','ch','吃喝',90);
INSERT INTO tbl_2_c VALUES('chi''yao','cy','吃药',85);
INSERT INTO tbl_2_c VALUES('chi''ku','ck','吃苦',80);
INSERT INTO tbl_2_c VALUES('chi''kui','ck','吃亏',75);
INSERT INTO tbl_2_c VALUES('chi''jing','cj','吃惊',70);
INSERT INTO tbl_2_c VALUES('chi''li','cl','吃力',65);
INSERT INTO tbl_2_c VALUES('chi''su','cs','吃素',60);
INSERT INTO tbl_2_c VALUES('chi''cu','cc','吃醋',55);
CREATE TABLE tbl_3_c(key TEXT,jp TEXT,value TEXT,weight INTEGER);
INSERT INTO tbl_3_c VALUES('chi''dong''xi','cdx','吃东西',50);
CREATE TABLE tbl_1_w(key TEXT,jp TEXT,value TEXT,weight INTEGER);
INSERT INTO tbl_1_w VALUES('wo','w','我',1000);";

const BY_WEIGHT: [&str; 8] = ["饭", "好", "喝", "药", "苦", "亏", "惊", "力"];

type Calls = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// 逐字平均分照表给，表外 -5。按键重排也会调它，所以记下每一次的上文和候选，测试按候选认出联想的那一次。`fast` 是它报的 `scores_prediction_pool_within_keystroke_budget`。
struct Stub {
    means: Vec<(&'static str, f32)>,
    calls: Calls,
    fast: bool,
}

impl CandidateScorer for Stub {
    fn context_length(&self) -> usize {
        64
    }

    fn scores_prediction_pool_within_keystroke_budget(&self) -> bool {
        self.fast
    }

    fn log_probabilities(&mut self, context: &str, candidates: &[&str]) -> Vec<f32> {
        self.calls.lock().unwrap().push((
            context.to_owned(),
            candidates.iter().map(|text| (*text).to_owned()).collect(),
        ));
        candidates
            .iter()
            .map(|text| {
                self.means
                    .iter()
                    .find(|(row, _)| row == text)
                    .map_or(-5.0, |(_, mean)| *mean)
            })
            .collect()
    }
}

/// 在逐键预算之内、会排联想的打分器，像过了自测预算的 ONNX 大模型。
fn stub(means: &[(&'static str, f32)]) -> (Box<dyn CandidateScorer>, Calls) {
    stub_with_speed(means, true)
}

/// 不在预算之内、只做逐键重排的打分器，像随包模型或退到 6x256 的 ONNX 后端。
fn slow_stub(means: &[(&'static str, f32)]) -> (Box<dyn CandidateScorer>, Calls) {
    stub_with_speed(means, false)
}

fn stub_with_speed(means: &[(&'static str, f32)], fast: bool) -> (Box<dyn CandidateScorer>, Calls) {
    let calls = Calls::default();
    (
        Box::new(Stub {
            means: means.to_vec(),
            calls: calls.clone(),
            fast,
        }),
        calls,
    )
}

/// 联想那一次打分（候选里有 东西）读到的上文。
fn prediction_contexts(calls: &Calls) -> Vec<String> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, candidates)| candidates.iter().any(|text| text == "东西"))
        .map(|(context, _)| context.clone())
        .collect()
}

fn type_keys(runtime: &mut Runtime, keys: &[u8]) {
    for value in keys {
        runtime
            .dispatch(Action::Character {
                value: *value,
                shift: false,
            })
            .unwrap();
    }
}

fn select_text(runtime: &mut Runtime, text: &str) -> Transition {
    let id = runtime
        .view()
        .candidates
        .iter()
        .find(|row| row.text == text)
        .unwrap_or_else(|| panic!("{text} is not among the candidates"))
        .id;
    runtime.dispatch(Action::Select(id)).unwrap()
}

fn commit_chi(runtime: &mut Runtime) {
    type_keys(runtime, b"chi");
    assert_eq!(select_text(runtime, "吃").commit.as_deref(), Some("吃"));
    assert!(runtime.view().prediction);
}

fn texts(runtime: &Runtime) -> Vec<String> {
    runtime
        .view()
        .candidates
        .iter()
        .map(|row| row.text.clone())
        .collect()
}

/// 联想的每一行连同编码、附注和来源，供逐行比较。
fn rows(runtime: &Runtime) -> Vec<(String, String, String, u8)> {
    runtime
        .view()
        .candidates
        .iter()
        .map(|row| {
            (
                row.text.clone(),
                row.code.clone(),
                row.annotation.clone(),
                row.source,
            )
        })
        .collect()
}

/// 同一个随包模型，只是报自己够快：对照用，说明下面那一条不排是速度的门禁，不是模型给不出顺序。
struct FastBundled(Reranker);

impl CandidateScorer for FastBundled {
    fn context_length(&self) -> usize {
        CandidateScorer::context_length(&self.0)
    }

    fn log_probabilities(&mut self, context: &str, candidates: &[&str]) -> Vec<f32> {
        self.0.log_probabilities(context, candidates)
    }

    fn scores_prediction_pool_within_keystroke_budget(&self) -> bool {
        true
    }
}

