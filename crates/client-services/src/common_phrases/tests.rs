use super::*;
use crate::community::resource::{CommunityResourceContent, SharedPhrase};
use std::fs;
use std::process::Command;

fn fresh() -> (tempfile::TempDir, CommonPhrasesStore) {
    let root = tempfile::tempdir().unwrap();
    let store = CommonPhrasesStore::new(root.path().join("preferences"));
    (root, store)
}

#[test]
fn categories_keep_old_documents_and_default_expansion() {
    let old: CommonPhrases =
        serde_json::from_value(serde_json::json!({"phrases": [], "packs": []})).unwrap();
    assert!(old.categories.is_empty());
    assert_eq!(old.expanded_category, "all");
    assert_eq!(
        serde_json::to_value(old).unwrap(),
        serde_json::json!({"phrases": [], "packs": []})
    );
}

#[test]
fn categories_validate_names_limits_and_keep_failed_writes_unchanged() {
    let (_root, store) = fresh();
    let first = store.add_category("  示例分类  ").unwrap();
    assert_eq!(first.categories[0].name, "示例分类");
    for name in [
        "",
        "\t",
        "全部",
        "未分类",
        "不展开",
        "a\nb",
        "a\0b",
        "过长分类名称超过二十个字就不能够再继续保存下去了",
    ] {
        assert!(matches!(
            store.add_category(name),
            Err(CommonPhrasesError::CategoryInvalid)
        ));
        assert_eq!(store.load().unwrap(), first);
    }
    assert!(matches!(
        store.add_category("示例分类"),
        Err(CommonPhrasesError::CategoryDuplicate)
    ));
    store.add_category(&"𠀀".repeat(10)).unwrap();
    assert!(matches!(
        store.add_category(&"𠀀".repeat(11)),
        Err(CommonPhrasesError::CategoryInvalid)
    ));
    for index in 2..MAX_CATEGORIES {
        store.add_category(&format!("合成分类{index}")).unwrap();
    }
    let full = store.load().unwrap();
    assert!(matches!(
        store.add_category("第七类"),
        Err(CommonPhrasesError::CategoryLimit)
    ));
    assert_eq!(store.load().unwrap(), full);
}

#[test]
fn category_rename_and_removal_keep_phrase_content_and_reset_default() {
    let (_root, store) = fresh();
    let phrase = store
        .add_with_note("合成正文\n第二行", "合成备注")
        .unwrap()
        .phrases[0]
        .clone();
    let category = store.add_category("待改名").unwrap().categories[0].id;
    store.categorize(phrase.id, Some(category)).unwrap();
    store.set_expanded_category(&category.to_string()).unwrap();
    store.rename_category(category, "已改名").unwrap();
    let reloaded = CommonPhrasesStore::new(store.file.parent().unwrap())
        .load()
        .unwrap();
    assert_eq!(reloaded.categories[0].name, "已改名");
    assert_eq!(reloaded.phrases[0].category, Some(category));
    assert_eq!(reloaded.expanded_category, category.to_string());
    let removed = store.remove_category(category).unwrap();
    assert_eq!(removed.phrases, vec![phrase]);
    assert_eq!(removed.expanded_category, "all");
    assert!(removed.categories.is_empty());
}

#[test]
fn category_changes_reject_unknown_ids_and_allow_collapsed_default() {
    let (_root, store) = fresh();
    let document = store.add("合成正文").unwrap();
    let phrase = document.phrases[0].id;
    assert!(matches!(
        store.categorize(phrase, Some(Uuid::new_v4())),
        Err(CommonPhrasesError::NotFound)
    ));
    assert!(matches!(
        store.set_expanded_category("unknown"),
        Err(CommonPhrasesError::NotFound)
    ));
    assert_eq!(store.load().unwrap(), document);
    assert!(store
        .set_expanded_category("")
        .unwrap()
        .expanded_category
        .is_empty());
    assert_eq!(
        store
            .set_expanded_category("all")
            .unwrap()
            .expanded_category,
        "all"
    );
}

