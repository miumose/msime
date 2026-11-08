//! 双拼组字末尾按 `/` 转超级简拼（#10860）经过 runtime 的部分：`/` 走标点入口也是转换，不先上屏首选；第二个 `/` 撤回转换，从转换前宿主高亮的那一行上屏再出 `/`，退格撤回时高亮也回到那一行，与以前单按 `/` 相同。真实 Engine，合成词库。

use super::*;

const FIXTURE: &str = "CREATE TABLE tbl_3_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_3_n VALUES('ni''shi''shui','nss','你是谁',1000),('na''se''su','nss','那色素',900);\
CREATE TABLE tbl_1_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_1_n VALUES('nu','n','怒',500);\
CREATE TABLE tbl_2_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_2_n VALUES('nu''shi','ns','怒视',900),('nu''shi','ns','努实',800);\
CREATE TABLE tbl_1_s(key TEXT,jp TEXT,value TEXT,weight INTEGER);\
INSERT INTO tbl_1_s VALUES('shi','s','是',1000);";

fn highlighted_seat(view: &View) -> Option<usize> {
    view.candidates
        .iter()
        .position(|candidate| candidate.highlighted)
}

/// 打 `nuu` 并把高亮移到第二行，返回首行和第二行的文字。
fn type_and_highlight_second(runtime: &mut Runtime) -> (String, String) {
    for value in *b"nuu" {
        character(runtime, value);
    }
    assert!(
        runtime.view().candidates.len() > 1,
        "{:?}",
        runtime.view().candidates
    );
    let moved = runtime.dispatch(Action::NextCandidate).unwrap();
    assert_eq!(highlighted_seat(&moved.view), Some(1));
    (
        moved.view.candidates[0].text.clone(),
        moved.view.candidates[1].text.clone(),
    )
}

