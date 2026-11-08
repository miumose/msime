use super::{append_canonical_pinyin, CreatingWordProgress, InputSession, SelectionTransition};
use crate::ime::personal_rerank::allocations::{count, measure};
use crate::pinyin::segment::is_complete_pinyin_input;
use crate::text::count_han_chars;

// 冻结 `f2c7d47dad0e56208cf0fa532c4a77ea0db6e859` 的完整校验和造词进度，仅适配函数名、旧调用和中文注释。
fn original_normalize_canonical_pinyin_for_word(pinyin: &str, word: &str) -> String {
    if pinyin.is_empty() {
        return String::new();
    }
    let segment_count = pinyin.bytes().filter(|&byte| byte == b'\'').count() + 1;
    if segment_count != count_han_chars(word) {
        return String::new();
    }
    if pinyin
        .split('\'')
        .any(|segment| segment.is_empty() || !is_complete_pinyin_input(segment))
    {
        return String::new();
    }
    pinyin.to_owned()
}

fn original_update_creating_word_progress(
    current_pinyin: &str,
    current_word: &str,
    selected_word: &str,
    transition: &SelectionTransition,
) -> CreatingWordProgress {
    let mut word = String::with_capacity(current_word.len() + selected_word.len());
    word.push_str(current_word);
    word.push_str(selected_word);
    let selected_canonical = original_normalize_canonical_pinyin_for_word(
        &transition.selected_canonical_pinyin,
        selected_word,
    );
    let prior_parts_storable = current_word.is_empty() || !current_pinyin.is_empty();
    let pinyin = if prior_parts_storable && !selected_canonical.is_empty() {
        append_canonical_pinyin(current_pinyin, &selected_canonical)
    } else {
        String::new()
    };
    let completed = !transition.continues_composition;
    let can_store =
        completed && !original_normalize_canonical_pinyin_for_word(&pinyin, &word).is_empty();
    let mut preedit =
        String::with_capacity(word.len() + transition.current_segmentation_with_cases.len());
    preedit.push_str(&word);
    preedit.push_str(&transition.current_segmentation_with_cases);
    CreatingWordProgress {
        preedit,
        pinyin,
        word,
        completed,
        can_store,
    }
}

fn assert_same(new: &CreatingWordProgress, old: &CreatingWordProgress) {
    assert_eq!(new, old);
    assert_eq!(new.pinyin.capacity(), old.pinyin.capacity());
    assert_eq!(new.word.capacity(), old.word.capacity());
    assert_eq!(new.preedit.capacity(), old.preedit.capacity());
}

fn saved_outputs(current: &CreatingWordProgress, selected: &str, word: &str, wubi: bool) -> usize {
    if wubi {
        return 0;
    }
    usize::from(!original_normalize_canonical_pinyin_for_word(selected, word).is_empty())
        + usize::from(current.can_store)
}

#[test]
fn creating_word_progress_does_not_copy_readings_for_validation() {
    assert!(is_complete_pinyin_input("ni"));
    let transition = SelectionTransition {
        selected_canonical_pinyin: "ni'hao".into(),
        ..SelectionTransition::default()
    };
    let (old, old_count) =
        count(|| original_update_creating_word_progress("", "", "合成", &transition));
    let (new, new_count) =
        count(|| InputSession::update_creating_word_progress("", "", "合成", &transition));
    assert_same(&new, &old);
    assert!(new.completed && new.can_store);
    eprintln!("progress old={old_count} new={new_count}");
    assert_eq!(new_count + 2, old_count);
    assert_eq!(new_count, 3);
}

