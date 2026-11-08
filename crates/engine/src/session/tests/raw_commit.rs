//! 回车原样上屏时学英文词的核心选项（`learn_english_on_enter`、`learn_typed_spellings_on_enter`）与 `Command::CommitRawWithoutLearning`。产品取值下的完整策略由 `host/tests.rs` 经门面钉住；这里钉住引擎库的默认值和选项本身。

use super::*;

fn learned(fixture: &Fixture, word: &str) -> i64 {
    count(
        &fixture.path().join(assets::ENGLISH_DICTIONARY),
        &format!("SELECT count(*) FROM english_words WHERE word='{word}'"),
    )
}

fn english_fixture() -> Fixture {
    Fixture::new(QUANPIN_FIXTURE).with_english(ENGLISH_FIXTURE)
}

/// 引擎库默认只在专用英文里学回车上屏的词：中文组字里拼不成音节的字母原样上屏，不进英文词库。
#[test]
fn by_default_enter_learns_only_dedicated_english() {
    let fixture = english_fixture();
    let mut session = fixture.session();
    assert!(!SessionOptions::new(fixture.paths()).learn_typed_spellings_on_enter);

    type_text(&mut session, "rustacean");
    let result = session.command(Command::CommitRaw);
    assert_eq!(result.commit.as_deref(), Some("rustacean"));
    assert_eq!(result.diagnostic, None);
    assert_eq!(learned(&fixture, "rustacean"), 0);

    session.set_dedicated_english(true);
    type_text(&mut session, "Zyzzyva");
    let result = session.command(Command::CommitRaw);
    assert_eq!(result.commit.as_deref(), Some("Zyzzyva"));
    assert_eq!(learned(&fixture, "zyzzyva"), 1);
}

/// 打开 `learn_typed_spellings_on_enter` 后，拼不成完整音节的拼音和临时英文里的字母也学成英文词（临时英文去掉入口字母 `Y`）；完整的拼音不学；总开关 `learn_english_on_enter` 关着时什么也不学。
#[test]
fn typed_spellings_are_learned_when_the_option_is_on() {
    let fixture = english_fixture();
    let mut session = fixture.session_with(|options| options.learn_typed_spellings_on_enter = true);

    type_text(&mut session, "rustacean");
    let result = session.command(Command::CommitRaw);
    assert_eq!(result.commit.as_deref(), Some("rustacean"));
    assert_eq!(result.diagnostic, None);
    assert_eq!(learned(&fixture, "rustacean"), 1);
    assert_eq!(
        count(
            &fixture.journal(),
            "SELECT count(*) FROM user_dictionary_operations WHERE dictionary='english' AND key='rustacean'"
        ),
        1
    );

    type_text(&mut session, "nihao");
    assert_eq!(
        session.command(Command::CommitRaw).commit.as_deref(),
        Some("nihao")
    );
    assert_eq!(learned(&fixture, "nihao"), 0);

    assert!(session.character(b'Y', true).handled);
    assert_eq!(
        session.snapshot().local_mode,
        LocalInputMode::TemporaryEnglish
    );
    type_text(&mut session, "ferris");
    assert_eq!(
        session.command(Command::CommitRaw).commit.as_deref(),
        Some("Yferris")
    );
    assert_eq!(learned(&fixture, "ferris"), 1);
    assert_eq!(learned(&fixture, "yferris"), 0);

    let mut off = fixture.session_with(|options| {
        options.learn_typed_spellings_on_enter = true;
        options.learn_english_on_enter = false;
    });
    type_text(&mut off, "crustacean");
    assert_eq!(
        off.command(Command::CommitRaw).commit.as_deref(),
        Some("crustacean")
    );
    assert_eq!(learned(&fixture, "crustacean"), 0);
}

/// `CommitRawWithoutLearning` 上屏同样的原文、不学：专用英文里取预编辑原文并结束组字，中文组字里就是不带学词策略的 `CommitRaw`。
#[test]
fn commit_raw_without_learning_commits_the_same_text_and_learns_nothing() {
    let fixture = english_fixture();
    let mut session = fixture.session_with(|options| options.learn_typed_spellings_on_enter = true);

    type_text(&mut session, "rustacean");
    let result = session.command(Command::CommitRawWithoutLearning);
    assert_eq!(result.commit.as_deref(), Some("rustacean"));
    assert!(session.snapshot().preedit.is_empty());
    assert_eq!(learned(&fixture, "rustacean"), 0);

    session.set_dedicated_english(true);
    type_text(&mut session, "Zyzzyva");
    let before = session.candidate_revision();
    let result = session.command(Command::CommitRawWithoutLearning);
    assert!(session.candidate_revision() > before);
    assert_eq!(result.commit.as_deref(), Some("Zyzzyva"));
    assert!(session.snapshot().preedit.is_empty());
    assert_eq!(learned(&fixture, "zyzzyva"), 0);

    assert_eq!(
        Command::from_u8(14),
        Some(Command::CommitRawWithoutLearning)
    );
    assert_eq!(
        Command::from_name("CommitRawWithoutLearning"),
        Some(Command::CommitRawWithoutLearning)
    );
}

