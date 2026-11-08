//! 光标后编辑经过 runtime 的部分：暂存交出去的文字、视图里的两个字段、候选身份、不重排首选。真实 Engine，合成词库。

use super::*;

const FIXTURE: &str = "CREATE TABLE tbl_1_w(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_1_w VALUES('wo','w','我',1000);\
CREATE TABLE tbl_1_h(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_1_h VALUES('hao','h','好',1000),('huo','h','火',300);\
CREATE TABLE tbl_1_x(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_1_x VALUES('xian','x','现',1000);\
CREATE TABLE tbl_1_c(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_1_c VALUES('chi','c','吃',1000),('chi','c','痴',200),('chi','c','尺',100);\
CREATE TABLE tbl_2_c(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_2_c VALUES('chi''huo','ch','吃货',500),('chi''huo','ch','痴货',300),('chi''huo','ch','尺蠖',200);\
CREATE TABLE tbl_1_g(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_1_g VALUES('guo','g','锅',1000),('guo','g','国',500),('guo','g','过',300);\
CREATE TABLE tbl_2_h(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_2_h VALUES('huo''guo','hg','火锅',5000);";

const TYPED: &str = "wohaoxianchihuoguo";

fn id_of(view: &View, text: &str) -> CandidateId {
    view.candidates
        .iter()
        .find(|row| row.text == text)
        .unwrap_or_else(|| panic!("{text} is not on the page: {:?}", texts(view)))
        .id
}

fn at_caret(runtime: &mut Runtime, caret: usize) -> View {
    type_characters(runtime, TYPED);
    runtime.set_caret(Some(caret)).unwrap();
    runtime.view()
}

/// 首选接着固定的那段，重排模型不能把别的行提到它前面。
#[test]
fn the_reranker_leaves_the_caret_edit_list_in_engine_order() {
    let typed = |caret_edit_suffix: bool| {
        let mut runtime = Runtime::new(
            WubiMixedEngine {
                scheme: 0,
                answered_by_pinyin_fallback: false,
                reading: String::new(),
                caret_edit_suffix,
            },
            5,
        )
        .unwrap();
        let model = favouring_model(&['态', '太', '快', '顿'], &['太', '快']);
        runtime.set_reranker(Some(Reranker::new(std::sync::Arc::new(model))));
        runtime.focus(true).unwrap();
        for value in *b"dyn" {
            runtime
                .dispatch(Action::Character {
                    value,
                    shift: false,
                })
                .unwrap();
        }
        texts(&runtime.view())
    };
    assert_eq!(typed(false), ["太快", "态", "顿"]);
    assert_eq!(typed(true), ["态", "太快", "顿"]);
}