#[test]
fn corrupt_category_references_leave_the_file_unchanged() {
    let (_root, store) = fresh();
    let document = store.add("合成正文").unwrap();
    let mut invalid = serde_json::to_value(document).unwrap();
    invalid["phrases"][0]["category"] = serde_json::json!(Uuid::new_v4());
    let bytes = serde_json::to_vec(&invalid).unwrap();
    fs::write(&store.file, &bytes).unwrap();
    assert!(matches!(
        store.add_category("合成分类"),
        Err(CommonPhrasesError::Corrupt)
    ));
    assert_eq!(fs::read(&store.file).unwrap(), bytes);
}

#[test]
fn category_backup_merges_names_preserves_existing_assignments_and_defaults() {
    let (_root, store) = fresh();
    let old = store.add("已有合成正文").unwrap().phrases[0].id;
    let category = store.add_category("本机分类").unwrap().categories[0].id;
    store.categorize(old, Some(category)).unwrap();
    store.set_expanded_category("").unwrap();
    let new = store.add("恢复的合成正文").unwrap().phrases[1].id;
    let imported = store
        .import_categories(
            &[
                CommonPhraseCategoryBackup {
                    id: Uuid::from_u128(1),
                    name: "本机分类".into(),
                    texts: vec!["恢复的合成正文".into()],
                },
                CommonPhraseCategoryBackup {
                    id: Uuid::from_u128(2),
                    name: "备份分类".into(),
                    texts: vec!["已有合成正文".into()],
                },
            ],
            &Uuid::from_u128(2).to_string(),
            &[new],
        )
        .unwrap();
    assert_eq!(imported.categories.len(), 2);
    assert_eq!(imported.phrases[0].category, Some(category));
    assert_eq!(imported.phrases[1].category, Some(category));
    assert!(imported.expanded_category.is_empty());
}

#[test]
fn category_backup_adopts_default_on_empty_device_and_rejects_partial_imports() {
    let (_root, store) = fresh();
    let phrase = store.add("合成备份正文").unwrap().phrases[0].id;
    let category = CommonPhraseCategoryBackup {
        id: Uuid::from_u128(2),
        name: "all".into(),
        texts: vec!["合成备份正文".into()],
    };
    let imported = store
        .import_categories(
            std::slice::from_ref(&category),
            &category.id.to_string(),
            &[phrase],
        )
        .unwrap();
    assert_eq!(imported.categories[0].name, "all");
    assert_eq!(
        imported.expanded_category,
        imported.categories[0].id.to_string()
    );
    assert_eq!(
        imported.phrases[0].category,
        Some(imported.categories[0].id)
    );
    let invalid = CommonPhraseCategoryBackup {
        id: Uuid::from_u128(3),
        name: "另一分类".into(),
        texts: vec!["合成备份正文".into()],
    };
    assert!(store
        .import_categories(&[category, invalid], "all", &[phrase])
        .is_err());
    assert_eq!(store.load().unwrap(), imported);
    for index in 1..MAX_CATEGORIES {
        store.add_category(&format!("本机分类{index}")).unwrap();
    }
    let full = store.load().unwrap();
    assert!(matches!(
        store.import_categories(
            &[CommonPhraseCategoryBackup {
                id: Uuid::from_u128(4),
                name: "第七分类".into(),
                texts: vec![]
            }],
            "all",
            &[]
        ),
        Err(CommonPhrasesError::CategoryLimit)
    ));
    assert_eq!(store.load().unwrap(), full);
}

fn pack(id: u128, revision: u32, texts: &[&str]) -> CommunityResource {
    CommunityResource {
        id: Uuid::from_u128(id),
        kind: CommunityResourceKind::Phrase,
        name: "邮件签名".into(),
        description: "示例短语包".into(),
        author: "示例作者".into(),
        content: CommunityResourceContent {
            phrases: texts
                .iter()
                .map(|text| SharedPhrase {
                    text: (*text).into(),
                    group: String::new(),
                })
                .collect(),
            ..Default::default()
        },
        revision,
        saves: 0,
        saved: false,
        owned: false,
        rating_count: 0,
        rating_average: 0.0,
        my_rating: 0,
        moderation: None,
    }
}

fn texts(document: &CommonPhrases) -> Vec<&str> {
    document
        .phrases
        .iter()
        .map(|phrase| phrase.text.as_str())
        .collect()
}

