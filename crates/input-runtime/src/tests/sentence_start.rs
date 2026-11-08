//! 一句话开头的语气词：没有组字时按的回车记成上文里的换行，开头单打 `ma`、`ba` 时句尾语气词「吗」「吧」让给后面的单字；句中照常。

use super::*;

/// 「吗」的词频远高于「妈」，「吧」略高于「爸」，与锁定词库里的先后相同。
const PARTICLE_DICTIONARY: &str =
    "CREATE TABLE tbl_1_m(key TEXT,jp TEXT,value TEXT,weight INTEGER);
INSERT INTO tbl_1_m VALUES('ma','m','吗',5664547);
INSERT INTO tbl_1_m VALUES('ma','m','妈',110515);
CREATE TABLE tbl_1_b(key TEXT,jp TEXT,value TEXT,weight INTEGER);
INSERT INTO tbl_1_b VALUES('ba','b','吧',8425271);
INSERT INTO tbl_1_b VALUES('ba','b','爸',6909598);
CREATE TABLE tbl_1_a(key TEXT,jp TEXT,value TEXT,weight INTEGER);
INSERT INTO tbl_1_a VALUES('a','a','啊',3000000);
INSERT INTO tbl_1_a VALUES('a','a','阿',1000000);";

fn first_after_typing(runtime: &mut Runtime, keys: &[u8]) -> String {
    for value in keys {
        runtime
            .dispatch(Action::Character {
                value: *value,
                shift: false,
            })
            .unwrap();
    }
    let first = runtime.all_candidates().candidates[0].text.clone();
    runtime.dispatch(Action::Command(Command::Cancel)).unwrap();
    first
}

/// 给每段文字一个固定分数的打分器，没列出的文字一律 -20。
struct FixedScores(&'static [(&'static str, f32)]);

impl CandidateScorer for FixedScores {
    fn context_length(&self) -> usize {
        64
    }

    fn log_probabilities(&mut self, _context: &str, candidates: &[&str]) -> Vec<f32> {
        candidates
            .iter()
            .map(|text| {
                self.0
                    .iter()
                    .find(|(known, _)| known == text)
                    .map_or(-20.0, |(_, score)| *score)
            })
            .collect()
    }
}