#[cfg(unix)]
#[test]
fn common_phrase_writes_stay_bound_to_the_locked_directory_after_replacement() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("preferences");
    fs::create_dir(&directory).unwrap();
    let store = CommonPhrasesStore::new(&directory);
    let lock = store.lock().unwrap();

    let moved = root.path().join("preferences-moved");
    fs::rename(&directory, &moved).unwrap();
    fs::create_dir(&directory).unwrap();

    store
        .write_locked(&lock, &CommonPhrases::default())
        .unwrap();

    assert!(moved.join(FILE_NAME).exists());
    assert!(!directory.join(FILE_NAME).exists());
    fs::remove_dir_all(moved).unwrap();
}

#[test]
fn common_phrases_start_empty_and_keep_multiline_text() {
    let (_root, store) = fresh();
    assert_eq!(store.load().unwrap(), CommonPhrases::default());
    let document = store.add("此致\n敬礼").unwrap();
    assert_eq!(texts(&document), ["此致\n敬礼"]);
    assert_eq!(document.phrases[0].pack, None);
    assert_eq!(store.load().unwrap(), document);
    // 文件里 `pack` 明确写成 null，和契约的模型一致。
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(&store.file).unwrap()).unwrap();
    assert_eq!(raw["phrases"][0]["pack"], serde_json::Value::Null);
    assert_eq!(raw["packs"], serde_json::json!([]));
}

#[test]
fn common_phrases_text_is_bounded_in_utf16_and_allows_only_line_feeds() {
    let (_root, store) = fresh();
    assert!(store.add(&"字".repeat(MAX_PHRASE_UTF16)).is_ok());
    assert!(matches!(
        store.add(&"字".repeat(MAX_PHRASE_UTF16 + 1)),
        Err(CommonPhrasesError::Invalid)
    ));
    // 补充平面的字占两个 UTF-16 单元：500 个正好到上限，501 个超出。
    assert!(store.add(&"𠀀".repeat(500)).is_ok());
    assert!(matches!(
        store.add(&"𠀀".repeat(501)),
        Err(CommonPhrasesError::Invalid)
    ));
    for invalid in ["", "  \n ", "制表\t符", "回车\r换行", "空\u{0}字符"] {
        assert!(
            matches!(store.add(invalid), Err(CommonPhrasesError::Invalid)),
            "{invalid:?}"
        );
    }
    assert_eq!(store.load().unwrap().phrases.len(), 2);
}



#[test]
fn common_phrases_replace_remove_and_move_by_id() {
    let (_root, store) = fresh();
    store.add("一").unwrap();
    store.add("二").unwrap();
    let document = store.add("三").unwrap();
    let ids: Vec<_> = document.phrases.iter().map(|phrase| phrase.id).collect();

    let document = store.move_to(ids[2], 0).unwrap();
    assert_eq!(texts(&document), ["三", "一", "二"]);
    let document = store.move_to(ids[2], 2).unwrap();
    assert_eq!(texts(&document), ["一", "二", "三"]);
    assert!(matches!(
        store.move_to(ids[0], 3),
        Err(CommonPhrasesError::Invalid)
    ));

    let document = store.replace(ids[1], "贰\n第二行").unwrap();
    assert_eq!(texts(&document), ["一", "贰\n第二行", "三"]);
    assert_eq!(document.phrases[1].id, ids[1]);

    let document = store.remove(ids[0]).unwrap();
    assert_eq!(texts(&document), ["贰\n第二行", "三"]);
    assert!(matches!(
        store.remove(ids[0]),
        Err(CommonPhrasesError::NotFound)
    ));
    assert!(matches!(
        store.replace(Uuid::new_v4(), "四"),
        Err(CommonPhrasesError::NotFound)
    ));
}



#[test]
fn invalid_local_pack_phrases_do_not_reserve_normalization_capacity() {
    let long = "字".repeat(MAX_PHRASE_UTF16 + 1);
    let resource = pack(1, 1, &[&long]);

    let (normalized, skipped) = super::normalized_pack_texts(&resource);

    assert!(normalized.is_empty());
    assert_eq!(normalized.capacity(), 0);
    assert_eq!(skipped, 1);
}


#[test]
fn common_phrases_corrupt_file_is_reported_and_left_alone() {
    let (_root, store) = fresh();
    store.add("保留").unwrap();
    for corrupt in [
        b"{not json".to_vec(),
        // 条目引用了没有安装的包。
        serde_json::to_vec(&serde_json::json!({
            "phrases": [{"id": Uuid::from_u128(9), "text": "孤儿", "pack": Uuid::from_u128(7)}],
            "packs": []
        }))
        .unwrap(),
        // 重复的 id。
        serde_json::to_vec(&serde_json::json!({
            "phrases": [
                {"id": Uuid::from_u128(9), "text": "一", "pack": null},
                {"id": Uuid::from_u128(9), "text": "二", "pack": null}
            ],
            "packs": []
        }))
        .unwrap(),
        // 不认识的字段。
        serde_json::to_vec(&serde_json::json!({"phrases": [], "packs": [], "extra": 1})).unwrap(),
    ] {
        fs::write(&store.file, &corrupt).unwrap();
        assert!(matches!(store.load(), Err(CommonPhrasesError::Corrupt)));
        assert!(matches!(
            store.add("新的"),
            Err(CommonPhrasesError::Corrupt)
        ));
        assert_eq!(fs::read(&store.file).unwrap(), corrupt);
    }
    let oversized = vec![b' '; MAX_FILE_BYTES as usize + 1];
    fs::write(&store.file, &oversized).unwrap();
    assert!(matches!(store.load(), Err(CommonPhrasesError::Corrupt)));
    assert_eq!(
        fs::metadata(&store.file).unwrap().len(),
        oversized.len() as u64
    );
}


#[cfg(unix)]
#[test]
fn common_phrases_refuse_a_symlinked_file() {
    use std::os::unix::fs::symlink;

    let (root, store) = fresh();
    fs::create_dir_all(root.path().join("preferences")).unwrap();
    let outside = root.path().join("outside.json");
    fs::write(&outside, b"{\"phrases\":[],\"packs\":[]}").unwrap();
    symlink(&outside, &store.file).unwrap();
    assert!(store.load().is_err());
    assert!(store.add("不该写到外面").is_err());
    assert_eq!(
        fs::read(&outside).unwrap(),
        b"{\"phrases\":[],\"packs\":[]}"
    );
}

const CHILD_DIRECTORY: &str = "MSIME_COMMON_PHRASES_CHILD_DIRECTORY";
const CHILD_PREFIX: &str = "MSIME_COMMON_PHRASES_CHILD_PREFIX";
const CHILD_ADDS: usize = 40;

/// 并发测试的子进程入口：只有父测试设了环境变量时才做事，单独运行时什么也不做。
#[test]
fn common_phrases_child_process_adds() {
    let (Ok(directory), Ok(prefix)) = (std::env::var(CHILD_DIRECTORY), std::env::var(CHILD_PREFIX))
    else {
        return;
    };
    let store = CommonPhrasesStore::new(directory);
    for index in 0..CHILD_ADDS {
        store.add(&format!("{prefix}{index}")).unwrap();
    }
}

#[test]
fn common_phrases_concurrent_adds_from_two_processes_are_all_kept() {
    let (root, store) = fresh();
    let directory = root.path().join("preferences");
    let executable = std::env::current_exe().unwrap();
    let children: Vec<_> = ["甲", "乙"]
        .into_iter()
        .map(|prefix| {
            Command::new(&executable)
                .args([
                    "--exact",
                    "common_phrases::tests::common_phrases_child_process_adds",
                    "--test-threads=1",
                ])
                .env(CHILD_DIRECTORY, &directory)
                .env(CHILD_PREFIX, prefix)
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let document = store.load().unwrap();
    assert_eq!(document.phrases.len(), CHILD_ADDS * 2);
    for prefix in ["甲", "乙"] {
        for index in 0..CHILD_ADDS {
            let text = format!("{prefix}{index}");
            assert!(texts(&document).contains(&text.as_str()), "{text}");
        }
    }
}

#[cfg(unix)]
#[test]
fn finished_scope_releases_inherited_lock() {
    let root = tempfile::tempdir().unwrap();
    let store = CommonPhrasesStore::new(root.path());
    let held = store.lock().unwrap();
    let inherited = crate::file_lock::testing::duplicate(&held._lock);
    crate::file_lock::testing::assert_finished_scope_releases_lock(
        held,
        inherited,
        &root.path().join(LOCK_FILE_NAME),
    );
}

#[test]
fn common_phrase_notes_are_optional_local_metadata() {
    let (_root, store) = fresh();
    let legacy = store.add("synthetic command").unwrap();
    assert!(legacy.phrases[0].note.is_empty());
    let raw = fs::read(&store.file).unwrap();
    assert!(
        !serde_json::from_slice::<serde_json::Value>(&raw).unwrap()["phrases"][0]
            .as_object()
            .unwrap()
            .contains_key("note")
    );
    let id = legacy.phrases[0].id;
    let edited = store
        .replace_with_note(id, "synthetic command", Some("用途说明\n第二行🙂"))
        .unwrap();
    assert_eq!(edited.phrases[0].text, "synthetic command");
    assert_eq!(store.load().unwrap().phrases[0].note, "用途说明\n第二行🙂");
    // 旧的正文修改及排序请求没有备注字段，必须保留说明。
    assert_eq!(
        store
            .replace(id, "synthetic changed command")
            .unwrap()
            .phrases[0]
            .note,
        "用途说明\n第二行🙂"
    );
    assert_eq!(
        store.move_to(id, 0).unwrap().phrases[0].note,
        "用途说明\n第二行🙂"
    );
    let cleared = store
        .replace_with_note(id, "synthetic changed command", Some(" \n "))
        .unwrap();
    assert!(cleared.phrases[0].note.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&store.file).unwrap()).unwrap();
    assert!(!value["phrases"][0]
        .as_object()
        .unwrap()
        .contains_key("note"));
}

#[test]
fn common_phrase_notes_validate_before_any_part_of_the_edit_is_written() {
    let (_root, store) = fresh();
    let document = store
        .add_with_note("synthetic original", "synthetic note")
        .unwrap();
    let id = document.phrases[0].id;
    let before = fs::read(&store.file).unwrap();
    for note in [
        "x".repeat(MAX_PHRASE_UTF16 + 1),
        "🙂".repeat(501),
        "tab\tvalue".into(),
        "nul\0value".into(),
    ] {
        assert!(matches!(
            store.replace_with_note(id, "synthetic changed", Some(&note)),
            Err(CommonPhrasesError::InvalidNote)
        ));
        assert_eq!(fs::read(&store.file).unwrap(), before);
    }
    assert!(store
        .replace_with_note(id, "synthetic original", Some(&"🙂".repeat(500)))
        .is_ok());
    assert!(matches!(
        store.add_with_note("synthetic original", "different note"),
        Err(CommonPhrasesError::Duplicate)
    ));
    assert!(store.remove(id).unwrap().phrases.is_empty());
}

#[test]
fn common_phrase_note_actions_preserve_legacy_callers_and_allow_explicit_clear() {
    let (_root, store) = fresh();
    let action =
        serde_json::from_value(serde_json::json!({"operation":"add", "text":"synthetic legacy"}))
            .unwrap();
    let document = store.perform(action).unwrap();
    let id = document.document.phrases[0].id;
    for request in [
        serde_json::json!({"operation":"replace", "id":id, "text":"synthetic legacy", "note":"synthetic description"}),
        serde_json::json!({"operation":"replace", "id":id, "text":"synthetic changed"}),
    ] {
        store
            .perform(serde_json::from_value(request).unwrap())
            .unwrap();
        assert_eq!(
            store.load().unwrap().phrases[0].note,
            "synthetic description"
        );
    }
    store.perform(serde_json::from_value(serde_json::json!({"operation":"replace", "id":id, "text":"synthetic changed", "note":""})).unwrap()).unwrap();
    assert!(store.load().unwrap().phrases[0].note.is_empty());
}

#[test]
fn common_phrase_malformed_note_is_not_overwritten() {
    let (_root, store) = fresh();
    let document = store.add("synthetic entry").unwrap();
    let mut raw = serde_json::to_value(document).unwrap();
    raw["phrases"][0]["note"] = serde_json::json!(false);
    let before = serde_json::to_vec(&raw).unwrap();
    fs::write(&store.file, &before).unwrap();
    assert!(matches!(
        store.add("synthetic second"),
        Err(CommonPhrasesError::Corrupt)
    ));
    assert_eq!(fs::read(&store.file).unwrap(), before);
}
