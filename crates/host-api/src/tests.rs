//! Unit tests for the parent module, in their own file because the module
//! is large enough that mixing them with the implementation obscured both.
//! Same `mod tests` as before, so `use super::*` still names the parent.

use super::*;

mod bilingual_glide_selection;
mod deferred_candidate_identity;
mod local_translation;
mod mobile_clipboard_retention;

use crate::account::msime_client_account_name;
use msime_client_core::host_surface::compiled_input_schemes;
use sha2::{Digest, Sha256};

#[test]
fn eighteen_key_quanpin_layout_starts_on_android_and_ios() {
    let preferences = Preferences {
        touch_keyboard_layout: TouchKeyboardLayout::EighteenKey,
        ..Preferences::default()
    };
    for platform in [
        HostPlatform::Android,
        HostPlatform::Ios,
        HostPlatform::Harmony,
        HostPlatform::Linux,
        HostPlatform::Macos,
        HostPlatform::Windows,
    ] {
        assert_eq!(
            layout_key_grid_for(platform, Some(SchemeType::Quanpin), &preferences),
            matches!(platform, HostPlatform::Android | HostPlatform::Ios)
                .then_some(KeyGrid::EighteenKey),
            "{platform:?}"
        );
        assert_eq!(
            layout_key_grid_for(platform, Some(SchemeType::Shuangpin), &preferences),
            None,
            "全拼布局不能覆盖双拼方案：{platform:?}"
        );
    }
}

// Keep relative-path fixtures on the checkout's volume while allowing /source to be read-only in the Linux container.
fn relative_fixture_dir(prefix: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let current = std::env::current_dir().unwrap();
    for (parents, ancestor) in current.ancestors().enumerate() {
        if let Ok(directory) = tempfile::Builder::new().prefix(prefix).tempdir_in(ancestor) {
            let mut relative = std::path::PathBuf::from(".");
            for _ in 0..parents {
                relative.push("..");
            }
            relative.push(directory.path().file_name().unwrap());
            return (directory, relative);
        }
    }
    panic!("no writable ancestor for a relative-path fixture");
}

#[test]
fn selection_statistics_use_the_candidate_id_absolute_index() {
    let action = Action::Select(CandidateId {
        session: 7,
        generation: 11,
        index: 9,
    });
    assert_eq!(selected_position(&action), Some(10));
    let action = Action::SelectAnyCandidate(CandidateId {
        session: 7,
        generation: 11,
        index: 14,
    });
    assert_eq!(selected_position(&action), Some(15));
}

#[test]
fn multilingual_preference_mapping_filters_unavailable_japanese_and_needs_two_languages() {
    use msime_client_core::preferences::MultilingualLanguage;
    let mut preferences = Preferences::default();
    preferences.multilingual_input.enabled = true;
    preferences.multilingual_input.languages = vec![
        MultilingualLanguage::Chinese,
        MultilingualLanguage::English,
        MultilingualLanguage::Japanese,
    ];
    assert_eq!(
        multilingual_languages_for(&preferences, Edition::full(), true),
        vec![
            msime_engine::multilingual::Language::Chinese,
            msime_engine::multilingual::Language::English,
            msime_engine::multilingual::Language::Japanese,
        ]
    );
    assert_eq!(
        multilingual_languages_for(&preferences, Edition::full(), false),
        vec![
            msime_engine::multilingual::Language::Chinese,
            msime_engine::multilingual::Language::English,
        ]
    );
    for id in ["wubi", "japanese", "vietnamese", "tibetan"] {
        assert!(
            multilingual_languages_for(&preferences, Edition::by_id(id).unwrap(), true).is_empty()
        );
    }
    preferences.multilingual_input.languages = vec![MultilingualLanguage::Japanese];
    assert!(multilingual_languages_for(&preferences, Edition::full(), true).is_empty());
}

#[test]
fn completed_transition_keeps_dispatch_view_without_pending_changes() {
    let directory = tempfile::tempdir().unwrap();
    let handle = test_host(directory.path());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let session = sessions.get_mut(&handle).unwrap();
        let dispatched = session
            .runtime
            .dispatch(Action::Character {
                value: b'n',
                shift: false,
            })
            .unwrap();
        let mut expected = dispatched.view.clone();
        expected.generation = expected.generation.wrapping_add(99);
        let completed = session.complete_transition(Transition {
            handled: dispatched.handled,
            commit: dispatched.commit,
            commit_cursor_suffix: dispatched.commit_cursor_suffix,
            commit_context: dispatched.commit_context,
            diagnostic: dispatched.diagnostic,
            view: expected.clone(),
        });
        assert_eq!(completed.view.generation, expected.generation);
    });
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

#[test]
fn currency_punctuation_follows_character_width_on_all_key_routes() {
    for route in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let preferences = Preferences {
            caps_lock_ascii_punctuation: true,
            ..chinese_preferences()
        };
        let handle = test_host_with_pinyin_fixture(directory.path(), preferences);
        assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
        let confirm_literal = |result: &Value, expected: &str| {
            assert_eq!(result["value"]["handled"], true, "{result}");
            assert!(result["value"]["commit"].is_null(), "{result}");
            let view = &result["value"]["view"];
            assert_eq!(view["editing_text"], "nihao$", "{result}");
            assert_eq!(view["candidates"].as_array().unwrap().len(), 1, "{result}");
            assert_eq!(view["candidates"][0]["text"], "nihao$", "{result}");
            let confirmed = read(msime_client_command(handle, 1));
            assert_eq!(confirmed["value"]["commit"], expected, "{confirmed}");
            assert_eq!(
                confirmed["value"]["view"]["editing_text"], "",
                "{confirmed}"
            );
        };
        for fullwidth in [false, true, false] {
            read(msime_client_set_character_width(handle, fullwidth));
            let currency = if fullwidth { "￥" } else { "¥" };
            for composing in [false, true] {
                if composing {
                    type_letters(handle, b"nihao");
                }
                let result = read(match route {
                    0 => msime_client_character(handle, b'$', true),
                    1 => msime_client_punctuation(handle, b'$'),
                    _ => msime_client_punctuation_with_context(handle, b'$', 0),
                });
                assert_eq!(result["value"]["handled"], true);
                if composing {
                    // 组字中的 `$` 保留原文；只有确认上屏时才转换字宽。
                    let literal = if fullwidth {
                        "ｎｉｈａｏ＄"
                    } else {
                        "nihao$"
                    };
                    confirm_literal(&result, literal);
                } else {
                    assert_eq!(result["value"]["commit"], currency);
                }
            }
            assert_eq!(
                read(msime_client_punctuation(handle, b','))["value"]["commit"],
                "，"
            );
        }
        read(msime_client_set_chinese_punctuation(handle, false));
        type_letters(handle, b"nihao");
        confirm_literal(&read(msime_client_punctuation(handle, b'$')), "nihao$");
        read(msime_client_set_chinese_punctuation(handle, true));
        type_letters(handle, b"nihao");
        read(msime_client_set_caps_lock(handle, true));
        // 大写锁定只在空闲时直出 ASCII，已有组字仍保留原文。
        confirm_literal(
            &read(msime_client_punctuation_with_context(handle, b'$', 0)),
            "nihao$",
        );
        let idle = read(msime_client_punctuation_with_context(handle, b'$', 0));
        assert_eq!(idle["value"]["handled"], false);
        assert!(idle["value"]["commit"].is_null());
        assert_eq!(read(msime_client_destroy(handle))["ok"], true);
    }
}

#[test]
fn default_sentence_model_uses_verified_resources_not_prepared_dictionaries() {
    let root = tempfile::tempdir().unwrap();
    let resources = root.path().join("resources");
    let dictionaries = root.path().join("user/dictionaries/generation");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::create_dir_all(&dictionaries).unwrap();
    std::fs::write(resources.join("sentence-model.safetensors"), b"synthetic").unwrap();

    let actual = ffi::sentence_model_path(resources.to_str().unwrap(), None);
    assert_eq!(actual, resources.join("sentence-model.safetensors"));
    assert!(actual.is_file());
    assert!(!dictionaries.join("sentence-model.safetensors").exists());

    let explicit = root.path().join("custom-model.safetensors");
    assert_eq!(
        ffi::sentence_model_path(resources.to_str().unwrap(), explicit.to_str()),
        explicit
    );
}

#[cfg(unix)]
#[test]
fn resource_verification_rejects_a_symlinked_state_root() {
    let root = tempfile::tempdir().unwrap();
    let resources = root.path().join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(resources.join("fixture.db"), b"fixture").unwrap();
    let specification = ResourceSet {
        source_commit: "a".repeat(40),
        artifacts: vec![msime_client_core::resources::Artifact {
            name: "fixture.db".into(),
            url: "https://example.invalid/fixture.db".into(),
            sha256: hex::encode(Sha256::digest(b"fixture")),
            size: 7,
        }],
    };
    let outside = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    std::os::unix::fs::symlink(outside.path(), &state).unwrap();

    assert!(
        verify_resources_once(&resources, &specification, &state, ON_DEMAND_ARTIFACTS).is_err()
    );
    assert!(!outside.path().join("verified-resources.json").exists());
}

#[cfg(unix)]
#[test]
fn resource_verification_rejects_an_existing_state_root_below_a_symlink() {
    let root = tempfile::tempdir().unwrap();
    let resources = root.path().join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(resources.join("fixture.db"), b"fixture").unwrap();
    let specification = ResourceSet {
        source_commit: "a".repeat(40),
        artifacts: vec![msime_client_core::resources::Artifact {
            name: "fixture.db".into(),
            url: "https://example.invalid/fixture.db".into(),
            sha256: hex::encode(Sha256::digest(b"fixture")),
            size: 7,
        }],
    };
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(outside.path().join("state")).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let linked = parent.path().join("linked");
    msime_path_trust::untrusted_symlink(outside.path(), &linked).unwrap();
    let state = linked.join("state");

    assert!(
        verify_resources_once(&resources, &specification, &state, ON_DEMAND_ARTIFACTS).is_err()
    );
    assert!(!outside
        .path()
        .join("state/verified-resources.json")
        .exists());
}

#[test]
fn local_mode_resource_gates_preserve_unrelated_modes() {
    let root = tempfile::tempdir().unwrap();
    for name in ["msime-others.db", "msime-english.db", "msime-japanese.dat"] {
        std::fs::write(root.path().join(name), b"fixture").unwrap();
    }
    let mut options = EngineOptions {
        resources: root.path().to_string_lossy().into_owned(),
        user_data: root.path().to_string_lossy().into_owned(),
        cache: root.path().to_string_lossy().into_owned(),
        dictionaries: root.path().to_string_lossy().into_owned(),
        first_page_rows: 6,
        scheme: 0,
        enabled_schemes: msime_engine::SchemeSet::ALL,
        shuangpin_profile: 0,
        shuangpin_custom_profile: None,
        shuangpin_preedit_uses_raw: true,
        shuangpin_preedit_separated: false,
        force_cantonese_shuangpin: false,
        cantonese_output_conversion: None,
        single_character_only: false,
        caret_edit_suffix: false,
        learning: false,
        learn_english_on_enter: true,
        next_word_prediction: false,
        autocorrect_transposition: false,
        autocorrect_neighbor: false,
        fuzzy_pinyin_rules: 0,
        wubi_mixed_pinyin: false,
        wubi_z_wildcard: false,
        wubi_reverse_code: true,
        wubi_profile: 0,
        helpcode: false,
        strict_quanpin_helpcode: false,
        show_helpcode: false,
        shuangpin_helpcode_modes: msime_engine::ShuangpinHelpcodeModes::default(),
        grid_helpcode: false,
        helpcode_schema: "ziranma".into(),
        helpcode_trigger_key: None,
        chinese_punctuation: true,
        paired_punctuation: true,
        punctuation_lock: 0,
        corner_quotes: false,
        hollow_brackets: false,
        enumeration_comma_backslash: true,
        enumeration_comma_slash: false,
        frequency_mode: "disabled".into(),
        frequency_trigger_count: 1,
        frequency_linear_step: 1,
        mixed_candidates: true,
        mixed_english: false,
        english_minimum_prefix: 2,
        english_first_page_count: 1,
        mixed_emoji: false,
        mixed_kaomoji: false,
        mixed_symbols: false,
        mixed_japanese: true,
        multilingual_languages: Vec::new(),
        local_unicode: true,
        local_date_time: true,
        local_date_time_inline: true,
        local_date_time_menu: true,
        local_quick_phrase: true,
        local_emoji: true,
        local_kaomoji: true,
        local_super_jianpin: true,
        local_super_jianpin_slash: true,
        local_temporary_english: true,
        local_temporary_japanese: true,
        local_expression: false,
        local_command: false,
        local_mention: false,
        local_chaizi: false,
        local_symbol: true,
        local_shift_letter_entry: true,
        chaizi_traditional_output: false,
        command_table: Vec::new(),
        plugin_commands: Vec::new(),
        mention_entries: Vec::new(),
        quick_phrase_table: Vec::new(),
        quick_phrase_candidates: false,
        quick_phrase_frequency: true,
        helpcode_table: None,
        helpcode_fallback_resources: None,
        custom_helpcode_root: None,
        sentence_association: msime_engine::SentenceAssociationOptions {
            word_lattice: true,
            neural_keyboard: false,
            show_next_on_duplicate: false,
            ..Default::default()
        },
        rescoring_context: String::new(),
        sentence_alternatives: true,
        vietnamese_input_method: 0,
        vietnamese_tone_style: 0,
        korean_layout: 0,
        cantonese_dictionary: String::new(),
        zhuyin_dictionary: String::new(),
        stroke_dictionary: String::new(),
        japanese_dictionary: String::new(),
        wubi_dictionary: String::new(),
    };
    apply_local_mode_resource_gates(&mut options, Edition::full());
    assert!(options.local_unicode);
    assert!(options.local_date_time);
    assert!(options.local_quick_phrase);
    assert!(options.local_super_jianpin);
    assert!(options.local_emoji);
    assert!(options.local_kaomoji);
    assert!(options.local_temporary_english);
    assert!(options.local_temporary_japanese);
    assert!(options.mixed_japanese);

    std::fs::remove_file(root.path().join("msime-others.db")).unwrap();
    std::fs::remove_file(root.path().join("msime-japanese.dat")).unwrap();
    apply_local_mode_resource_gates(&mut options, Edition::full());
    assert!(!options.local_emoji);
    assert!(!options.local_kaomoji);
    assert!(!options.local_temporary_japanese);
    assert!(!options.mixed_japanese);
    assert!(options.local_temporary_english);
    assert!(options.local_unicode);
    assert!(options.local_date_time);
    assert!(options.local_quick_phrase);
    assert!(options.local_super_jianpin);
}

#[test]
fn surface_route_boundary_resolves_panels_and_rejects_bad_buffers() {
    let parse = |value: &str| {
        // SAFETY: the slice outlives the call.
        read(unsafe { msime_client_parse_surface_route(value.as_ptr(), value.len()) })
    };

    let keyboard = parse("keyboard");
    assert_eq!(keyboard["ok"], true);
    assert_eq!(keyboard["value"]["route"], "keyboard");
    assert_eq!(keyboard["value"]["panel"]["label"], "keyboard-panel");
    assert_eq!(keyboard["value"]["panel"]["width"], 1100);

    // The local clipboard panel the desktop host ships must stay reachable.
    let clipboard = parse("clipboard");
    assert_eq!(clipboard["value"]["panel"]["label"], "clipboard-panel");
    assert_eq!(clipboard["value"]["panel"]["height"], 620);

    let deep_link = parse("settings:voice");
    assert_eq!(deep_link["ok"], true);
    assert_eq!(deep_link["value"]["route"], "settings:voice");
    // Settings is the main window, so it carries no panel geometry.
    assert!(deep_link["value"].get("panel").is_none());

    for rejected in ["", "account", "settings:unknown", "Settings"] {
        assert_eq!(parse(rejected)["ok"], false, "accepted {rejected:?}");
    }

    // A null buffer and an oversized length are refused, not dereferenced.
    assert_eq!(
        read(unsafe { msime_client_parse_surface_route(std::ptr::null(), 8) })["ok"],
        false
    );
    let value = "settings";
    assert_eq!(
        read(unsafe { msime_client_parse_surface_route(value.as_ptr(), 4096) })["ok"],
        false
    );
    let invalid = [0xff_u8, 0xfe];
    assert_eq!(
        read(unsafe { msime_client_parse_surface_route(invalid.as_ptr(), invalid.len()) })["ok"],
        false
    );
}

#[test]
fn smart_punctuation_gesture_boundary_arms_and_decides_from_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            smart_punctuation: true,
            smart_punctuation_repeat: true,
            smart_punctuation_space_convert: true,
            ..chinese_preferences()
        },
    );
    let arm = |body: serde_json::Value| {
        let text = body.to_string();
        // SAFETY: the buffer outlives the call.
        read(unsafe { msime_client_smart_punctuation_arm(handle, text.as_ptr(), text.len()) })
    };
    let decide = |body: serde_json::Value| {
        let text = body.to_string();
        // SAFETY: the buffer outlives the call.
        read(unsafe { msime_client_smart_punctuation_decide(handle, text.as_ptr(), text.len()) })
    };

    // An ASCII mark the host committed arms the repeat gesture and nothing else.
    let armed = arm(json!({
        "ascii": 44, "commit": ",", "timestamp_ms": 1_000,
        "editor_generation": 3, "auto_closed_pair": false
    }));
    assert_eq!(armed["ok"], true);
    assert_eq!(armed["value"]["repeat"]["committed"], ",");
    assert!(
        armed["value"]["space"].is_null(),
        "a comma is not a Chinese mark"
    );

    let repeat = armed["value"]["repeat"].clone();
    let replaced = decide(json!({
        "character": 44, "preceding": ",", "timestamp_ms": 1_500,
        "editor_generation": 3, "repeat": repeat, "space": null
    }));
    assert_eq!(replaced["value"]["replace_with"], "，");
    assert!(replaced["value"]["space_ascii"].is_null());

    // Past the window, in another editor, or over a character that is no longer there: nothing.
    for (timestamp, generation, preceding) in [(4_000, 3, ","), (1_500, 9, ","), (1_500, 3, "好")]
    {
        let answer = decide(json!({
            "character": 44, "preceding": preceding, "timestamp_ms": timestamp,
            "editor_generation": generation, "repeat": armed["value"]["repeat"], "space": null
        }));
        assert!(
            answer["value"]["replace_with"].is_null(),
            "{timestamp} {generation} {preceding} still replaced"
        );
    }

    // A committed Chinese mark arms the space conversion instead.
    let armed = arm(json!({
        "ascii": 46, "commit": "。", "timestamp_ms": 2_000,
        "editor_generation": 3, "auto_closed_pair": false
    }));
    assert!(
        armed["value"]["repeat"].is_null(),
        "the ASCII press never landed"
    );
    assert_eq!(armed["value"]["space"]["ascii"], 46);

    let space = armed["value"]["space"].clone();
    let converted = decide(json!({
        "character": 32, "preceding": "。", "timestamp_ms": 2_100,
        "editor_generation": 3, "repeat": null, "space": space
    }));
    assert_eq!(converted["value"]["space_ascii"], 46);
    // An auto-closed pair never arms: the caret sits between the two marks.
    let paired = arm(json!({
        "ascii": 40, "commit": "（", "timestamp_ms": 2_000,
        "editor_generation": 3, "auto_closed_pair": true
    }));
    assert!(paired["value"]["space"].is_null());

    // Malformed buffers are refused, not dereferenced.
    assert_eq!(
        read(unsafe { msime_client_smart_punctuation_arm(handle, std::ptr::null(), 8) })["ok"],
        false
    );
    let body = json!({
        "character": 32, "preceding": "ab", "timestamp_ms": 1,
        "editor_generation": 3, "repeat": null, "space": null
    })
    .to_string();
    assert_eq!(
        read(unsafe { msime_client_smart_punctuation_decide(handle, body.as_ptr(), body.len()) })
            ["ok"],
        false,
        "preceding must be a single scalar"
    );
}

#[test]
fn smart_punctuation_gestures_follow_their_own_switches() {
    let dir = tempfile::tempdir().unwrap();
    let arm = |handle: u64, body: serde_json::Value| {
        let text = body.to_string();
        // SAFETY: the buffer outlives the call.
        read(unsafe { msime_client_smart_punctuation_arm(handle, text.as_ptr(), text.len()) })
    };
    let comma = json!({
        "ascii": 44, "commit": ",", "timestamp_ms": 0,
        "editor_generation": 1, "auto_closed_pair": false
    });
    let period = json!({
        "ascii": 46, "commit": "。", "timestamp_ms": 0,
        "editor_generation": 1, "auto_closed_pair": false
    });

    // 两者的默认不同：重复转中文跟着智能标点在所有平台上默认开，它是句末数字（「100。」）的退路；空格转换处处默认关，因为它改写的是用户已经看着落下去的字符。
    let shipped = test_host_preferences(dir.path(), chinese_preferences());
    assert_eq!(
        arm(shipped, comma.clone())["value"]["repeat"]["committed"],
        ","
    );
    assert!(
        arm(shipped, period.clone())["value"]["space"].is_null(),
        "space conversion is off until it is asked for"
    );

    // Each switch answers only for its own gesture.
    let repeat_off = test_host_preferences(
        dir.path(),
        Preferences {
            smart_punctuation: true,
            smart_punctuation_repeat: false,
            smart_punctuation_space_convert: true,
            ..chinese_preferences()
        },
    );
    assert!(arm(repeat_off, comma.clone())["value"]["repeat"].is_null());
    assert_eq!(
        arm(repeat_off, period.clone())["value"]["space"]["ascii"],
        46
    );

    // Smart punctuation off is the whole family off, whatever the sub-switches say.
    let all_off = test_host_preferences(
        dir.path(),
        Preferences {
            smart_punctuation: false,
            smart_punctuation_repeat: true,
            smart_punctuation_space_convert: true,
            ..chinese_preferences()
        },
    );
    assert!(arm(all_off, comma)["value"]["repeat"].is_null());
    assert!(arm(all_off, period)["value"]["space"].is_null());
}

#[test]
fn smart_punctuation_space_decision_rechecks_live_switches() {
    for (smart, space_convert) in [(true, true), (false, true), (true, false)] {
        let dir = tempfile::tempdir().unwrap();
        let mut preferences = Preferences {
            smart_punctuation: true,
            smart_punctuation_space_convert: true,
            ..chinese_preferences()
        };
        let handle = test_host_preferences(dir.path(), preferences.clone());
        assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
        let committed = read(msime_client_punctuation_with_context(
            handle,
            b'.',
            u32::from('中'),
        ));
        assert_eq!(committed["value"]["commit"], "。");
        let arm = json!({
            "ascii": 46, "commit": committed["value"]["commit"], "timestamp_ms": 1_000,
            "editor_generation": 3, "auto_closed_pair": false
        })
        .to_string();
        // 安全：请求缓冲区在调用期间保持有效。
        let armed =
            read(unsafe { msime_client_smart_punctuation_arm(handle, arm.as_ptr(), arm.len()) });
        assert_eq!(armed["value"]["space"]["ascii"], 46);

        preferences.smart_punctuation = smart;
        preferences.smart_punctuation_space_convert = space_convert;
        let updated = update(handle, 1, &preferences);
        assert_eq!(updated["ok"], true);
        assert_eq!(updated["value"]["deferred"], false);

        // 宿主保存的快照与编辑器上下文都仍有效，执行时必须尊重已经生效的开关。
        let decide = json!({
            "character": 32, "preceding": "。", "timestamp_ms": 1_100,
            "editor_generation": 3, "repeat": null, "space": armed["value"]["space"]
        })
        .to_string();
        // 安全：请求缓冲区在调用期间保持有效。
        let decided = read(unsafe {
            msime_client_smart_punctuation_decide(handle, decide.as_ptr(), decide.len())
        });
        assert_eq!(decided["ok"], true);
        assert_eq!(
            decided["value"]["space_ascii"],
            if smart && space_convert {
                json!(46)
            } else {
                Value::Null
            },
            "smart={smart}, space_convert={space_convert}"
        );
        assert_eq!(read(msime_client_destroy(handle))["ok"], true);
    }
}

#[test]
fn shuangpin_key_hint_boundary_publishes_the_engine_face() {
    let hints = |value: &str| {
        // SAFETY: the slice outlives the call.
        read(unsafe { msime_client_shuangpin_key_hints(value.as_ptr(), value.len()) })
    };

    let xiaohe = hints("xiaohe");
    assert_eq!(xiaohe["ok"], true);
    // Both units the key carries, not just the first one.
    assert_eq!(xiaohe["value"]["K"], "ing uai");
    // Initials and finals stay on their own side of the separator.
    assert_eq!(xiaohe["value"]["V"], "zh / ui ü");
    assert!(xiaohe["value"].as_object().unwrap().len() >= 26);

    // Each profile answers for itself rather than for the Engine's default.
    assert_ne!(hints("microsoft")["value"], xiaohe["value"]);
    assert_eq!(hints("microsoft")["value"][";"], "ing");

    // Cantonese shuangpin has no Mandarin profile name; the reserved name gives its face.
    let cantonese = hints("cantonese");
    assert_eq!(cantonese["ok"], true);
    assert_eq!(cantonese["value"]["I"], "gw / i");
    assert_eq!(cantonese["value"]["P"], "eon un ung ut");
    assert_eq!(cantonese["value"][";"], "ik ing");
    assert_eq!(cantonese["value"].as_object().unwrap().len(), 27);

    // An unknown name yields nothing instead of mislabelling the keys.
    for unknown in ["", "quanpin", "xiaohe-v2", "cantonese_shuangpin"] {
        assert_eq!(
            hints(unknown)["value"].as_object().unwrap().len(),
            0,
            "{unknown:?} produced hints"
        );
    }

    // A null buffer and an oversized length are refused, not dereferenced.
    assert_eq!(
        read(unsafe { msime_client_shuangpin_key_hints(std::ptr::null(), 6) })["ok"],
        false
    );
    let value = "xiaohe";
    assert_eq!(
        read(unsafe { msime_client_shuangpin_key_hints(value.as_ptr(), 4096) })["ok"],
        false
    );
}

#[test]
fn shuangpin_zero_initial_boundary_publishes_the_engine_table() {
    let codes = |value: &str| {
        // SAFETY: the slice outlives the call.
        read(unsafe { msime_client_shuangpin_zero_initials(value.as_ptr(), value.len()) })
    };

    let xiaohe = codes("xiaohe");
    assert_eq!(xiaohe["ok"], true);
    assert_eq!(xiaohe["value"].as_object().unwrap().len(), 12);
    assert_eq!(xiaohe["value"]["ang"], "ah");

    let unknown = codes("xiaohe-v2");
    assert_eq!(unknown["ok"], true);
    assert_eq!(unknown["value"].as_object().unwrap().len(), 0);

    assert_eq!(
        read(unsafe { msime_client_shuangpin_zero_initials(std::ptr::null(), 6) })["ok"],
        false
    );
}

#[test]
fn host_capability_boundary_describes_each_platform() {
    let capabilities = |value: &str| {
        // SAFETY: the slice outlives the call.
        read(unsafe { msime_client_host_capabilities(value.as_ptr(), value.len()) })
    };

    let linux = capabilities("linux");
    assert_eq!(linux["ok"], true);
    assert_eq!(linux["value"]["platform"], "linux");
    assert_eq!(linux["value"]["restart_input_method"], true);
    assert_eq!(linux["value"]["ime_mode_scope"], true);
    // Every host reads capabilities through this boundary; the Fcitx5 border follows the theme, so the Linux page gets no border picker.
    assert_eq!(linux["value"]["candidate_border_color"], false);
    assert_eq!(linux["value"]["candidate_selection_appearance"], false);
    // Fcitx5 主题照 macOS 应用圆角、整体缩放、半透明卡片、内边距和 logo 开关；IBus 面板做不到的由 candidate_panel_limit 说明。
    assert_eq!(linux["value"]["candidate_corner_radius"], true);
    assert_eq!(linux["value"]["candidate_window_scale"], true);
    assert_eq!(linux["value"]["candidate_window_opacity"], true);
    assert_eq!(linux["value"]["candidate_padding"], true);
    assert_eq!(linux["value"]["app_logo"], true);
    // 菜单栏状态项是 macOS 独有的：Linux 由 Fcitx5/IBus 面板显示模式。
    assert_eq!(linux["value"]["input_mode_status_item"], false);

    let windows = capabilities("windows");
    assert_eq!(windows["value"]["restart_input_method"], true);
    assert_eq!(windows["value"]["panel_windows"], true);
    // Typing statistics used to be gated on an Android user-agent match.
    assert_eq!(windows["value"]["typing_statistics"], true);
    // Windows 有托盘语言栏，不提供菜单栏状态项。
    assert_eq!(windows["value"]["input_mode_status_item"], false);

    let macos = capabilities("macos");
    assert_eq!(macos["value"]["restart_input_method"], true);
    assert_eq!(macos["value"]["candidate_follow_cursor"], true);
    assert_eq!(macos["value"]["input_mode_status_item"], true);
    // 「展开候选」网格目前只有 macOS 实现。
    assert_eq!(macos["value"]["expand_candidates"], true);
    assert_eq!(windows["value"]["expand_candidates"], false);
    assert_eq!(linux["value"]["expand_candidates"], false);
    // 桌面宿主的各界面一律跟随颜色模式，编号色和边框色跟随主题，设置页按这三项收起对应的控件。
    assert_eq!(macos["value"]["surface_themes_follow"], true);
    assert_eq!(macos["value"]["candidate_number_color"], false);
    assert_eq!(macos["value"]["candidate_border_color"], false);
    assert_eq!(windows["value"]["surface_themes_follow"], true);
    assert_eq!(windows["value"]["candidate_number_color"], false);
    assert_eq!(windows["value"]["candidate_border_color"], false);
    assert_eq!(linux["value"]["surface_themes_follow"], true);
    assert_eq!(linux["value"]["candidate_number_color"], false);

    let android = capabilities("android");
    assert_eq!(android["value"]["panel_windows"], false);
    assert_eq!(android["value"]["window_chrome"], false);
    // Android 的键盘仍读各界面的单独明暗和自定义编号色。
    assert_eq!(android["value"]["surface_themes_follow"], false);
    assert_eq!(android["value"]["candidate_number_color"], true);

    assert_eq!(capabilities("bsd")["ok"], false);
    assert_eq!(capabilities("")["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_host_capabilities(std::ptr::null(), 5) })["ok"],
        false
    );
}

#[test]
fn cloud_clipboard_e2e_boundary_starts_with_a_key_request() {
    let call = |bytes: &[u8]| {
        // SAFETY: the slice outlives the call.
        read(unsafe { msime_client_cloud_clipboard_e2e(bytes.as_ptr(), bytes.len()) })
    };
    let started = call(br#"{"start":{"op":"list"},"keystore":null,"device_name":"Test"}"#);
    assert_eq!(started["ok"], true);
    let value = &started["value"];
    assert_eq!(value["request"]["method"], "GET");
    assert!(value["request"]["path"]
        .as_str()
        .unwrap()
        .starts_with("/v1/users/me/clipboard/keys?device_id="));
    assert!(value["keystore"].as_str().unwrap().starts_with("v1."));
    assert!(value["state"].is_object());

    assert_eq!(call(b"{}")["ok"], false);
    assert_eq!(
        // SAFETY: a null pointer is rejected before it is read.
        read(unsafe { msime_client_cloud_clipboard_e2e(std::ptr::null(), 0) })["ok"],
        false
    );

    // 同一个入口按 domain 分派到输入习惯同步的驱动。
    let learning = call(
        br#"{"domain":"learning","start":{"op":"status"},"keystore":null,"device_name":"Test"}"#,
    );
    assert_eq!(learning["ok"], true);
    assert!(learning["value"]["request"]["path"]
        .as_str()
        .unwrap()
        .starts_with("/v1/users/me/learning?device_id="));
    assert_eq!(
        call(br#"{"domain":"dictionary","start":{"op":"status"}}"#)["ok"],
        false
    );
}

#[test]
fn traditional_conversion_boundary_returns_text_or_null() {
    let convert = |bytes: &[u8]| {
        // SAFETY: the slice outlives the call.
        let raw = unsafe { msime_client_simplified_to_traditional(bytes.as_ptr(), bytes.len()) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: a non-null result is an owned NUL-terminated string from this library.
        let text = unsafe { std::ffi::CStr::from_ptr(raw) }
            .to_str()
            .unwrap()
            .to_owned();
        // SAFETY: released exactly once.
        unsafe { msime_client_string_free(raw) };
        Some(text)
    };
    assert_eq!(convert("头发".as_bytes()).as_deref(), Some("頭髮"));
    assert_eq!(convert(b"").as_deref(), Some(""));
    assert_eq!(convert(b"\xff\xfe"), None);
    assert_eq!(convert(b"a\0b"), None);
    // SAFETY: null is part of the documented contract.
    assert!(unsafe { msime_client_simplified_to_traditional(std::ptr::null(), 4) }.is_null());
    // A native length is bounded before the borrowed slice is formed, so a malformed or stale
    // length cannot make the ABI scan beyond the caller's intended text buffer.
    let fixture = b"fixture";
    assert!(
        unsafe { msime_client_simplified_to_traditional(fixture.as_ptr(), (1 << 20) + 1) }
            .is_null()
    );
}

#[test]
fn traditional_variant_conversion_boundary_selects_the_configuration() {
    let convert = |bytes: &[u8], variant: u32| {
        // SAFETY: the slice outlives the call.
        let raw =
            unsafe { msime_client_convert_to_traditional(bytes.as_ptr(), bytes.len(), variant) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: a non-null result is an owned NUL-terminated string from this library.
        let text = unsafe { std::ffi::CStr::from_ptr(raw) }
            .to_str()
            .unwrap()
            .to_owned();
        // SAFETY: released exactly once.
        unsafe { msime_client_string_free(raw) };
        Some(text)
    };
    assert_eq!(
        convert("软件里面".as_bytes(), 0).as_deref(),
        Some("軟件裏面")
    );
    assert_eq!(
        convert("软件里面".as_bytes(), 1).as_deref(),
        Some("軟件裡面")
    );
    assert_eq!(
        convert("软件里面".as_bytes(), 2).as_deref(),
        Some("軟體裡面")
    );
    assert_eq!(convert("软件".as_bytes(), 3), None);
    assert_eq!(convert(b"\xff\xfe", 1), None);
    assert_eq!(convert(b"a\0b", 2), None);
    // SAFETY: null is part of the documented contract.
    assert!(unsafe { msime_client_convert_to_traditional(std::ptr::null(), 4, 1) }.is_null());
    let fixture = b"fixture";
    assert!(
        unsafe { msime_client_convert_to_traditional(fixture.as_ptr(), (1 << 20) + 1, 2) }
            .is_null()
    );
}

#[test]
fn font_catalog_boundary_lists_sorted_bounded_families() {
    let result = read(msime_client_list_font_families());
    if !system_fonts::supported() {
        assert_eq!(result, json!({ "ok": false, "error": "unsupported" }));
        return;
    }
    #[cfg(windows)]
    assert_eq!(result["ok"], true, "Windows 始终具备系统字体集合");
    // A Linux machine without fontconfig cannot list anything; that is a failure response, never a panic or a partial list.
    if result["ok"] == false {
        assert!(matches!(
            result["error"].as_str(),
            Some("font_catalog" | "font_catalog_limit")
        ));
        return;
    }
    let names: Vec<String> = serde_json::from_value(result["value"].clone()).unwrap();
    assert!(!names.is_empty());
    assert!(names.len() <= 16_384);
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(names
        .iter()
        .all(|name| !name.is_empty() && msime_client_core::is_bounded_text(name, 128)));
}

#[test]
fn abi_version_reports_the_theme_revision() {
    assert_eq!(msime_client_abi_version(), 3);
}
#[test]
fn native_preference_save_clears_history_only_after_successful_disable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let store = PreferencesStore::new(directory.path());
    let mut enabled = store
        .save(
            0,
            Preferences {
                clipboard_history: true,
                ..Preferences::default()
            },
        )
        .unwrap();
    assert!(store
        .capture_clipboard_text("synthetic saved history".into())
        .unwrap());
    let history = directory.path().join("clipboard_history.json");
    let original = std::fs::read(&history).unwrap();
    let save = |revision, snapshot: &PreferencesSnapshot| {
        let bytes = serde_json::to_vec(snapshot).unwrap();
        read(unsafe {
            msime_client_save_preferences(
                path.as_ptr(),
                path.len(),
                revision,
                bytes.as_ptr(),
                bytes.len(),
            )
        })
    };
    enabled.preferences.clipboard_history = false;
    assert_eq!(save(0, &enabled)["ok"], false);
    assert_eq!(std::fs::read(&history).unwrap(), original);
    let result = save(enabled.revision, &enabled);
    assert_eq!(result["ok"], true);
    assert_eq!(result["value"]["revision"], enabled.revision + 1);
    assert!(
        !history.exists(),
        "successful native disable retained clipboard history"
    );
    assert!(!store
        .capture_clipboard_text("synthetic stopped capture".into())
        .unwrap());
    let mut restored = store.load().unwrap();
    restored.preferences.clipboard_history = true;
    assert_eq!(save(restored.revision, &restored)["ok"], true);
    assert!(store
        .capture_clipboard_text("synthetic new capture".into())
        .unwrap());
    let before = std::fs::read(&history).unwrap();
    let current = store.load().unwrap();
    assert_eq!(save(current.revision, &current)["ok"], true);
    assert_eq!(std::fs::read(&history).unwrap(), before);
    std::fs::remove_file(&history).unwrap();
    std::fs::create_dir(&history).unwrap();
    let mut disabled = store.load().unwrap();
    disabled.preferences.clipboard_history = false;
    assert_eq!(save(disabled.revision, &disabled)["ok"], false);
    assert!(!store.load().unwrap().preferences.clipboard_history);
    assert_eq!(store.load().unwrap().revision, disabled.revision + 1);
    assert!(history.is_dir());
}

#[test]
fn desktop_clipboard_history_lists_pins_removes_and_clears_the_shared_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let call = |action: serde_json::Value| {
        let body = serde_json::to_vec(&json!({ "directory": path, "action": action })).unwrap();
        read(unsafe { msime_client_desktop_clipboard_history(body.as_ptr(), body.len()) })
    };
    let store = PreferencesStore::new(directory.path());
    store
        .save(
            0,
            Preferences {
                clipboard_history: true,
                ..Preferences::default()
            },
        )
        .unwrap();
    for text in ["synthetic first", "synthetic second"] {
        assert!(store.capture_clipboard_text(text.into()).unwrap());
    }
    let listed = call(json!({ "operation": "list" }));
    assert_eq!(listed["value"]["enabled"], true);
    let entries = listed["value"]["entries"].as_array().unwrap();
    assert_eq!(entries[0]["text"], "synthetic second");
    assert_eq!(entries[1]["text"], "synthetic first");
    assert!(entries[0]["timestampMs"].as_u64().unwrap() > 0);
    assert_eq!(entries[0]["pinned"], false);

    // A pinned entry sorts first and the answer is the history after the change.
    let pinned =
        call(json!({ "operation": "set_pinned", "text": "synthetic first", "pinned": true }));
    assert_eq!(pinned["value"]["entries"][0]["text"], "synthetic first");
    assert_eq!(pinned["value"]["entries"][0]["pinned"], true);
    let removed = call(json!({ "operation": "remove", "text": "synthetic second" }));
    assert_eq!(removed["value"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        call(json!({ "operation": "remove", "text": "" }))["error"],
        "clipboard_history_invalid"
    );
    assert_eq!(
        call(json!({ "operation": "unknown" }))["error"],
        "clipboard_history_invalid"
    );
    let cleared = call(json!({ "operation": "clear" }));
    assert_eq!(cleared["value"]["entries"], json!([]));
    assert!(!directory.path().join("clipboard_history.json").exists());

    // Switched off: the list is empty and sync is refused before the clipboard is read.
    assert!(store
        .capture_clipboard_text("synthetic third".into())
        .unwrap());
    let mut disabled = store.load().unwrap();
    disabled.preferences.clipboard_history = false;
    store.save(disabled.revision, disabled.preferences).unwrap();
    let listed = call(json!({ "operation": "list" }));
    assert_eq!(listed["value"]["enabled"], false);
    assert_eq!(listed["value"]["entries"], json!([]));
    assert_eq!(
        call(json!({ "operation": "sync" }))["error"],
        "clipboard_history_disabled"
    );
    let relative = br#"{"directory":"relative","action":{"operation":"list"}}"#;
    assert_eq!(
        read(unsafe { msime_client_desktop_clipboard_history(relative.as_ptr(), relative.len()) })
            ["error"],
        "clipboard_history_invalid"
    );
}

#[test]
fn account_cloud_clipboard_refuses_without_a_signed_in_session_or_a_valid_action() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let call = |action: serde_json::Value| {
        let body = serde_json::to_vec(&json!({ "directory": path, "action": action })).unwrap();
        read(unsafe { msime_client_account_cloud_clipboard(body.as_ptr(), body.len()) })
    };
    // No account-session.json: every action stops at the session, before the network.
    let id = "0123456789abcdef".repeat(4);
    for action in [
        json!({ "operation": "list", "search": "" }),
        json!({ "operation": "add", "text": "synthetic upload" }),
        json!({ "operation": "pin", "id": id, "pinned": true }),
        json!({ "operation": "set_retention", "days": 7 }),
        json!({ "operation": "clear" }),
    ] {
        let result = call(action.clone());
        assert_eq!(result["ok"], false, "{action}");
        assert_eq!(result["error"], "account_unauthorized", "{action}");
    }
    for action in [
        json!({ "operation": "add", "text": "" }),
        json!({ "operation": "add", "text": "界".repeat(4001) }),
        json!({ "operation": "pin", "id": "entry-1", "pinned": true }),
        json!({ "operation": "pin", "id": id }),
        json!({ "operation": "set_retention", "days": 3 }),
        json!({ "operation": "unknown" }),
        json!({}),
    ] {
        assert_eq!(
            call(action.clone())["error"],
            "invalid_cloud_clipboard",
            "{action}"
        );
    }
    assert!(!directory.path().join("account-session.json").exists());

    // 只用已有身份：本机没有密钥串时，经驱动的动作答 clipboard_identity_missing，不生成也不写出密钥串；不经驱动的动作照常停在会话上。
    let borrowed = |action: serde_json::Value| {
        let body = serde_json::to_vec(
            &json!({ "directory": path, "create_identity": false, "action": action }),
        )
        .unwrap();
        read(unsafe { msime_client_account_cloud_clipboard(body.as_ptr(), body.len()) })
    };
    for action in [
        json!({ "operation": "list", "search": "" }),
        json!({ "operation": "add", "text": "synthetic upload" }),
        json!({ "operation": "devices" }),
    ] {
        assert_eq!(
            borrowed(action.clone())["error"],
            "clipboard_identity_missing",
            "{action}"
        );
    }
    assert_eq!(
        borrowed(json!({ "operation": "clear" }))["error"],
        "account_unauthorized"
    );
    assert!(!directory.path().join("cloud-clipboard-keystore").exists());
    // 镜像过来的密钥串可以解析时照常往下走，这里停在没有登录会话上。
    use msime_client_services::account::AccountSessionStorage as _;
    let keystore = msime_client_core::cloud::clipboard_e2e::Keystore::generate()
        .unwrap()
        .encode();
    msime_client_services::account::FileAccountSessionStorage::new(
        directory.path(),
        msime_client_services::account::AccountSessionFileLayout::Native,
    )
    .save_clipboard_keystore(&keystore)
    .unwrap();
    assert_eq!(
        borrowed(json!({ "operation": "list", "search": "" }))["error"],
        "account_unauthorized"
    );
}

#[test]
fn relay_cloud_clipboard_checks_its_input_and_the_configured_relay_before_the_network() {
    let preferences = tempfile::tempdir().unwrap();
    let keystore = tempfile::tempdir().unwrap();
    let preferences_path = preferences.path().to_str().unwrap().to_owned();
    let keystore_path = keystore.path().to_str().unwrap().to_owned();
    let source = || {
        let bytes = preferences_path.as_bytes();
        read(unsafe { msime_client_cloud_clipboard_source(bytes.as_ptr(), bytes.len()) })
    };
    let call = |body: serde_json::Value| {
        let body = serde_json::to_vec(&body).unwrap();
        read(unsafe { msime_client_relay_cloud_clipboard(body.as_ptr(), body.len()) })
    };
    let request = |action: serde_json::Value| {
        json!({
            "preferences_directory": preferences_path,
            "directory": keystore_path,
            "device_name": "合成设备",
            "action": action,
        })
    };

    // 没配置中继：走账号，中继入口直接答 relay_not_configured。
    assert_eq!(source()["value"]["source"], "account");
    assert_eq!(
        call(request(json!({ "operation": "list", "search": "" })))["error"],
        "relay_not_configured"
    );
    let relative = b"relative";
    assert_eq!(
        read(unsafe { msime_client_cloud_clipboard_source(relative.as_ptr(), relative.len()) })
            ["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_cloud_clipboard_source(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_relay_cloud_clipboard(std::ptr::null(), 0) })["error"],
        "invalid_cloud_clipboard"
    );

    let store = PreferencesStore::new(preferences.path());
    let mut configured = store.load().unwrap().preferences;
    // 端口 9 上没有服务，真发出去的请求答 relay_unavailable。
    configured.cloud_clipboard_relay.url = "http://127.0.0.1:9".into();
    configured.cloud_clipboard_relay.token = "synthetic-relay-token".into();
    store.save(0, configured).unwrap();
    assert_eq!(source()["value"]["source"], "relay");

    for body in [
        request(json!({ "operation": "set_retention", "days": 3 })),
        request(json!({ "operation": "pin", "id": "entry-1", "pinned": true })),
        request(json!({})),
        json!({ "preferences_directory": "relative", "directory": keystore_path, "action": { "operation": "clear" } }),
        json!({ "preferences_directory": preferences_path, "directory": "relative", "action": { "operation": "clear" } }),
        json!({ "preferences_directory": preferences_path, "directory": keystore_path, "action": { "operation": "clear" }, "extra": true }),
    ] {
        assert_eq!(
            call(body.clone())["error"],
            "invalid_cloud_clipboard",
            "{body}"
        );
    }
    assert_eq!(
        call(request(json!({ "operation": "clear" })))["error"],
        "relay_unavailable"
    );
    // 只用已有身份：目录里没有这个中继的密钥串时，经驱动的动作停在本机（不是 relay_unavailable），不写出密钥串；直接请求的动作照常发出。
    let borrowed = |action: serde_json::Value| {
        let mut body = request(action);
        body["create_identity"] = json!(false);
        call(body)
    };
    for action in [
        json!({ "operation": "list", "search": "" }),
        json!({ "operation": "add", "text": "合成上传" }),
        json!({ "operation": "devices" }),
        json!({ "operation": "reset" }),
    ] {
        assert_eq!(
            borrowed(action.clone())["error"],
            "clipboard_identity_missing",
            "{action}"
        );
    }
    assert!(!keystore
        .path()
        .join("cloud-clipboard-relay-keystore")
        .exists());
    assert_eq!(
        borrowed(json!({ "operation": "clear" }))["error"],
        "relay_unavailable"
    );
    let mut wrong_type = request(json!({ "operation": "list", "search": "" }));
    wrong_type["create_identity"] = json!("no");
    assert_eq!(call(wrong_type)["error"], "invalid_cloud_clipboard");
    // 同一份配置快照提供修订号；地址、令牌改变或切回账号后旧动作都停在联网之前。
    let revision = source()["value"]["revision"].as_u64().unwrap();
    assert_eq!(revision, 1);
    let mut bound = request(json!({ "operation": "clear" }));
    bound["expected_revision"] = json!(revision);
    assert_eq!(call(bound.clone())["error"], "relay_unavailable");
    for field in ["token", "url"] {
        let snapshot = store.load().unwrap();
        let mut changed = snapshot.preferences;
        if field == "token" {
            changed.cloud_clipboard_relay.token = "another-synthetic-token".into();
        } else {
            changed.cloud_clipboard_relay.url = "http://127.0.0.1:10".into();
        }
        store.save(snapshot.revision, changed).unwrap();
        assert_eq!(call(bound.clone())["error"], "clipboard_source_changed");
    }
    let account_guarded = serde_json::to_vec(&json!({
        "directory": keystore_path, "preferences_directory": preferences_path,
        "action": { "operation": "clear" },
    }))
    .unwrap();
    assert_eq!(
        read(unsafe {
            msime_client_account_cloud_clipboard(account_guarded.as_ptr(), account_guarded.len())
        })["error"],
        "clipboard_source_changed"
    );
    let snapshot = store.load().unwrap();
    let mut changed = snapshot.preferences;
    changed.cloud_clipboard_relay = Default::default();
    store.save(snapshot.revision, changed).unwrap();
    assert_eq!(call(bound)["error"], "clipboard_source_changed");
    // 账号入口也收这三个动作：这个目录里没有登录会话，停在会话上，不联网。
    let account = serde_json::to_vec(&json!({
        "directory": keystore_path,
        "action": { "operation": "set_retention", "days": 7 },
    }))
    .unwrap();
    assert_eq!(
        read(unsafe { msime_client_account_cloud_clipboard(account.as_ptr(), account.len()) })
            ["error"],
        "account_unauthorized"
    );
}

#[test]
fn relay_exchange_checks_the_relay_and_the_request_before_the_network() {
    let call = |body: serde_json::Value| {
        let body = serde_json::to_vec(&body).unwrap();
        read(unsafe { msime_client_relay_cloud_clipboard_exchange(body.as_ptr(), body.len()) })
    };
    // 端口 9 上没有服务，真发出去的请求答 relay_unavailable。
    let request = |url: &str, method: &str, path: &str| {
        json!({
            "url": url,
            "token": "synthetic-relay-token",
            "method": method,
            "path": path,
            "body": "{}",
        })
    };
    let loopback = "http://127.0.0.1:9/msime";
    assert_eq!(
        call(request(
            loopback,
            "GET",
            "/v1/users/me/clipboard/keys?device_id=0"
        ))["error"],
        "relay_unavailable"
    );
    // 地址和令牌按偏好的规则校验：不是 HTTPS 的远端地址、太短的令牌都不发。
    assert_eq!(
        call(request(
            "http://relay.example.test",
            "GET",
            "/v1/users/me/clipboard"
        ))["error"],
        "relay_not_configured"
    );
    let mut short = request(loopback, "GET", "/v1/users/me/clipboard");
    short["token"] = json!("short");
    assert_eq!(call(short)["error"], "relay_not_configured");
    // 驱动之外的接口、认不出的方法、多余的字段：不发出去。
    for body in [
        request(loopback, "GET", "/v1/users/me/profile"),
        request(loopback, "GET", "/v1/users/me/clipboard/../profile"),
        request(loopback, "PATCH", "/v1/users/me/clipboard"),
        json!({ "url": loopback, "token": "synthetic-relay-token", "method": "GET" }),
        json!({ "url": loopback, "token": "synthetic-relay-token", "method": "GET", "path": "/v1/users/me/clipboard", "directory": "/tmp" }),
    ] {
        assert_eq!(
            call(body.clone())["error"],
            "invalid_cloud_clipboard",
            "{body}"
        );
    }
    let mut large = request(loopback, "PUT", "/v1/users/me/clipboard/settings");
    large["body"] = json!("x".repeat(70 * 1024));
    assert_eq!(call(large)["error"], "invalid_cloud_clipboard");
    assert_eq!(
        read(unsafe { msime_client_relay_cloud_clipboard_exchange(std::ptr::null(), 0) })["error"],
        "invalid_cloud_clipboard"
    );
}

#[test]
fn a_webdav_drive_is_a_relay_source_with_its_own_backend() {
    let preferences = tempfile::tempdir().unwrap();
    let keystore = tempfile::tempdir().unwrap();
    let preferences_path = preferences.path().to_str().unwrap().to_owned();
    let keystore_path = keystore.path().to_str().unwrap().to_owned();
    let source = || {
        let bytes = preferences_path.as_bytes();
        read(unsafe { msime_client_cloud_clipboard_source(bytes.as_ptr(), bytes.len()) })
    };
    let call = |action: serde_json::Value| {
        let body = serde_json::to_vec(&json!({
            "preferences_directory": preferences_path,
            "directory": keystore_path,
            "device_name": "合成设备",
            "action": action,
        }))
        .unwrap();
        read(unsafe { msime_client_relay_cloud_clipboard(body.as_ptr(), body.len()) })
    };
    assert!(source()["value"].get("backend").is_none());

    let store = PreferencesStore::new(preferences.path());
    let mut configured = store.load().unwrap().preferences;
    // 端口 9 上没有服务：真发出去的请求答 relay_unavailable。
    configured.cloud_clipboard_webdav.url = "http://127.0.0.1:9/dav".into();
    configured.cloud_clipboard_webdav.username = "synthetic@example.test".into();
    configured.cloud_clipboard_webdav.password = "synthetic-app-password".into();
    store.save(0, configured.clone()).unwrap();
    // 网盘也报 `relay`：已有的宿主不认识新的来源值，会当成错误；`backend` 只用来给用户说明。
    assert_eq!(source()["value"]["source"], "relay");
    assert_eq!(source()["value"]["backend"], "webdav");
    assert_eq!(
        call(json!({ "operation": "clear" }))["error"],
        "relay_unavailable"
    );
    assert_eq!(
        call(json!({ "operation": "set_retention", "days": 3 }))["error"],
        "invalid_cloud_clipboard"
    );
    // 中继和网盘不能同时配置，保存时就被拒绝。
    let mut both = configured.clone();
    both.cloud_clipboard_relay.url = "http://127.0.0.1:9".into();
    both.cloud_clipboard_relay.token = "synthetic-relay-token".into();
    let revision = store.load().unwrap().revision;
    assert!(store.save(revision, both).is_err());

    let mut relay = configured;
    relay.cloud_clipboard_webdav = Default::default();
    relay.cloud_clipboard_relay.url = "http://127.0.0.1:9".into();
    relay.cloud_clipboard_relay.token = "synthetic-relay-token".into();
    store.save(revision, relay).unwrap();
    assert_eq!(source()["value"]["backend"], "relay");
}

#[test]
fn the_webdav_exchange_takes_only_its_own_fields_and_a_cache_directory() {
    let cache = tempfile::tempdir().unwrap();
    let cache_path = cache.path().to_str().unwrap().to_owned();
    let call = |body: serde_json::Value| {
        let body = serde_json::to_vec(&body).unwrap();
        read(unsafe { msime_client_relay_cloud_clipboard_exchange(body.as_ptr(), body.len()) })
    };
    let request = || {
        json!({
            "backend": "webdav",
            "url": "http://127.0.0.1:9/dav",
            "username": "synthetic@example.test",
            "password": "synthetic-app-password",
            "directory": cache_path,
            "method": "GET",
            "path": "/v1/users/me/clipboard",
        })
    };
    assert_eq!(call(request())["error"], "relay_unavailable");
    // 不合规的请求在本机答，不碰网盘。
    let mut local = request();
    local["path"] = json!("/v1/users/me/clipboard/keys?device_id=nope");
    let answer = call(local);
    assert_eq!(answer["value"]["status"], 400);
    let without = |field: &str| {
        let mut body = request();
        body.as_object_mut().unwrap().remove(field);
        body
    };
    let with = |field: &str, value: serde_json::Value| {
        let mut body = request();
        body[field] = value;
        body
    };
    for body in [
        without("directory"),
        without("username"),
        without("password"),
        with("directory", json!("relative")),
        with("token", json!("synthetic-relay-token")),
        with("backend", json!("ftp")),
        with("path", json!("/v1/users/me/profile")),
    ] {
        assert_eq!(
            call(body.clone())["error"],
            "invalid_cloud_clipboard",
            "{body}"
        );
    }
    assert_eq!(
        call(with("url", json!("http://dav.example.test/dav")))["error"],
        "relay_not_configured"
    );
    assert_eq!(
        call(with("username", json!("has:colon")))["error"],
        "relay_not_configured"
    );
    // 中继的请求不收网盘的字段。
    assert_eq!(
        call(json!({
            "url": "http://127.0.0.1:9", "token": "synthetic-relay-token", "username": "x",
            "method": "GET", "path": "/v1/users/me/clipboard",
        }))["error"],
        "invalid_cloud_clipboard"
    );
}

#[test]
fn the_webdav_probe_checks_the_draft_before_the_network() {
    let call = |body: serde_json::Value| {
        let body = serde_json::to_vec(&body).unwrap();
        read(unsafe { msime_client_cloud_clipboard_webdav_probe(body.as_ptr(), body.len()) })
    };
    let draft = json!({
        "url": "http://127.0.0.1:9/dav",
        "username": "synthetic@example.test",
        "password": "synthetic-app-password",
    });
    assert_eq!(call(draft.clone())["error"], "relay_unavailable");
    let mut relative = draft.clone();
    relative["directory"] = json!("relative");
    assert_eq!(call(relative)["error"], "invalid_cloud_clipboard");
    let mut extra = draft.clone();
    extra["token"] = json!("x");
    assert_eq!(call(extra)["error"], "invalid_cloud_clipboard");
    let mut remote_http = draft.clone();
    remote_http["url"] = json!("http://dav.example.test/dav");
    assert_eq!(call(remote_http)["error"], "relay_not_configured");
    let mut empty = draft;
    empty["password"] = json!("");
    assert_eq!(call(empty)["error"], "relay_not_configured");
    assert_eq!(
        read(unsafe { msime_client_cloud_clipboard_webdav_probe(std::ptr::null(), 0) })["error"],
        "invalid_cloud_clipboard"
    );
}

#[test]
fn mixed_input_changes_defer_until_composition_ends() {
    use msime_client_core::preferences::MixedInputPreferences;
    let dir = tempfile::tempdir().unwrap();
    // Start with every mixed-input switch opposite to the update below, whatever the platform's factory default is, so the deferral is observable for each one.
    let mut initial = chinese_preferences();
    initial.mixed_input.emoji = false;
    initial.mixed_input.kaomoji = false;
    // 日文混输要有日文模型才交给 Engine，资源目录里先放一份。
    std::fs::create_dir_all(dir.path().join("resources")).unwrap();
    std::fs::write(dir.path().join("resources/msime-japanese.dat"), b"fixture").unwrap();
    let handle = test_host_preferences(dir.path(), initial);
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'U', true));
    let before = read(msime_client_view(handle));
    let mut preferences = Preferences {
        mixed_input: MixedInputPreferences {
            enabled: true,
            first_page_count: 1,
            english: false,
            minimum_prefix: 8,
            emoji: true,
            kaomoji: true,
            japanese: true,
            symbols: false,
            quick_phrase: false,
            quick_phrase_frequency: true,
        },
        ..Preferences::default()
    };
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
    assert_eq!(read(msime_client_view(handle)), before);
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let options = &sessions[&handle].options;
        assert!(options.mixed_english);
        assert!(!options.mixed_emoji);
        assert!(!options.mixed_kaomoji);
        assert!(!options.mixed_japanese);
        // 初始偏好把符号混输打开，延迟期间保留旧值。
        assert!(options.mixed_symbols);
    });
    read(msime_client_command(handle, 3));
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let options = &sessions[&handle].options;
        assert!(!options.mixed_english);
        // 三个桌面宿主不读文档里的触发字符数，固定为默认的 5；触屏宿主照文档。
        let expected_prefix = if cfg!(any(
            target_os = "android",
            target_os = "ios",
            target_env = "ohos"
        )) {
            8
        } else {
            5
        };
        assert_eq!(options.english_minimum_prefix, expected_prefix);
        assert!(options.mixed_emoji && options.mixed_kaomoji);
        assert!(options.mixed_japanese);
        assert!(!options.mixed_symbols);
    });
    preferences.mixed_input.minimum_prefix = 9;
    assert_eq!(update(handle, 2, &preferences)["ok"], false);
    read(msime_client_destroy(handle));
}
/// Android 键盘显示期间每秒把读到的偏好原样再交一次：同一份快照不再整套重做，但回复与第一次相同，组字中延后的偏好照样在组字结束后生效，设置页重新导入的效果包照样跟上。
#[test]
fn an_identical_preferences_resend_answers_like_the_first_update() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    bundle_wubi_table(&path("resources"));
    let state = path("state");
    let options = json!({ "api_version": 1, "resources": path("resources"), "user_data": path("user"), "cache": path("cache"), "dictionaries": path("dictionaries"), "preferences_directory": state, "preferences": chinese_preferences() }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'U', true));

    let mut preferences = chinese_preferences();
    preferences.candidate_page_size = 7;
    preferences.plugins.effect_pack = "neon".into();
    let first = update(handle, 1, &preferences);
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["value"]["deferred"], true);
    assert_eq!(update(handle, 1, &preferences), first);
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        assert!(sessions[&handle].requested_settled);
        assert!(sessions[&handle].preferences_pending);
    });
    let effect = read(msime_client_typing_effect_settings(handle));
    assert!(effect["value"]["issue"].is_string(), "{effect}");

    // 设置页装上了选中的效果包：原样再交一次偏好就跟上，不必等下一次聚焦。
    let pack = state.join("plugins/effect/neon");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(
        pack.join("plugin.toml"),
        "schema_version = 1\nkind = \"effect\"\nid = \"neon\"\nname = \"Neon\"\nversion = \"1\"\nlicense = \"CC0-1.0\"\n[effect]\nstyle = \"sparks\"\nintensity = 80\ncolors = [\"#00FFCC\"]\nparticles = 12\n",
    )
    .unwrap();
    assert_eq!(update(handle, 1, &preferences), first);
    let effect = read(msime_client_typing_effect_settings(handle));
    assert_eq!(effect["value"]["issue"], Value::Null, "{effect}");
    assert_eq!(effect["value"]["style"], "sparks");

    // 组字结束时延后的偏好换上，再交一次报告已经应用。
    read(msime_client_command(handle, 3));
    let settled = update(handle, 1, &preferences);
    assert_eq!(settled["ok"], true, "{settled}");
    assert_eq!(settled["value"]["deferred"], false);
    assert_eq!(settled["value"]["revision"], 1);
    assert_eq!(
        settled["value"]["floating_toolbar"],
        first["value"]["floating_toolbar"]
    );
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        assert!(!sessions[&handle].preferences_pending);
        assert_eq!(sessions[&handle].applied, preferences);
    });

    // 同一修订号换了内容仍然拒绝，不会被当成没变。
    preferences.candidate_page_size = 6;
    assert_eq!(update(handle, 1, &preferences)["ok"], false);
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

/// 桌面宿主只按 `cantonese.input_method` 强制粤语双拼，触屏宿主只按当前触屏方案；两边的字段互不影响，别的方案下都不强制。
#[test]
fn cantonese_shuangpin_is_forced_by_each_host_kind_own_field() {
    let desktop_choice = Preferences {
        cantonese: msime_client_core::preferences::CantonesePreferences {
            input_method: CantoneseInputMethod::Shuangpin,
        },
        ..Preferences::default()
    };
    let mut touch_choice = Preferences::default();
    touch_choice.touch_keyboard_schemes.selected = Some(TouchKeyboardScheme::CantoneseShuangpin);
    for platform in [
        HostPlatform::Macos,
        HostPlatform::Windows,
        HostPlatform::Linux,
    ] {
        assert!(
            force_cantonese_shuangpin_for(platform, InputScheme::Cantonese, &desktop_choice),
            "{platform:?}"
        );
        assert!(
            !force_cantonese_shuangpin_for(platform, InputScheme::Cantonese, &touch_choice),
            "{platform:?}"
        );
        assert!(
            !force_cantonese_shuangpin_for(platform, InputScheme::Quanpin, &desktop_choice),
            "{platform:?}"
        );
        assert!(
            !force_cantonese_shuangpin_for(
                platform,
                InputScheme::Cantonese,
                &Preferences::default()
            ),
            "{platform:?}"
        );
    }
    for platform in [
        HostPlatform::Android,
        HostPlatform::Ios,
        HostPlatform::Harmony,
    ] {
        assert!(
            force_cantonese_shuangpin_for(platform, InputScheme::Cantonese, &touch_choice),
            "{platform:?}"
        );
        assert!(
            !force_cantonese_shuangpin_for(platform, InputScheme::Cantonese, &desktop_choice),
            "{platform:?}"
        );
        assert!(
            !force_cantonese_shuangpin_for(platform, InputScheme::Wubi, &touch_choice),
            "{platform:?}"
        );
    }
}

/// 不读触发字符数的宿主（macOS、Windows、Linux）一律拿默认的 5，触屏宿主照文档里存的数。
#[test]
fn english_minimum_prefix_is_fixed_where_the_host_does_not_read_it() {
    for platform in [
        HostPlatform::Android,
        HostPlatform::Ios,
        HostPlatform::Harmony,
    ] {
        assert_eq!(english_minimum_prefix_for(platform, 2), 2, "{platform:?}");
        assert_eq!(english_minimum_prefix_for(platform, 8), 8, "{platform:?}");
    }
    for platform in [
        HostPlatform::Macos,
        HostPlatform::Windows,
        HostPlatform::Linux,
    ] {
        for stored in 1..=8 {
            assert_eq!(
                english_minimum_prefix_for(platform, stored),
                5,
                "{platform:?}"
            );
        }
    }
}

#[test]
fn frequency_changes_wait_for_composition_and_reject_invalid_updates() {
    use msime_client_core::preferences::{FrequencyMode, FrequencyPreferences};
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'U', true));
    let before = read(msime_client_view(handle));
    let mut preferences = Preferences {
        frequency: FrequencyPreferences {
            mode: FrequencyMode::Linear,
            trigger_count: 3,
            linear_step: 2,
        },
        ..Preferences::default()
    };
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
    assert_eq!(read(msime_client_view(handle)), before);
    SESSIONS
        .with(|sessions| assert_eq!(sessions.borrow()[&handle].options.frequency_mode, "promote"));
    read(msime_client_command(handle, 3));
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        assert_eq!(sessions[&handle].options.frequency_mode, "linear");
        assert_eq!(sessions[&handle].options.frequency_trigger_count, 3);
        assert_eq!(sessions[&handle].options.frequency_linear_step, 2);
    });
    preferences.frequency.trigger_count = 0;
    assert_eq!(update(handle, 2, &preferences)["ok"], false);
    read(msime_client_destroy(handle));
}

#[test]
fn fuzzy_pinyin_changes_wait_for_composition_and_disabled_rules_are_retained() {
    use msime_client_core::preferences::{FuzzyPinyinPreferences, FuzzyPinyinRule};
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'z', false));
    let mut preferences = Preferences {
        fuzzy_pinyin: FuzzyPinyinPreferences {
            enabled: true,
            rules: [FuzzyPinyinRule::ZZh].into_iter().collect(),
            seeded: false,
        },
        ..Preferences::default()
    };
    let queued = update(handle, 1, &preferences);
    assert_eq!(queued["value"]["deferred"], true);
    SESSIONS.with(|sessions| assert_eq!(sessions.borrow()[&handle].options.fuzzy_pinyin_rules, 0));
    read(msime_client_command(handle, 3));
    SESSIONS.with(|sessions| assert_eq!(sessions.borrow()[&handle].options.fuzzy_pinyin_rules, 1));

    preferences.fuzzy_pinyin.enabled = false;
    let disabled = update(handle, 2, &preferences);
    assert_eq!(disabled["value"]["deferred"], false);
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert_eq!(session.options.fuzzy_pinyin_rules, 0);
        assert!(!session.applied.fuzzy_pinyin.enabled);
        assert!(session
            .applied
            .fuzzy_pinyin
            .rules
            .contains(&FuzzyPinyinRule::ZZh));
    });
    read(msime_client_destroy(handle));
}

#[test]
fn wubi_mixed_pinyin_reaches_engine_and_applies_after_composition() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'a', false));
    let mut preferences = Preferences {
        scheme: InputScheme::Wubi,
        wubi_mixed_pinyin: true,
        ..Preferences::default()
    };
    let queued = update(handle, 1, &preferences);
    assert_eq!(queued["value"]["deferred"], true);
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].options.wubi_mixed_pinyin);
    });

    read(msime_client_command(handle, 3));
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert!(session.options.wubi_mixed_pinyin);
        assert!(session.applied.wubi_mixed_pinyin);
    });

    preferences.wubi_mixed_pinyin = false;
    let disabled = update(handle, 2, &preferences);
    assert_eq!(disabled["value"]["deferred"], false);
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].options.wubi_mixed_pinyin);
    });
    read(msime_client_destroy(handle));
}

/// 「只出单字」和其他候选偏好一样等组字结束才重建 Engine，关掉时立即生效。
#[test]
fn single_character_only_reaches_engine_after_composition() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'n', false));
    let mut preferences = Preferences {
        single_character_only: true,
        ..Preferences::default()
    };
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].options.single_character_only);
    });

    read(msime_client_command(handle, 3));
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert!(session.options.single_character_only);
        assert!(session.applied.single_character_only);
    });

    preferences.single_character_only = false;
    assert_eq!(update(handle, 2, &preferences)["value"]["deferred"], false);
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].options.single_character_only);
    });
    read(msime_client_destroy(handle));
}

fn caret_edit_host(root: &std::path::Path, preferences: Preferences) -> u64 {
    let path = |name| {
        let path = root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let resources = path("resources");
    let dictionaries = path("dictionaries");
    let fixture = "CREATE TABLE tbl_1_w(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_w VALUES('wo','w','我',1000);
                   CREATE TABLE tbl_1_h(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_h VALUES('hao','h','好',1000),('huo','h','火',300);
                   CREATE TABLE tbl_1_x(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_x VALUES('xian','x','现',1000);
                   CREATE TABLE tbl_1_c(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_c VALUES('chi','c','吃',1000);
                   CREATE TABLE tbl_2_c(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_2_c VALUES('chi''huo','ch','吃货',500);
                   CREATE TABLE tbl_1_g(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_g VALUES('guo','g','锅',1000);
                   CREATE TABLE tbl_2_h(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_2_h VALUES('huo''guo','hg','火锅',5000);
                   CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);";
    for directory in [&resources, &dictionaries] {
        rusqlite::Connection::open(directory.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(fixture)
            .unwrap();
    }
    bundle_wubi_table(&resources);
    let options = json!({ "api_version": 1, "resources": resources, "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries, "preferences": preferences }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true);
    created["value"]["session"].as_u64().unwrap()
}

/// 打完 `wohaoxianchihuoguo`，左移 9 次停在 `wohaoxian|chihuoguo`，返回此时的视图。
fn caret_edit_view(handle: u64) -> Value {
    for byte in b"wohaoxianchihuoguo" {
        read(msime_client_character(handle, *byte, false));
    }
    let mut view = Value::Null;
    for _ in 0..9 {
        view = read(msime_client_command(handle, 4))["value"]["view"].clone();
    }
    view
}

/// 「移动光标后编辑光标后的部分」是 Engine 偏好：桌面宿主上和「只出单字」一样等组字结束才重建 Engine。视图在模式生效时带上 `caret_edit_suffix` 和 `caret_edit_prefix`，首选是固定的那段接上整句；关掉时两项都不出现。
#[test]
fn caret_edit_suffix_reaches_the_engine_after_composition_and_the_view() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    let handle = caret_edit_host(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    let off = caret_edit_view(handle);
    assert!(off.get("caret_edit_suffix").is_none(), "{off}");
    assert!(off.get("caret_edit_prefix").is_none(), "{off}");
    assert_eq!(off["candidates"][0]["text"], "我好现");

    // 组字中打开：先记下，组字结束后才生效。
    preferences.caret_edit_suffix = true;
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].options.caret_edit_suffix);
    });
    read(msime_client_command(handle, 3));
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert!(session.options.caret_edit_suffix);
        assert!(session.applied.caret_edit_suffix);
    });
    let on = caret_edit_view(handle);
    assert_eq!(on["caret_edit_suffix"], true, "{on}");
    assert_eq!(on["caret_edit_prefix"], "我好现");
    assert_eq!(on["candidates"][0]["text"], "我好现吃火锅");
    assert_eq!(on["caret_position"], 9);

    // 组字中关掉：这一段组字照旧按光标后编辑走完，下一段才回到光标前缀解码。
    preferences.caret_edit_suffix = false;
    assert_eq!(update(handle, 2, &preferences)["value"]["deferred"], true);
    let committed = read(msime_client_command(handle, 1));
    assert_eq!(committed["value"]["commit"], "我好现吃火锅");
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].options.caret_edit_suffix);
    });
    let again = caret_edit_view(handle);
    assert!(again.get("caret_edit_suffix").is_none(), "{again}");
    assert_eq!(again["candidates"][0]["text"], "我好现");
    read(msime_client_destroy(handle));
}

/// 粤拼输出简体是 Engine 偏好：打开后 Engine 拿到繁转简，关掉后不再转换；默认关，粤拼照旧输出繁体。
#[test]
fn cantonese_simplified_output_hands_the_engine_a_conversion() {
    let mut preferences = Preferences::default();
    assert!(!preferences.cantonese_simplified_output);
    assert!(cantonese_output_conversion(&preferences).is_none());
    preferences.cantonese_simplified_output = true;
    let conversion = cantonese_output_conversion(&preferences).expect("conversion");
    assert_eq!(conversion.apply("佢係嗰個人喎"), "佢系嗰个人喎");

    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    SESSIONS.with(|sessions| {
        assert!(sessions.borrow()[&handle]
            .options
            .cantonese_output_conversion
            .is_none());
    });
    let mut preferences = Preferences {
        cantonese_simplified_output: true,
        ..Preferences::default()
    };
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], false);
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert_eq!(
            session.options.cantonese_output_conversion,
            Some(TextConversion(
                msime_client_core::chinese_conversion::cantonese_to_simplified
            ))
        );
        assert!(session.applied.cantonese_simplified_output);
    });
    preferences.cantonese_simplified_output = false;
    update(handle, 2, &preferences);
    SESSIONS.with(|sessions| {
        assert!(sessions.borrow()[&handle]
            .options
            .cantonese_output_conversion
            .is_none());
    });
    read(msime_client_destroy(handle));
}

#[test]
fn wubi_auto_commit_unique_reaches_the_runtime_without_waiting_for_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    SESSIONS.with(|sessions| {
        assert!(sessions.borrow()[&handle].runtime.wubi_auto_commit_unique());
    });

    // 组字进行中也立刻生效：它是宿主状态，不等 Engine 重建。
    read(msime_client_character(handle, b'a', false));
    let mut preferences = Preferences {
        scheme: InputScheme::Wubi,
        wubi_auto_commit_unique: false,
        ..Preferences::default()
    };
    let queued = update(handle, 1, &preferences);
    assert_eq!(queued["value"]["deferred"], true);
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].runtime.wubi_auto_commit_unique());
    });

    preferences.wubi_auto_commit_unique = true;
    update(handle, 2, &preferences);
    SESSIONS.with(|sessions| {
        assert!(sessions.borrow()[&handle].runtime.wubi_auto_commit_unique());
    });
    read(msime_client_destroy(handle));
}

#[test]
fn backspace_selection_preference_reaches_real_sessions_and_live_updates() {
    for initial in [false, true] {
        for live_change in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = |name| {
                let path = directory.path().join(name);
                std::fs::create_dir_all(&path).unwrap();
                path
            };
            let resources = path("resources");
            let dictionaries = path("dictionaries");
            let fixture = "CREATE TABLE tbl_1_w(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                INSERT INTO tbl_1_w VALUES('wo','w','我',100);
                CREATE TABLE tbl_1_s(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                INSERT INTO tbl_1_s VALUES('shi','s','是',100);
                CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);
                CREATE TABLE quick_parases(key TEXT,value TEXT,weight INTEGER);
                CREATE INDEX idx_quick_parases_key_weight ON quick_parases(key,weight DESC);";
            for root in [&resources, &dictionaries] {
                rusqlite::Connection::open(root.join("msime-pinyin.db"))
                    .unwrap()
                    .execute_batch(fixture)
                    .unwrap();
            }
            let mut preferences = Preferences {
                backspace_reverts_selection: initial,
                candidate_page_size: 9,
                ..chinese_preferences()
            };
            let options = json!({ "api_version": 1, "resources": resources,
                "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries,
                "preferences": preferences, "phrase_preedit": true })
            .to_string();
            let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
            assert_eq!(created["ok"], true, "{created}");
            let handle = created["value"]["session"].as_u64().unwrap();
            read(msime_client_focus(handle, true));
            let view = type_letters(handle, b"woshi");
            let index = view["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .position(|candidate| candidate["text"] == "我")
                .expect("synthetic partial candidate");
            let generation = view["generation"].as_u64().unwrap();
            let held = read(msime_client_select(handle, generation, index));
            assert_eq!(held["value"]["view"]["phrase_prefix"], "我", "{held}");
            assert_eq!(held["value"]["view"]["editing_text"], "shi", "{held}");
            if live_change {
                preferences.backspace_reverts_selection = !initial;
                assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
            }
            for expected in ["sh", "s"] {
                let edited = read(msime_client_command(handle, 0));
                assert_eq!(edited["value"]["view"]["phrase_prefix"], "我", "{edited}");
                assert_eq!(
                    edited["value"]["view"]["editing_text"], expected,
                    "{edited}"
                );
                assert!(edited["value"]["commit"].is_null(), "{edited}");
            }
            let ended = read(msime_client_command(handle, 0));
            if preferences.backspace_reverts_selection {
                assert_eq!(ended["value"]["view"]["editing_text"], "wo", "{ended}");
                assert!(ended["value"]["commit"].is_null(), "{ended}");
            } else {
                assert_eq!(ended["value"]["commit"], "我", "{ended}");
                assert_eq!(ended["value"]["view"]["editing_text"], "", "{ended}");
                let next = read(msime_client_command(handle, 0));
                assert_eq!(next["value"]["handled"], false, "{next}");
                assert!(next["value"]["commit"].is_null(), "{next}");
            }
            assert_eq!(
                ended["value"]["view"]["phrase_prefix"]
                    .as_str()
                    .unwrap_or_default(),
                "",
                "{ended}"
            );
            read(msime_client_destroy(handle));
        }
    }
}

/// `navigation.left_right` 是共享偏好但由 runtime 解释：新建会话时读一次，之后每次偏好更新也跟着走，宿主的按键路由一行都不用改。
/// 开启时右键在输入串末尾换成下一个候选，显式关闭后同一个键什么也不做；缺键按出厂值读作开。
#[test]
fn left_right_navigation_preference_reaches_real_sessions_and_live_updates() {
    let root = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    assert!(preferences.navigation.left_right);
    // 左右键只在横排候选窗里选词；缺省是竖排。
    preferences.candidate_layout = msime_client_core::preferences::CandidateLayout::Horizontal;
    let handle = test_host_with_pinyin_fixture(root.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    type_letters(handle, b"nihao");
    let highlighted = |view: &Value| {
        view["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["highlighted"] == true)
            .map(|candidate| candidate["text"].as_str().unwrap().to_owned())
    };

    // 默认开启时右键在输入串末尾换成下一个候选，输入串一个字都没变。
    let default_open = read(msime_client_command(handle, 5));
    assert_eq!(default_open["ok"], true, "{default_open}");
    assert_eq!(
        default_open["value"]["view"]["caret_position"], 5,
        "{default_open}"
    );
    assert_eq!(
        default_open["value"]["view"]["editing_text"], "nihao",
        "{default_open}"
    );
    let first = default_open["value"]["view"]["candidates"][0]["text"]
        .as_str()
        .map(str::to_owned);
    let default_highlighted = highlighted(&default_open["value"]["view"]);
    assert!(first.is_some(), "{default_open}");
    assert_ne!(default_highlighted, first, "{default_open}");

    // 显式关闭后同一个键恢复普通光标命令，高亮不变。
    preferences.navigation.left_right = false;
    assert_eq!(update(handle, 1, &preferences)["ok"], true);
    let closed = read(msime_client_command(handle, 5));
    assert_eq!(
        highlighted(&closed["value"]["view"]),
        default_highlighted,
        "{closed}"
    );
    assert_eq!(closed["value"]["view"]["caret_position"], 5, "{closed}");

    // 重新打开、但候选窗改成竖排：左右键与候选方向垂直，同一个键仍是普通光标命令。
    preferences.navigation.left_right = true;
    preferences.candidate_layout = msime_client_core::preferences::CandidateLayout::Vertical;
    assert_eq!(update(handle, 2, &preferences)["ok"], true);
    let vertical = read(msime_client_command(handle, 5));
    assert_eq!(
        highlighted(&vertical["value"]["view"]),
        default_highlighted,
        "{vertical}"
    );
    assert_eq!(vertical["value"]["view"]["caret_position"], 5, "{vertical}");

    read(msime_client_destroy(handle));
}

#[test]
fn wubi_candidate_selection_updates_while_engine_preferences_are_deferred() {
    let root = tempfile::tempdir().unwrap();
    let handle = test_host(root.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'a', false));
    let mut preferences = Preferences::default();
    preferences.wubi_candidate_selection.enabled = true;
    preferences.wubi_candidate_selection.second_key = "/".into();
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
    SESSIONS.with(|sessions| {
        assert_eq!(
            sessions.borrow()[&handle]
                .runtime
                .wubi_candidate_selection()
                .slot(b'/'),
            Some(1)
        );
    });
    preferences.wubi_candidate_selection.enabled = false;
    update(handle, 2, &preferences);
    SESSIONS.with(|sessions| {
        assert_eq!(
            sessions.borrow()[&handle]
                .runtime
                .wubi_candidate_selection()
                .slot(b'/'),
            None
        );
    });
    read(msime_client_destroy(handle));
}

#[test]
fn wubi_profile_reaches_engine_options() {
    use msime_client_core::preferences::WubiProfile;
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    SESSIONS.with(|sessions| {
        assert_eq!(sessions.borrow()[&handle].options.wubi_profile, 0);
    });
    let preferences = Preferences {
        scheme: InputScheme::Wubi,
        wubi_profile: WubiProfile::Wubi98,
        ..Preferences::default()
    };
    update(handle, 1, &preferences);
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert_eq!(session.options.wubi_profile, 1);
        assert_eq!(session.applied.wubi_profile, WubiProfile::Wubi98);
    });
    let preferences = Preferences {
        wubi_profile: WubiProfile::WubiXsj,
        ..preferences
    };
    update(handle, 2, &preferences);
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert_eq!(session.options.wubi_profile, 2);
        assert_eq!(session.applied.wubi_profile, WubiProfile::WubiXsj);
    });
    read(msime_client_destroy(handle));
}

/// #10790：「候选显示剩余编码」决定引擎给不给五笔候选附完整码，建会话和改偏好两条路都要带上。
#[test]
fn wubi_code_hint_reaches_engine_options() {
    // 出厂默认开着，引擎照旧给五笔候选附码。
    assert!(Preferences::default().wubi_code_hint);
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    SESSIONS.with(|sessions| {
        assert!(sessions.borrow()[&handle].options.wubi_reverse_code);
    });
    let mut preferences = Preferences {
        scheme: InputScheme::Wubi,
        wubi_code_hint: false,
        ..chinese_preferences()
    };
    update(handle, 1, &preferences);
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert!(!session.options.wubi_reverse_code);
        assert!(!session.applied.wubi_code_hint);
    });
    preferences.wubi_code_hint = true;
    update(handle, 2, &preferences);
    SESSIONS.with(|sessions| {
        assert!(sessions.borrow()[&handle].options.wubi_reverse_code);
    });
    read(msime_client_destroy(handle));

    let created_dir = tempfile::tempdir().unwrap();
    let created = test_host_preferences(
        created_dir.path(),
        Preferences {
            scheme: InputScheme::Wubi,
            wubi_code_hint: false,
            ..chinese_preferences()
        },
    );
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&created].options.wubi_reverse_code);
    });
    read(msime_client_destroy(created));
}

/// 小鹤的键位，只把 ing 从 k 挪到 `;`。
fn custom_shuangpin_table() -> msime_client_core::preferences::ShuangpinCustomProfile {
    serde_json::from_value(json!({
        "initials": { "zh": "v", "ch": "i", "sh": "u" },
        "finals": {
            "iu": "q", "ei": "w", "e": "e", "uan": "r", "ue": "t", "ve": "t", "un": "y",
            "u": "u", "i": "i", "uo": "o", "o": "o", "ie": "p", "a": "a", "ong": "s",
            "iong": "s", "ai": "d", "en": "f", "eng": "g", "ang": "h", "an": "j", "uai": "k",
            "ing": ";", "uang": "l", "iang": "l", "ou": "z", "ua": "x", "ia": "x", "ao": "c",
            "ui": "v", "v": "v", "in": "b", "iao": "n", "ian": "m"
        },
        "zero_initials": {
            "a": "aa", "ai": "ai", "an": "an", "ang": "ah", "ao": "ao", "e": "ee", "ei": "ei",
            "en": "en", "eng": "eg", "er": "er", "o": "oo", "ou": "ou"
        }
    }))
    .unwrap()
}

#[test]
fn custom_shuangpin_profile_reaches_engine_options_or_falls_back_to_xiaohe() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    // 选了自定义方案却没有表：按小鹤运行，诊断里写明原因。
    let mut preferences = Preferences {
        scheme: InputScheme::Shuangpin,
        shuangpin_profile: ShuangpinProfile::Custom,
        ..chinese_preferences()
    };
    let response = update(handle, 1, &preferences);
    let diagnostic = response["value"]["diagnostic"].as_str().unwrap_or_default();
    assert!(
        diagnostic.starts_with(
            "Custom shuangpin profile unusable because initials unit \"zh\" has no key"
        ),
        "{response}"
    );
    assert_eq!(response["value"]["view"]["shuangpin_profile"], "xiaohe");
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert_eq!(session.options.shuangpin_profile, 0);
        assert_eq!(session.options.shuangpin_custom_profile, None);
    });

    preferences.shuangpin_custom_profile = custom_shuangpin_table();
    let response = update(handle, 2, &preferences);
    assert!(response["value"].get("diagnostic").is_none(), "{response}");
    assert_eq!(response["value"]["view"]["shuangpin_profile"], "custom");
    assert_eq!(response["value"]["view"]["microsoft_shuangpin"], true);
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert_eq!(session.options.shuangpin_profile, 4);
        assert_eq!(
            session.options.shuangpin_custom_profile,
            Some(engine_shuangpin_custom_table(&custom_shuangpin_table()))
        );
    });
    read(msime_client_character(handle, b'x', false));
    let typed = read(msime_client_character(handle, b';', false));
    assert_eq!(typed["value"]["view"]["editing_text"], "x;", "{typed}");

    // 换回内置方案时不再把表交给 Engine，但偏好里的表留着。
    read(msime_client_command(handle, 3));
    preferences.shuangpin_profile = ShuangpinProfile::Ziranma;
    update(handle, 3, &preferences);
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert_eq!(session.options.shuangpin_profile, 1);
        assert_eq!(session.options.shuangpin_custom_profile, None);
        assert!(!session.applied.shuangpin_custom_profile.is_empty());
    });
    read(msime_client_destroy(handle));
}

/// 九键双拼的布局让全拼会话以九键双拼开始：数字两位一个音节，`View.key_grid` 说出网格而 `nine_key` 为假；换了表就换网格；自定义表不能用时按米旮旯 3 运行并报告原因；别的方案没有它。
#[test]
fn a_nine_key_shuangpin_layout_starts_the_grid_with_the_chosen_table() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        touch_keyboard_layout: TouchKeyboardLayout::NineKeyShuangpin,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
    // 换表用例需要单字频度稳定切分排序，避免无频度的路径按字典序挤出查询预算。
    let priors = "CREATE TABLE tbl_1_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                  INSERT INTO tbl_1_n VALUES('ni','n','甲',100);
                  CREATE TABLE tbl_1_h(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                  INSERT INTO tbl_1_h VALUES('hao','h','乙',100);";
    for directory in ["resources", "dictionaries"] {
        rusqlite::Connection::open(dir.path().join(directory).join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(priors)
            .unwrap();
    }
    read(msime_client_focus(handle, true));
    let view = read(msime_client_view(handle));
    assert_eq!(view["value"]["key_grid"], "nine_key_shuangpin");
    assert_eq!(view["value"]["nine_key"], false);
    // 米旮旯 3：ni 是 64，hao 是 47。
    let mut typed = Value::Null;
    for &digit in b"6447" {
        typed = read(msime_client_character(handle, digit, false));
        assert_eq!(typed["value"]["handled"], true, "{typed}");
    }
    let view = &typed["value"]["view"];
    assert_eq!(view["candidates"][0]["text"], "本地", "{view}");
    assert_eq!(view["nine_key_reading"], "ni'hao", "{view}");
    // 数字是组码，不是选词键：米旮旯 3 不用 1 以外的键时也一样，1 在这里是 ong、ou、ing。
    let one = read(msime_client_character(handle, b'1', false));
    assert_eq!(one["value"]["view"]["editing_text"], "64471", "{one}");
    read(msime_client_command(handle, 3));

    // 米旮旯 2 的 ao 在 2：同一个词是 64 42。
    preferences.nine_key_shuangpin_profile = NineKeyShuangpinProfile::Migala2;
    let updated = update(handle, 1, &preferences);
    assert_eq!(updated["ok"], true, "{updated}");
    assert_eq!(updated["value"]["deferred"], false, "{updated}");
    assert_eq!(updated["value"]["view"]["key_grid"], "nine_key_shuangpin");
    for &digit in b"644" {
        read(msime_client_character(handle, digit, false));
    }
    typed = read(msime_client_character(handle, b'2', false));
    assert_eq!(
        typed["value"]["view"]["candidates"][0]["text"], "本地",
        "{typed}"
    );
    assert_eq!(
        typed["value"]["view"]["nine_key_reading"], "ni'hao",
        "{typed}"
    );
    // 米旮旯 2 不用 0 键：0 不进九宫格，组字照旧。
    let zero = read(msime_client_character(handle, b'0', false));
    assert_eq!(zero["value"]["view"]["editing_text"], "6442", "{zero}");
    read(msime_client_command(handle, 3));

    // 自定义表不能用：按米旮旯 3 运行，诊断里写明原因。
    preferences.nine_key_shuangpin_profile = NineKeyShuangpinProfile::Custom;
    let response = update(handle, 2, &preferences);
    let diagnostic = response["value"]["diagnostic"].as_str().unwrap_or_default();
    assert!(
        diagnostic.starts_with(
            "Nine-key shuangpin table unusable because initials unit \"b\" has no key"
        ),
        "{response}"
    );
    assert!(diagnostic.ends_with("using migala3."), "{response}");
    assert_eq!(response["value"]["view"]["key_grid"], "nine_key_shuangpin");
    for &digit in b"6447" {
        typed = read(msime_client_character(handle, digit, false));
    }
    assert_eq!(
        typed["value"]["view"]["candidates"][0]["text"], "本地",
        "{typed}"
    );
    read(msime_client_command(handle, 3));
    // 能用的自定义表（辜氏 2 低重码的表）照用，不再报告。
    preferences.nine_key_shuangpin_custom_profile =
        builtin_nine_key_shuangpin_custom_profile(NineKeyShuangpinProfile::Gushi2Low).unwrap();
    let response = update(handle, 3, &preferences);
    assert!(response["value"].get("diagnostic").is_none(), "{response}");

    // 宿主可以关掉网格再用 3 打开，表仍按偏好；别的方案没有九键双拼。
    assert_eq!(
        read(msime_client_set_key_grid(handle, 0))["value"]["key_grid"],
        "none"
    );
    assert_eq!(
        read(msime_client_set_key_grid(handle, 3))["value"]["key_grid"],
        "nine_key_shuangpin"
    );
    preferences.scheme = InputScheme::Shuangpin;
    let shuangpin = update(handle, 4, &preferences);
    assert_eq!(
        shuangpin["value"]["view"]["key_grid"], "none",
        "{shuangpin}"
    );
    assert_eq!(read(msime_client_set_key_grid(handle, 3))["ok"], false);
    read(msime_client_destroy(handle));
}

fn nine_key_shuangpin_layout(request: Value) -> Value {
    let request = request.to_string();
    read(unsafe { msime_client_nine_key_shuangpin_layout(request.as_ptr(), request.len()) })
}

/// 键面和编辑器从同一个接口取表：内置方案给出它的表和键面；自定义表给出问题的位置，不能用时键面是回退到的米旮旯 3；不认识的方案名报错。
#[test]
fn nine_key_shuangpin_layouts_give_tables_problems_and_key_faces() {
    let migala3 = nine_key_shuangpin_layout(json!({ "profile": "migala3" }));
    assert_eq!(migala3["ok"], true, "{migala3}");
    let value = &migala3["value"];
    assert_eq!(value["effective"], "migala3");
    assert_eq!(value["problem"], Value::Null);
    assert_eq!(value["table"]["initials"]["zh"], "8");
    assert_eq!(value["table"]["zero_initials"]["an"], "25");
    let keys = value["keys"].as_array().unwrap();
    let faces: Vec<&str> = keys
        .iter()
        .map(|key| key["key"].as_str().unwrap())
        .collect();
    assert_eq!(faces, ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]);
    assert_eq!(keys[1]["initials"], json!(["b", "c"]));
    assert_eq!(keys[1]["finals"], json!(["a", "ai", "iang"]));
    assert_eq!(keys[1]["zero_initial"], true);
    assert_eq!(keys[9]["finals"], json!(["u", "ü", "ei"]));

    let mut table =
        builtin_nine_key_shuangpin_custom_profile(NineKeyShuangpinProfile::Migala2).unwrap();
    table.finals.insert("ang".into(), "x".into());
    let broken = nine_key_shuangpin_layout(json!({ "profile": "custom", "table": table }));
    let value = &broken["value"];
    assert_eq!(value["effective"], "migala3", "{broken}");
    assert_eq!(
        value["problem"],
        json!({ "kind": "invalid_key", "part": "finals", "unit": "ang", "key": "x" })
    );
    assert_eq!(value["keys"].as_array().unwrap().len(), 10);

    table.finals.insert("ang".into(), "5".into());
    let custom = nine_key_shuangpin_layout(json!({ "profile": "custom", "table": table }));
    let value = &custom["value"];
    assert_eq!(value["effective"], "custom", "{custom}");
    assert_eq!(value["problem"], Value::Null);
    // 米旮旯 2 不用 0 键。
    assert_eq!(value["keys"].as_array().unwrap().len(), 9);

    assert_eq!(
        nine_key_shuangpin_layout(json!({ "profile": "gushi2" }))["ok"],
        false
    );
    assert_eq!(
        nine_key_shuangpin_layout(json!({ "profile": "migala3", "extra": 1 }))["ok"],
        false
    );
}

#[test]
fn builtin_nine_key_shuangpin_tables_are_valid_custom_profiles() {
    for profile in [
        NineKeyShuangpinProfile::Migala2,
        NineKeyShuangpinProfile::Migala3,
        NineKeyShuangpinProfile::Gushi2Low,
    ] {
        let table = builtin_nine_key_shuangpin_custom_profile(profile).unwrap();
        assert_eq!(
            (
                table.initials.len(),
                table.finals.len(),
                table.zero_initials.len()
            ),
            (23, 33, 12),
            "{profile:?}"
        );
        assert_eq!(nine_key_shuangpin_custom_profile_problem(&table), None);
        assert_eq!(validate_nine_key_shuangpin_custom_profile(&table), Ok(()));
    }
    assert_eq!(
        builtin_nine_key_shuangpin_custom_profile(NineKeyShuangpinProfile::Custom),
        None
    );
    let mut table =
        builtin_nine_key_shuangpin_custom_profile(NineKeyShuangpinProfile::Migala3).unwrap();
    table.zero_initials.remove("er");
    let problem = nine_key_shuangpin_custom_profile_problem(&table).unwrap();
    assert_eq!(problem.kind, ShuangpinCustomProblemKind::MissingUnit);
    assert_eq!(problem.part, Some("zero_initials"));
    assert_eq!(problem.unit.as_deref(), Some("er"));
    assert!(validate_nine_key_shuangpin_custom_profile(&table)
        .unwrap_err()
        .contains("zero_initials unit \"er\" has no key"));
}

#[test]
fn builtin_shuangpin_tables_are_valid_custom_profiles() {
    for profile in [
        ShuangpinProfile::Xiaohe,
        ShuangpinProfile::Ziranma,
        ShuangpinProfile::Shoudao,
        ShuangpinProfile::Microsoft,
        ShuangpinProfile::SmartAbc,
    ] {
        let table = builtin_shuangpin_custom_profile(profile).unwrap();
        assert_eq!(
            (
                table.initials.len(),
                table.finals.len(),
                table.zero_initials.len()
            ),
            (3, 33, 12),
            "{profile:?}"
        );
        assert_eq!(
            custom_shuangpin_profile_problem(&table),
            None,
            "{profile:?}"
        );
        assert_eq!(
            validate_custom_shuangpin_profile(&table),
            Ok(()),
            "{profile:?}"
        );
    }
    assert_eq!(
        builtin_shuangpin_custom_profile(ShuangpinProfile::Xiaohe)
            .unwrap()
            .finals["ing"],
        "k"
    );
    assert_eq!(
        builtin_shuangpin_custom_profile(ShuangpinProfile::Microsoft)
            .unwrap()
            .finals["ing"],
        ";"
    );
    assert_eq!(
        builtin_shuangpin_custom_profile(ShuangpinProfile::Custom),
        None
    );
}

/// 设置页拿到的问题逐项对应 Engine 的错误，JSON 里只有这种错误用得到的字段。
#[test]
fn custom_shuangpin_problems_name_the_unit_key_and_code() {
    let edited = |edit: &dyn Fn(&mut msime_client_core::preferences::ShuangpinCustomProfile)| {
        let mut table = custom_shuangpin_table();
        edit(&mut table);
        let problem = custom_shuangpin_profile_problem(&table);
        // 不留存的检查与 MCP 保存前用的校验结论相同。
        assert_eq!(
            problem.is_none(),
            validate_custom_shuangpin_profile(&table).is_ok()
        );
        problem.map(|problem| serde_json::to_value(problem).unwrap())
    };
    assert_eq!(edited(&|_| {}), None);
    assert_eq!(
        edited(&|table| {
            table.initials.insert("b".into(), "b".into());
        }),
        Some(json!({ "kind": "unknown_unit", "part": "initials", "unit": "b" }))
    );
    assert_eq!(
        edited(&|table| {
            table.finals.remove("iong");
        }),
        Some(json!({ "kind": "missing_unit", "part": "finals", "unit": "iong" }))
    );
    assert_eq!(
        edited(&|table| {
            table.zero_initials.insert("a".into(), ";A".into());
        }),
        Some(json!({ "kind": "invalid_key", "part": "zero_initials", "unit": "a", "key": ";a" }))
    );
    assert_eq!(
        edited(&|table| {
            table.initials.insert("sh".into(), "s".into());
        }),
        Some(json!({ "kind": "initial_key_taken", "part": "initials", "unit": "sh", "key": "s" }))
    );
    assert_eq!(
        edited(&|table| {
            table.zero_initials.insert("a".into(), "ua".into());
        }),
        Some(json!({ "kind": "ambiguous_code", "code": "ua", "first": "sha", "second": "a" }))
    );
    // 偏好里的表是对象，同一个单位写不出两次；这一种只可能从 Engine 的表来，照样逐项对应。
    assert_eq!(
        serde_json::to_value(ShuangpinCustomProblem::from(
            ShuangpinCustomProfileError::DuplicateUnit {
                part: ShuangpinCustomTablePart::Finals,
                unit: "iu".into(),
            }
        ))
        .unwrap(),
        json!({ "kind": "duplicate_unit", "part": "finals", "unit": "iu" })
    );
}

#[test]
fn japanese_mode_switch_defers_and_restores_chinese_profile() {
    use msime_client_core::preferences::ChineseScheme;
    let dir = tempfile::tempdir().unwrap();
    let chinese = Preferences {
        scheme: InputScheme::Shuangpin,
        shuangpin_profile: ShuangpinProfile::Microsoft,
        last_chinese_scheme: Some(ChineseScheme::Shuangpin),
        ..chinese_preferences()
    };
    let handle = test_host_preferences(dir.path(), chinese.clone());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'b', false));
    read(msime_client_character(handle, b';', false));
    let japanese = Preferences {
        scheme: InputScheme::Japanese,
        touch_keyboard_layout: TouchKeyboardLayout::NineKey,
        ..chinese.clone()
    };
    let queued = update(handle, 1, &japanese);
    assert_eq!(queued["value"]["deferred"], true);
    assert_eq!(
        queued["value"]["view"]["touch_keyboard_layout"],
        "twenty_six_key"
    );
    let committed = read(msime_client_command(handle, 2));
    assert_eq!(committed["value"]["commit"], "b;");
    assert_eq!(committed["value"]["commit_context"]["scheme"], 1);
    assert_eq!(committed["value"]["view"]["scheme"], 3);
    assert_eq!(committed["value"]["view"]["nine_key"], false);
    assert_eq!(
        committed["value"]["view"]["touch_keyboard_layout"],
        "nine_key"
    );
    let kana = read(msime_client_character(handle, b'a', false));
    assert_eq!(kana["ok"], true);
    assert_eq!(kana["value"]["view"]["preedit"], "a");
    assert_eq!(kana["value"]["view"]["reading"], "あ");
    assert_eq!(kana["value"]["view"]["scheme"], 3);
    assert_eq!(kana["value"]["view"]["candidates"][0]["text"], "あ");
    assert_eq!(kana["value"]["view"]["candidates"][1]["text"], "ア");
    let small_kana = read(msime_client_command(handle, 10));
    assert_eq!(small_kana["value"]["handled"], true);
    assert_eq!(small_kana["value"]["view"]["reading"], "ぁ");
    let committed_small_kana = read(msime_client_command(handle, 11));
    assert_eq!(committed_small_kana["value"]["commit"], "ぁ");
    assert_eq!(committed_small_kana["value"]["view"]["reading"], "");
    read(msime_client_character(handle, b'a', false));
    let committed_kana = read(msime_client_command(handle, 11));
    assert_eq!(committed_kana["value"]["commit"], "あ");
    assert_eq!(committed_kana["value"]["view"]["reading"], "");
    read(msime_client_command(handle, 3));
    read(msime_client_character(handle, b'n', false));
    let syllable_separator = read(msime_client_character(handle, b'\'', false));
    assert_eq!(syllable_separator["ok"], true);
    assert_eq!(
        syllable_separator["value"]["view"]["candidates"][0]["text"],
        "ん"
    );
    assert_eq!(update(handle, 2, &chinese)["value"]["deferred"], true);
    read(msime_client_command(handle, 3));
    assert_eq!(
        read(msime_client_view(handle))["value"]["touch_keyboard_layout"],
        "twenty_six_key"
    );
    read(msime_client_character(handle, b'b', false));
    assert_eq!(
        read(msime_client_character(handle, b';', false))["value"]["view"]["editing_text"],
        "b;"
    );
    read(msime_client_destroy(handle));
}

#[test]
fn raw_commit_without_learning_crosses_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    for character in b"xq" {
        read(msime_client_character(handle, *character, false));
    }
    let committed = read(msime_client_command(handle, 15));
    assert_eq!(committed["ok"], true);
    assert_eq!(committed["value"]["commit"], "xq");
    assert_eq!(committed["value"]["view"]["editing_text"], "");
    read(msime_client_destroy(handle));
}

#[test]
fn japanese_commands_are_unhandled_for_non_japanese_schemes() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    let typed = read(msime_client_character(handle, b'a', false));
    assert_eq!(typed["value"]["view"]["reading"], "");

    let variant = read(msime_client_command(handle, 10));
    assert_eq!(variant["value"]["handled"], false);
    assert_eq!(variant["value"]["view"]["editing_text"], "a");
    assert_eq!(variant["value"]["view"]["reading"], "");

    let reading = read(msime_client_command(handle, 11));
    assert_eq!(reading["value"]["handled"], false);
    assert!(reading["value"]["commit"].is_null());
    assert_eq!(reading["value"]["view"]["reading"], "");
    read(msime_client_destroy(handle));
}

#[test]
fn japanese_commands_apply_to_the_twenty_six_key_scheme() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            scheme: InputScheme::Japanese,
            touch_keyboard_layout: TouchKeyboardLayout::TwentySixKey,
            ..chinese_preferences()
        },
    );
    read(msime_client_focus(handle, true));
    let typed = read(msime_client_character(handle, b'a', false));
    assert_eq!(
        typed["value"]["view"]["touch_keyboard_layout"],
        "twenty_six_key"
    );
    assert_eq!(typed["value"]["view"]["reading"], "あ");

    let variant = read(msime_client_command(handle, 10));
    assert_eq!(variant["value"]["handled"], true);
    assert_eq!(variant["value"]["view"]["reading"], "ぁ");
    let committed = read(msime_client_command(handle, 11));
    assert_eq!(committed["value"]["commit"], "ぁ");
    assert_eq!(committed["value"]["view"]["reading"], "");
    read(msime_client_destroy(handle));
}

// 藏文经过宿主边界：大写字母是拼写，组字时显示转换后的藏文，空格加音节点、`/` 加垂符上屏，回车不加音节点，拼写符号随组字状态变化。
#[test]
fn tibetan_scheme_crosses_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            scheme: InputScheme::Tibetan,
            ..chinese_preferences()
        },
    );
    read(msime_client_focus(handle, true));
    let idle = read(msime_client_command(handle, 3));
    assert_eq!(idle["value"]["view"]["spelling_symbols"], "'/");
    let mut typed = Value::Null;
    for character in b"bkra" {
        typed = read(msime_client_character(handle, *character, false));
        assert_eq!(typed["value"]["handled"], true);
    }
    let view = &typed["value"]["view"];
    assert_eq!(view["scheme"], 8);
    assert_eq!(view["preedit"], "བཀྲ");
    assert_eq!(view["candidates"], json!([]));
    assert_eq!(view["spelling_symbols"], "'+-./");
    let space = read(msime_client_character(handle, b' ', false));
    assert_eq!(space["value"]["handled"], true);
    assert_eq!(space["value"]["commit"], "བཀྲ་");
    assert_eq!(space["value"]["commit_context"]["scheme"], 8);

    for character in b"shis" {
        read(msime_client_character(handle, *character, false));
    }
    let shad = read(msime_client_character(handle, b'/', false));
    assert_eq!(shad["value"]["handled"], true);
    assert_eq!(shad["value"]["commit"], "ཤིས།");
    let alone = read(msime_client_character(handle, b'/', false));
    assert_eq!(alone["value"]["handled"], true);
    assert_eq!(alone["value"]["commit"], "།");

    // 大写字母是拼写而不是 Shift 命令：`Ta` 是反写字母 ཊ。
    read(msime_client_character(handle, b'T', true));
    let retroflex = read(msime_client_character(handle, b'a', false));
    assert_eq!(retroflex["value"]["view"]["preedit"], "ཊ");
    let enter = read(msime_client_command(handle, 2));
    assert_eq!(enter["value"]["handled"], true);
    assert_eq!(enter["value"]["commit"], "ཊ");
    read(msime_client_destroy(handle));
}

/// 测试用的插件指令提供者：不论触发词和参数，交回固定的行。
#[cfg(any(
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos"))
))]
struct FixedPluginRows(Vec<(String, String)>);

#[cfg(any(
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos"))
))]
impl msime_input_runtime::CommandProvider for FixedPluginRows {
    fn provide(
        &mut self,
        _trigger: &str,
        _argument: &str,
        out: &mut Vec<(String, String)>,
    ) -> msime_input_runtime::CommandOutcome {
        out.extend(self.0.iter().cloned());
        msime_input_runtime::CommandOutcome::Ran
    }
}

/// 在 crate 内部给会话挂上插件指令 `/uuid` 和一个交回固定行的提供者，代替 host-api 以后经授权算出的那一份。
#[cfg(any(
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos"))
))]
fn attach_plugin_rows(handle: u64, rows: &[&str]) {
    let rows = rows
        .iter()
        .map(|text| ((*text).to_owned(), String::new()))
        .collect();
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let session = sessions.get_mut(&handle).unwrap();
        session
            .runtime
            .set_plugin_commands(&[msime_input_runtime::PluginCommand {
                trigger: "uuid".into(),
                title: "UUID".into(),
                extended: false,
            }])
            .unwrap();
        session
            .runtime
            .set_command_provider(Some(Box::new(FixedPluginRows(rows))));
    });
}

/// 打一串字母，返回最后的视图。
fn type_letters(handle: u64, keys: &[u8]) -> Value {
    let mut view = Value::Null;
    for key in keys {
        let result = read(msime_client_character(handle, *key, false));
        assert_eq!(result["ok"], true, "{result}");
        view = result["value"]["view"].clone();
    }
    view
}

/// 页上有脚本插件的行（来源 14）时整页不请求释义和翻译：不论开的是在线翻译、英文释义还是离线释义。对照组是同一偏好下的 `/rq`，它照常请求，确认拦下它的是插件行本身。
#[cfg(any(
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos"))
))]
#[test]
fn plugin_rows_ask_for_no_gloss_or_translation() {
    type Configure = fn(&mut Preferences);
    let variants: [(&str, Configure); 3] = [
        ("translation", |preferences| {
            preferences.candidate_translations = true;
        }),
        ("english gloss", |preferences| {
            preferences.candidate_english_gloss = true;
        }),
        ("offline gloss", |preferences| {
            preferences.candidate_english_gloss = true;
            preferences.translation_target_language =
                msime_client_core::preferences::TranslationTargetLanguage::Fr;
        }),
    ];
    for (label, configure) in variants {
        let dir = tempfile::tempdir().unwrap();
        offline_gloss_fixture(&dir.path().join("offline-glosses/zh-fr.db"), "fr");
        let mut preferences = chinese_preferences();
        preferences.local_modes.command = true;
        preferences.candidate_translations = false;
        preferences.candidate_english_gloss = false;
        preferences.tencent_tmt.enabled = false;
        configure(&mut preferences);
        let handle = test_host_preferences(dir.path(), preferences);
        assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
        attach_plugin_rows(handle, &["你好", "hello"]);

        let view = type_letters(handle, b"/rq");
        assert!(!view["candidates"].as_array().unwrap().is_empty());
        assert!(
            !read(msime_client_translation_query(handle))["value"].is_null(),
            "{label}: the control asks"
        );
        read(msime_client_command(handle, 3));

        let view = type_letters(handle, b"/uuid");
        assert_eq!(view["candidates"][0]["text"], "你好", "{label}");
        assert_eq!(view["candidates"][0]["source"], 14, "{label}");
        assert!(
            read(msime_client_translation_query(handle))["value"].is_null(),
            "{label}"
        );
        read(msime_client_destroy(handle));
    }
}

/// 全角输出开着时插件行原样上屏（半角 ASCII），候选窗里显示的就是上屏的；同一个会话里内置指令 `/rq` 的上屏照旧转全角。
#[cfg(any(
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos"))
))]
#[test]
fn plugin_rows_commit_half_width_in_full_width_mode() {
    let directory = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    preferences.local_modes.command = true;
    let handle = test_host_preferences(directory.path(), preferences);
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    read(msime_client_set_character_width(handle, true));
    attach_plugin_rows(handle, &["0f8fad5b-d9cb-469f"]);

    let view = type_letters(handle, b"/uuid");
    assert_eq!(view["candidates"][0]["text"], "0f8fad5b-d9cb-469f");
    let committed = read(msime_client_command(handle, 1));
    assert_eq!(committed["value"]["commit"], "0f8fad5b-d9cb-469f");
    assert_eq!(
        committed["value"]["commit_context"]["local_mode"],
        "command"
    );
    assert!(
        committed["value"]["commit_context"]
            .get("plugin_row")
            .is_none(),
        "the flag stays off the wire"
    );

    let view = type_letters(handle, b"/rq");
    let date = view["candidates"][0]["text"].as_str().unwrap().to_owned();
    let widened: String = date
        .chars()
        .map(|character| match character {
            '!'..='~' => char::from_u32(character as u32 + 0xfee0).unwrap(),
            ' ' => '\u{3000}',
            other => other,
        })
        .collect();
    assert!(date.chars().any(|character| character.is_ascii_digit()));
    let committed = read(msime_client_command(handle, 1));
    assert_eq!(committed["value"]["commit"], widened.as_str());
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

// 全角输出开着时，中文方案的上屏都转成全角，网址例外：上屏后 view 已回到 `none`，所以按上屏前的 `commit_context.local_mode` 豁免。
#[test]
fn url_commits_stay_half_width_in_full_width_mode() {
    let directory = tempfile::tempdir().unwrap();
    let handle = test_host(directory.path());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    read(msime_client_set_character_width(handle, true));

    // 对照：同样的 ASCII 字母不在网址模式时照旧变成全角。
    for character in b"abc" {
        read(msime_client_character(handle, *character, false));
    }
    let plain = read(msime_client_command(handle, 2));
    assert_eq!(plain["value"]["commit"], "ａｂｃ");

    for character in b"www" {
        read(msime_client_character(handle, *character, false));
    }
    let opened = read(msime_client_punctuation(handle, b'.'));
    assert!(opened["value"]["commit"].is_null());
    assert_eq!(opened["value"]["view"]["local_mode"], "url");
    for character in b"a1" {
        read(msime_client_character(handle, *character, false));
    }
    read(msime_client_punctuation(handle, b'/'));
    let committed = read(msime_client_command(handle, 2));
    assert_eq!(committed["value"]["commit"], "www.a1/");
    assert_eq!(committed["value"]["commit_context"]["local_mode"], "url");
    assert_eq!(committed["value"]["view"]["local_mode"], "none");

    // 空格上屏同样保持半角，末尾不带空格。
    for character in b"www" {
        read(msime_client_character(handle, *character, false));
    }
    read(msime_client_punctuation(handle, b'.'));
    read(msime_client_character(handle, b'a', false));
    let spaced = read(msime_client_command(handle, 1));
    assert_eq!(spaced["value"]["commit"], "www.a");
    assert_eq!(spaced["value"]["commit_context"]["local_mode"], "url");

    // 结束网址的英文标点不属于网址，和普通组字后的同一个键一样转全角。
    read(msime_client_set_chinese_punctuation(handle, false));
    for character in b"www" {
        read(msime_client_character(handle, *character, false));
    }
    read(msime_client_punctuation(handle, b'.'));
    read(msime_client_character(handle, b'a', false));
    let finished = read(msime_client_punctuation(handle, b'<'));
    assert_eq!(finished["value"]["commit_context"]["local_mode"], "url");
    assert_eq!(finished["value"]["commit"], "www.a\u{ff1c}");
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

/// 把一次合成的上屏交给 `complete_transition`，返回宿主拿到的上屏文本。`scheme` 和 `local_mode` 是上屏前记下的 `commit_context`。
fn completed_commit(
    handle: u64,
    commit: &str,
    scheme: u8,
    local_mode: &str,
    full_width: bool,
) -> Option<String> {
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let session = sessions.get_mut(&handle).unwrap();
        let mut view = session.runtime.view();
        if full_width {
            view.character_width = CharacterWidth::Fullwidth;
        }
        session
            .complete_transition(Transition {
                handled: true,
                commit: Some(commit.to_owned()),
                commit_cursor_suffix: None,
                commit_context: Some(msime_input_runtime::OutputContext {
                    scheme,
                    local_mode: local_mode.to_owned(),
                    script_conversion: true,
                    typing_statistics: true,
                    plugin_row: false,
                }),
                diagnostic: None,
                view,
            })
            .commit
    })
}

// 「中英文之间加空格」只在开关打开、中文方案、普通组字或快捷短语、超级简拼的上屏里加；网址、日期等快捷模式和非中文方案原样。关着时上屏一字不改。
#[test]
fn chinese_english_spacing_follows_the_switch_and_the_commit_mode() {
    let directory = tempfile::tempdir().unwrap();
    let handle = test_host(directory.path());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    let text = "用Rust写插件";
    assert_eq!(
        completed_commit(handle, text, 0, "none", false).as_deref(),
        Some(text)
    );

    let preferences = Preferences {
        chinese_english_spacing: true,
        ..chinese_preferences()
    };
    assert_eq!(update(handle, 1, &preferences)["ok"], true);
    for local_mode in ["none", "quick_phrase", "super_jianpin"] {
        assert_eq!(
            completed_commit(handle, text, 0, local_mode, false).as_deref(),
            Some("用 Rust 写插件"),
            "{local_mode}"
        );
    }
    // 五笔和粤拼同样是中文方案。
    for scheme in [2, 5] {
        assert_eq!(
            completed_commit(handle, "800人", scheme, "none", false).as_deref(),
            Some("800 人"),
            "{scheme}"
        );
    }
    for local_mode in [
        "url",
        "date_time",
        "command",
        "expression",
        "mention",
        "unicode",
        "temporary_english",
        "temporary_japanese",
    ] {
        assert_eq!(
            completed_commit(handle, text, 0, local_mode, false).as_deref(),
            Some(text),
            "{local_mode}"
        );
    }
    // 日文、韩文方案不加。
    for scheme in [3, 4] {
        assert_eq!(
            completed_commit(handle, text, scheme, "none", false).as_deref(),
            Some(text),
            "{scheme}"
        );
    }
    // 全角在前：字母已是全角，不加空格，也就不会出现 U+3000。
    let wide = completed_commit(handle, text, 0, "none", true).unwrap();
    assert_eq!(wide, "用Ｒｕｓｔ写插件");
    assert!(!wide.contains('\u{3000}'));

    let off = Preferences {
        chinese_english_spacing: false,
        ..preferences
    };
    assert_eq!(update(handle, 2, &off)["ok"], true);
    assert_eq!(
        completed_commit(handle, text, 0, "none", false).as_deref(),
        Some(text)
    );
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

// 端到端：K 模式快捷短语「用Rust写」经完整的派发链路上屏为「用 Rust 写」。
#[test]
fn korean_scheme_crosses_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            scheme: InputScheme::Korean,
            ..chinese_preferences()
        },
    );
    read(msime_client_focus(handle, true));
    for character in b"gk" {
        read(msime_client_character(handle, *character, false));
    }
    let typed = read(msime_client_character(handle, b's', false));
    assert_eq!(typed["value"]["handled"], true);
    assert!(typed["value"]["commit"].is_null());
    let view = &typed["value"]["view"];
    assert_eq!(view["scheme"], 4);
    assert_eq!(view["preedit"], "한");
    assert_eq!(view["reading"], "한");
    assert_eq!(view["editing_text"], "gks");
    assert_eq!(view["caret_position"], 3);
    assert_eq!(view["candidates"], json!([]));
    assert_eq!(view["page_count"], 0);
    assert_eq!(
        read(msime_client_online_query(handle))["value"],
        Value::Null
    );

    // The next syllable's first key commits the previous one in the same transition.
    let next = read(msime_client_character(handle, b'r', false));
    assert_eq!(next["value"]["handled"], true);
    assert_eq!(next["value"]["commit"], "한");
    assert_eq!(next["value"]["commit_context"]["scheme"], 4);
    assert_eq!(next["value"]["view"]["reading"], "ㄱ");

    // Shift selects the double consonant.
    read(msime_client_command(handle, 0));
    let double = read(msime_client_character(handle, b'R', true));
    assert_eq!(double["value"]["view"]["local_mode"], "none");
    assert_eq!(double["value"]["view"]["reading"], "ㄲ");

    // Space commits the syllable and is left for the host to insert.
    let space = read(msime_client_command(handle, 1));
    assert_eq!(space["value"]["handled"], false);
    assert_eq!(space["value"]["commit"], "ㄲ");
    assert_eq!(space["value"]["view"]["editing_text"], "");
    // Paging has nothing to page and leaves the key to the host.
    let page = read(msime_client_command(handle, 100));
    assert_eq!(page["value"]["handled"], false);

    // Punctuation is half-width ASCII even with Chinese punctuation on and full-width output selected.
    read(msime_client_set_character_width(handle, true));
    for character in b"rk" {
        read(msime_client_character(handle, *character, false));
    }
    let period = read(msime_client_punctuation_with_context(handle, b'.', 0));
    assert_eq!(period["value"]["handled"], true);
    assert_eq!(period["value"]["commit"], "가.");
    let idle = read(msime_client_punctuation_with_context(handle, b',', 0));
    assert_eq!(idle["value"]["handled"], false);
    assert!(idle["value"]["commit"].is_null());
    let arm = json!({"ascii": b'.', "commit": ".", "timestamp_ms": 1, "editor_generation": 1, "auto_closed_pair": false}).to_string();
    let armed =
        read(unsafe { msime_client_smart_punctuation_arm(handle, arm.as_ptr(), arm.len()) });
    assert!(armed["value"]["repeat"].is_null());

    // Leaving the client commits the open syllable instead of dropping it.
    for character in b"rk" {
        read(msime_client_character(handle, *character, false));
    }
    let left = read(msime_client_focus(handle, false));
    assert_eq!(left["value"]["commit"], "가");
    assert_eq!(left["value"]["view"]["editing_text"], "");

    // The dedicated English mode keeps its own rules under the Korean scheme: full-width output widens its commits.
    read(msime_client_focus(handle, true));
    read(msime_client_set_english_mode(handle, true));
    for character in b"ab" {
        read(msime_client_character(handle, *character, false));
    }
    let english = read(msime_client_command(handle, 2));
    assert_eq!(english["value"]["commit"], "ａｂ");
    read(msime_client_destroy(handle));
}

/// MSIME_CONVERT_HANJA (16) reaches the Korean engine through the runtime: it opens the composing syllable's Hanja list, the ordinary candidate commands choose from it, and Cancel and Finish keep the Hangul.
#[test]
fn korean_hanja_conversion_crosses_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            scheme: InputScheme::Korean,
            ..chinese_preferences()
        },
    );
    read(msime_client_focus(handle, true));
    let type_keys = |keys: &[u8]| {
        for character in keys {
            read(msime_client_character(handle, *character, false));
        }
    };

    type_keys(b"gks");
    let opened = read(msime_client_command(handle, 16));
    assert_eq!(opened["value"]["handled"], true);
    assert!(opened["value"]["commit"].is_null());
    let view = &opened["value"]["view"];
    assert_eq!(view["scheme"], 4);
    assert_eq!(view["preedit"], "한");
    assert_eq!(view["candidates"][0]["text"], "韓");
    assert_eq!(
        view["candidates"][0]["annotation"],
        "나라 이름 한, 한나라 한"
    );
    assert_eq!(view["candidates"][1]["text"], "漢");
    assert!(view["page_count"].as_u64().unwrap() > 1);

    // The ordinary candidate commands: next candidate, then commit the highlighted one.
    read(msime_client_command(handle, 102));
    let chosen = read(msime_client_command(handle, 1));
    assert_eq!(chosen["value"]["handled"], true);
    assert_eq!(chosen["value"]["commit"], "漢");
    assert_eq!(chosen["value"]["commit_context"]["scheme"], 4);
    assert_eq!(chosen["value"]["view"]["editing_text"], "");

    // Cancel closes the list and keeps the syllable; the trigger reopens it and Finish commits the Hangul.
    type_keys(b"gks");
    read(msime_client_command(handle, 16));
    let cancelled = read(msime_client_command(handle, 3));
    assert_eq!(cancelled["value"]["handled"], true);
    assert_eq!(cancelled["value"]["view"]["preedit"], "한");
    assert_eq!(cancelled["value"]["view"]["candidates"], json!([]));
    read(msime_client_command(handle, 16));
    read(msime_client_command(handle, 102));
    let finished = read(msime_client_command(handle, 9));
    assert_eq!(finished["value"]["commit"], "한");
    assert_eq!(finished["value"]["view"]["candidates"], json!([]));

    // A lone jamo has no Hanja: the command is unhandled and the jamo keeps composing.
    type_keys(b"r");
    let refused = read(msime_client_command(handle, 16));
    assert_eq!(refused["value"]["handled"], false);
    assert!(refused["value"]["commit"].is_null());
    assert_eq!(refused["value"]["view"]["preedit"], "ㄱ");

    // 17 和 18 是整句改字的光标键，韩文里进不了改字，与字母光标键一样写出音节；19 是宿主自己上屏后的取消（MSIME_CANCEL_AFTER_HOST_COMMIT），20 是宿主替失焦结束组字（MSIME_FINISH_ON_FOCUS_LOSS），是导航命令前的最后一个，下一个编号仍未定义。
    let left = read(msime_client_command(handle, 17));
    assert_eq!(left["value"]["handled"], false);
    assert_eq!(left["value"]["commit"], "ㄱ");
    assert_eq!(
        read(msime_client_command(handle, 21))["error"],
        "unknown input command"
    );
    read(msime_client_destroy(handle));
}

#[test]
fn handwriting_layout_is_exposed_only_after_pending_composition_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'n', false));
    read(msime_client_character(handle, b'i', false));
    let handwriting = Preferences {
        touch_keyboard_layout: TouchKeyboardLayout::Handwriting,
        ..Preferences::default()
    };
    let queued = update(handle, 1, &handwriting);
    assert_eq!(queued["value"]["deferred"], true);
    assert_eq!(
        queued["value"]["view"]["touch_keyboard_layout"],
        "twenty_six_key"
    );
    let finished = read(msime_client_command(handle, 9));
    assert_eq!(finished["value"]["commit"], "ni");
    assert_eq!(
        finished["value"]["view"]["touch_keyboard_layout"],
        "handwriting"
    );
    assert_eq!(finished["value"]["view"]["nine_key"], false);
    read(msime_client_destroy(handle));
}

/// `word` 在各移动宿主所画 26 键布局上的滑行请求，单位是像素、键为 80 x 120：每八像素、8 毫秒一个点。
fn glide_request(word: &str) -> String {
    let rows: [(&[u8], f32); 3] = [(b"qwertyuiop", 0.0), (b"asdfghjkl", 0.5), (b"zxcvbnm", 1.5)];
    let mut keys = [[0.0f32; 2]; 26];
    for (row, (letters, indent)) in rows.iter().enumerate() {
        for (column, &letter) in letters.iter().enumerate() {
            keys[usize::from(letter - b'a')] = [
                (column as f32 + indent + 0.5) * 80.0,
                (row as f32 + 0.5) * 120.0,
            ];
        }
    }
    let mut points = Vec::new();
    for pair in word.as_bytes().windows(2) {
        let [ax, ay] = keys[usize::from(pair[0] - b'a')];
        let [bx, by] = keys[usize::from(pair[1] - b'a')];
        let steps = (((bx - ax).powi(2) + (by - ay).powi(2)).sqrt() / 8.0)
            .ceil()
            .max(1.0);
        for step in 0..steps as usize {
            let t = step as f32 / steps;
            points.push([
                ax + (bx - ax) * t,
                ay + (by - ay) * t,
                points.len() as f32 * 8.0,
            ]);
        }
    }
    let [x, y] = keys[usize::from(word.as_bytes()[word.len() - 1] - b'a')];
    points.push([x, y, points.len() as f32 * 8.0]);
    json!({"keys": keys, "key_width": 80.0, "key_height": 120.0, "points": points}).to_string()
}

/// 共享偏好 `mixed_input.symbols` 决定 Engine 的 `mixed_symbols`：缺键读作开（出厂值），打 `zhengfu` 时 ± 接在 正负 后面；关掉（不在组字时立即生效）不再出现。
#[test]
fn symbol_candidates_follow_the_mixed_input_preference() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    assert!(preferences.mixed_input.symbols);
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    let typed = |handle: u64| {
        let mut view = Value::Null;
        for byte in b"zhengfu" {
            view = read(msime_client_character(handle, *byte, false))["value"]["view"].clone();
        }
        view["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|candidate| candidate["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let texts = typed(handle);
    assert_eq!(texts[..3], ["政府", "正负", "±"], "{texts:?}");
    SESSIONS.with(|sessions| assert!(sessions.borrow()[&handle].options.mixed_symbols));
    read(msime_client_command(handle, 3));

    preferences.mixed_input.symbols = false;
    assert_eq!(update(handle, 1, &preferences)["ok"], true);
    let texts = typed(handle);
    assert!(!texts.contains(&"±".to_owned()), "{texts:?}");
    SESSIONS.with(|sessions| assert!(!sessions.borrow()[&handle].options.mixed_symbols));
    read(msime_client_destroy(handle));
}

#[test]
fn a_glide_stroke_composes_through_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_with_pinyin_fixture(dir.path(), chinese_preferences());
    read(msime_client_focus(handle, true));
    let request = glide_request("nihao");
    let glided = read(unsafe { msime_client_glide(handle, request.as_ptr(), request.len()) });
    assert_eq!(glided["ok"], true, "{glided}");
    assert_eq!(glided["value"]["handled"], true);
    assert_eq!(glided["value"]["view"]["editing_text"], "nihao");
    assert_eq!(glided["value"]["view"]["candidates"][0]["text"], "本地");
    read(msime_client_destroy(handle));
}

#[test]
fn a_bilingual_glide_request_preserves_default_requests_and_rejects_invalid_flags() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["resources", "dictionaries"] {
        std::fs::create_dir_all(dir.path().join(name)).unwrap();
        rusqlite::Connection::open(dir.path().join(name).join("msime-english.db")).unwrap()
            .execute_batch("CREATE TABLE english_words(word TEXT,display TEXT,weight INTEGER); INSERT INTO english_words VALUES('hello','hello',1000);").unwrap();
    }
    let handle = test_host_with_pinyin_fixture(dir.path(), chinese_preferences());
    read(msime_client_focus(handle, true));
    let mut request: Value = serde_json::from_str(&glide_request("nihao")).unwrap();
    request["bilingual"] = json!(true);
    let encoded = request.to_string();
    let result = read(unsafe { msime_client_glide(handle, encoded.as_ptr(), encoded.len()) });
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"]["handled"], true);
    assert!(result["value"]["view"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["text"] == "本地"));
    request["bilingual"] = json!("true");
    let invalid = request.to_string();
    let rejected = read(unsafe { msime_client_glide(handle, invalid.as_ptr(), invalid.len()) });
    assert_eq!(rejected["ok"], false);
    let view = &result["value"]["view"];
    let index = view["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .position(|row| row["text"] == "本地")
        .unwrap();
    let selected = read(msime_client_select(
        handle,
        view["generation"].as_u64().unwrap(),
        index,
    ));
    assert_eq!(selected["value"]["commit"], "本地");
    let mut english: Value = serde_json::from_str(&glide_request("hello")).unwrap();
    english["bilingual"] = json!(true);
    let encoded = english.to_string();
    let result = read(unsafe { msime_client_glide(handle, encoded.as_ptr(), encoded.len()) });
    let view = &result["value"]["view"];
    let index = view["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .position(|row| row["text"] == "hello")
        .unwrap();
    let selected = read(msime_client_select(
        handle,
        view["generation"].as_u64().unwrap(),
        index,
    ));
    assert_eq!(selected["value"]["commit"], "hello ");
    read(msime_client_destroy(handle));
}

/// 26 个字母键排成一行，键宽 10、键高 10，`id` 是代次。
fn touch_geometry_json(id: u32) -> String {
    let keys: Vec<Value> = (0..26)
        .map(|index| json!([f64::from(index) * 10.0, 0.0, 10.0, 10.0]))
        .collect();
    json!({"id": id, "keys": keys, "semicolon": null, "duplicates": []}).to_string()
}

fn set_key_geometry(handle: u64, json: &str) -> Value {
    read(unsafe { msime_client_set_key_geometry(handle, json.as_ptr(), json.len()) })
}

/// 去掉只属于某一个会话的字段（候选 ID 里的会话句柄），剩下的就是两条路径都该逐字节相同的回应。
fn without_session(mut value: Value) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(fields) => {
                fields.remove("session");
                fields.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut value);
    value
}

/// 触点入口退回 `msime_client_character`：代次不符、坐标不是有限数时回应逐字节相同，ABI 仍是 3。
#[test]
fn touch_characters_without_usable_touches_match_plain_characters() {
    assert_eq!(msime_client_abi_version(), 3);
    let preferences = Preferences {
        scheme: InputScheme::Shuangpin,
        ..chinese_preferences()
    };
    let plain_dir = tempfile::tempdir().unwrap();
    let touched_dir = tempfile::tempdir().unwrap();
    let plain = test_host_with_pinyin_fixture(plain_dir.path(), preferences.clone());
    let touched = test_host_with_pinyin_fixture(touched_dir.path(), preferences);
    read(msime_client_focus(plain, true));
    read(msime_client_focus(touched, true));
    assert_eq!(
        set_key_geometry(touched, &touch_geometry_json(4))["ok"],
        true
    );
    for (byte, geometry_id, x) in [
        (b'n', 5, 130.0),
        (b'i', 4, f32::NAN),
        (b'h', 4, f32::INFINITY),
        (b'c', 3, 20.0),
    ] {
        let expected = read(msime_client_character(plain, byte, false));
        let actual = read(msime_client_touch_character(
            touched,
            byte,
            false,
            geometry_id,
            x,
            1.0,
        ));
        assert_eq!(
            without_session(actual),
            without_session(expected),
            "{}",
            byte as char
        );
    }
    read(msime_client_destroy(plain));
    read(msime_client_destroy(touched));
}

/// 键位几何的边界：不合格的 JSON 整份拒绝，空串清除；带触点的按键照常组字。
#[test]
fn key_geometry_is_validated_at_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Shuangpin,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    let valid = touch_geometry_json(4);
    assert_eq!(set_key_geometry(handle, &valid)["ok"], true);
    let mut extra: Value = serde_json::from_str(&valid).unwrap();
    extra["scale"] = json!(2);
    let mut short: Value = serde_json::from_str(&valid).unwrap();
    short["keys"].as_array_mut().unwrap().pop();
    let mut flat: Value = serde_json::from_str(&valid).unwrap();
    flat["keys"][0][2] = json!(0.0);
    // 每个数都有限，归一后的键心却溢出成无穷（键宽 1e-3、键心 1e36）；距离平方会溢出的远键也一样拒绝。
    let mut overflowing: Value = serde_json::from_str(&valid).unwrap();
    for key in overflowing["keys"].as_array_mut().unwrap() {
        key[2] = json!(1.0e-3);
    }
    overflowing["keys"][0][0] = json!(1.0e36);
    let mut far: Value = serde_json::from_str(&valid).unwrap();
    far["keys"][0][0] = json!(1.0e30);
    for rejected in [
        extra.to_string(),
        short.to_string(),
        flat.to_string(),
        overflowing.to_string(),
        far.to_string(),
        "[]".to_owned(),
    ] {
        assert_eq!(
            set_key_geometry(handle, &rejected)["ok"],
            false,
            "{rejected}"
        );
    }
    let oversized = format!("{valid}{}", " ".repeat(16 * 1024));
    assert_eq!(set_key_geometry(handle, &oversized)["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_set_key_geometry(handle, std::ptr::null(), 4) })["ok"],
        false
    );
    let typed = read(msime_client_touch_character(
        handle, b'n', false, 4, 131.0, 1.0,
    ));
    assert_eq!(typed["ok"], true, "{typed}");
    assert_eq!(typed["value"]["handled"], true);
    assert_eq!(typed["value"]["view"]["editing_text"], "n");
    // 空串和 `id` 为 0 都是清除。
    assert_eq!(
        read(unsafe { msime_client_set_key_geometry(handle, std::ptr::null(), 0) })["ok"],
        true
    );
    assert_eq!(
        set_key_geometry(handle, &touch_geometry_json(0))["ok"],
        true
    );
    read(msime_client_destroy(handle));
}

/// 视图 JSON 里候选的 `confident_correction` 只在按首选规则坐上第一位的双拼触点纠错行上出现（#10018 PR3）；默认参数（m_sp 无穷大）下带触点打字，视图里没有一行带这个字段，与加字段之前逐字节相同。
#[test]
fn touched_views_carry_no_confident_correction_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Shuangpin,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    assert_eq!(
        set_key_geometry(handle, &touch_geometry_json(4))["ok"],
        true
    );
    let mut view = Value::Null;
    for byte in *b"nihc" {
        let x = f32::from(byte - b'a') * 10.0 + 1.0;
        let typed = read(msime_client_touch_character(handle, byte, false, 4, x, 1.0));
        assert_eq!(typed["ok"], true, "{typed}");
        view = typed["value"]["view"].clone();
    }
    let candidates = view["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty(), "{view}");
    for candidate in candidates {
        assert!(
            candidate.get("confident_correction").is_none(),
            "{candidate}"
        );
    }
    read(msime_client_destroy(handle));
}

#[test]
fn a_malformed_glide_request_fails_without_touching_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_with_pinyin_fixture(dir.path(), chinese_preferences());
    read(msime_client_focus(handle, true));
    let valid: Value = serde_json::from_str(&glide_request("nihao")).unwrap();
    let mut short = valid.clone();
    short["keys"].as_array_mut().unwrap().pop();
    let mut flat = valid.clone();
    flat["key_width"] = json!(0.0);
    let mut lonely = valid.clone();
    lonely["points"] = json!([[1.0, 2.0]]);
    let mut odd = valid.clone();
    odd["points"][1] = json!([1.0]);
    let mut extra = valid;
    extra["pressure"] = json!(1);
    for request in [short, flat, lonely, odd, extra] {
        let request = request.to_string();
        let failed = read(unsafe { msime_client_glide(handle, request.as_ptr(), request.len()) });
        assert_eq!(failed["ok"], false, "{request}");
    }
    assert_eq!(
        read(unsafe { msime_client_glide(handle, std::ptr::null(), 4) })["ok"],
        false
    );
    assert_eq!(read(msime_client_view(handle))["value"]["editing_text"], "");
    read(msime_client_destroy(handle));
}

#[test]
fn nine_key_filters_cross_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_set_nine_key_mode(handle, true));
    let idle = read(unsafe { msime_client_set_nine_key_filter(handle, true, std::ptr::null(), 0) });
    assert_eq!(idle["value"]["handled"], false, "nothing composing");
    read(msime_client_character(handle, b'6', false));
    let filtered =
        read(unsafe { msime_client_set_nine_key_filter(handle, true, std::ptr::null(), 0) });
    assert_eq!(filtered["value"]["handled"], true);
    assert_eq!(filtered["value"]["view"]["nine_key_single_character"], true);
    assert_eq!(filtered["value"]["view"]["nine_key_strokes"], "");
    let wrong = b"q";
    let rejected =
        read(unsafe { msime_client_set_nine_key_filter(handle, false, wrong.as_ptr(), 1) });
    assert_eq!(rejected["value"]["handled"], false, "not a stroke");
    assert_eq!(
        read(unsafe { msime_client_set_nine_key_filter(handle, false, std::ptr::null(), 2) })["ok"],
        false
    );
    let long = [b'h'; 65];
    assert_eq!(
        read(unsafe { msime_client_set_nine_key_filter(handle, false, long.as_ptr(), long.len()) })
            ["ok"],
        false
    );
    let cancelled = read(msime_client_command(handle, 3));
    assert_eq!(
        cancelled["value"]["view"]["nine_key_single_character"],
        false
    );
    read(msime_client_destroy(handle));
}

#[test]
fn nine_key_mode_and_spelling_identity_cross_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    let enabled = read(msime_client_set_nine_key_mode(handle, true));
    assert_eq!(enabled["value"]["nine_key"], true);
    let typed = read(msime_client_character(handle, b'6', false));
    assert_eq!(typed["value"]["handled"], true);
    let view = &typed["value"]["view"];
    let generation = view["generation"].as_u64().unwrap();
    assert!(!view["nine_key_spellings"].as_array().unwrap().is_empty());
    assert_eq!(
        read(msime_client_choose_nine_key_spelling(
            handle,
            generation - 1,
            0
        ))["ok"],
        false
    );
    let selected = read(msime_client_choose_nine_key_spelling(handle, generation, 0));
    assert_eq!(selected["value"]["handled"], true);
    assert_eq!(
        read(msime_client_set_nine_key_mode(handle, false))["ok"],
        false
    );
    read(msime_client_command(handle, 3));
    assert_eq!(
        read(msime_client_set_nine_key_mode(handle, false))["value"]["nine_key"],
        false
    );

    let mut preferences = Preferences {
        candidate_page_size: 4,
        touch_keyboard_layout: TouchKeyboardLayout::NineKey,
        ..Preferences::default()
    };
    assert_eq!(
        update(handle, 1, &preferences)["value"]["view"]["nine_key"],
        true
    );
    preferences.learning = false;
    assert_eq!(
        update(handle, 2, &preferences)["value"]["view"]["nine_key"],
        true
    );
    preferences.touch_keyboard_layout = TouchKeyboardLayout::TwentySixKey;
    assert_eq!(
        update(handle, 3, &preferences)["value"]["view"]["nine_key"],
        false
    );
    preferences.touch_keyboard_layout = TouchKeyboardLayout::Handwriting;
    assert_eq!(
        update(handle, 4, &preferences)["value"]["view"]["touch_keyboard_layout"],
        "handwriting"
    );
    assert_eq!(read(msime_client_view(handle))["value"]["nine_key"], false);
    assert_eq!(
        read(msime_client_set_nine_key_mode(handle, true))["value"]["nine_key"],
        true
    );
    preferences.candidate_page_size = 3;
    assert_eq!(
        update(handle, 5, &preferences)["value"]["view"]["nine_key"],
        true
    );
    preferences.scheme = InputScheme::Wubi;
    assert_eq!(
        update(handle, 6, &preferences)["value"]["view"]["nine_key"],
        false
    );
    assert_eq!(
        read(msime_client_set_nine_key_mode(handle, true))["ok"],
        false
    );
    read(msime_client_destroy(handle));

    let persisted_dir = tempfile::tempdir().unwrap();
    let persisted = test_host_preferences(
        persisted_dir.path(),
        Preferences {
            touch_keyboard_layout: TouchKeyboardLayout::NineKey,
            ..Preferences::default()
        },
    );
    assert_eq!(
        read(msime_client_view(persisted))["value"]["nine_key"],
        true
    );
    read(msime_client_destroy(persisted));
}

/// 14 键经宿主接口开关：`View.key_grid` 说出网格，`nine_key` 只表示九键。组码只从 `msime_client_grid_key` 进来，硬件字母照常走全拼；选拼写不被当成过期请求。
#[test]
fn fourteen_key_grid_crosses_the_host_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    assert_eq!(read(msime_client_view(handle))["value"]["key_grid"], "none");
    assert_eq!(
        read(msime_client_set_key_grid(handle, u8::MAX))["ok"],
        false
    );
    assert_eq!(read(msime_client_view(handle))["value"]["key_grid"], "none");
    let enabled = read(msime_client_set_key_grid(handle, 2));
    assert_eq!(enabled["value"]["key_grid"], "fourteen_key");
    assert_eq!(enabled["value"]["nine_key"], false);

    assert_eq!(enabled["value"]["grid_composing"], false);

    // 网格开着，硬件字母照样走 26 键全拼：`grid_composing` 为假，宿主据此按 26 键的退格画光标。
    let hardware = read(msime_client_character(handle, b'q', false));
    assert_eq!(hardware["value"]["handled"], true);
    assert_eq!(hardware["value"]["view"]["preedit"], "q");
    assert_eq!(hardware["value"]["view"]["key_grid"], "fourteen_key");
    assert_eq!(hardware["value"]["view"]["grid_composing"], false);
    assert_eq!(
        read(msime_client_grid_key(handle, b'n'))["value"]["handled"],
        false
    );
    read(msime_client_command(handle, 3));

    let typed = read(msime_client_grid_key(handle, b'n'));
    assert_eq!(typed["value"]["handled"], true, "{typed}");
    let view = &typed["value"]["view"];
    assert_eq!(view["grid_composing"], true);
    assert!(!view["nine_key_spellings"].as_array().unwrap().is_empty());
    let generation = view["generation"].as_u64().unwrap();
    let selected = read(msime_client_choose_nine_key_spelling(handle, generation, 0));
    assert_eq!(selected["value"]["handled"], true, "{selected}");
    assert_eq!(read(msime_client_set_key_grid(handle, 0))["ok"], false);
    read(msime_client_command(handle, 3));
    assert_eq!(
        read(msime_client_set_key_grid(handle, 0))["value"]["key_grid"],
        "none"
    );

    let nine_key = read(msime_client_set_nine_key_mode(handle, true));
    assert_eq!(nine_key["value"]["key_grid"], "nine_key");
    let letters = read(msime_client_character(handle, b'n', false));
    assert_eq!(letters["value"]["view"]["preedit"], "n", "{letters}");
    assert_eq!(letters["value"]["view"]["grid_composing"], false);
    read(msime_client_command(handle, 3));
    let digits = read(msime_client_character(handle, b'6', false));
    assert_eq!(digits["value"]["view"]["grid_composing"], true, "{digits}");
    read(msime_client_command(handle, 3));
    assert_eq!(
        read(msime_client_grid_key(handle, b'n'))["value"]["handled"],
        false
    );
    read(msime_client_set_key_grid(handle, 2));
    assert_eq!(
        read(msime_client_set_nine_key_mode(handle, false))["value"]["key_grid"],
        "none"
    );
    read(msime_client_destroy(handle));
}

/// 设置宿主看不到输入法实际加载了哪份词库：建会话时把资源目录记进词库更新目录，从没装过更新时不建这个目录。
#[test]
fn creating_a_session_records_the_resources_it_loaded_for_dictionary_updates() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let resources = path("resources");
    let dictionaries = path("dictionaries");
    let state = path("state");
    let fixture = "CREATE TABLE tbl_1_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_n VALUES('ni','n','你',100);
                   CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);
                   CREATE TABLE quick_parases(key TEXT,value TEXT,weight INTEGER);
                   CREATE INDEX idx_quick_parases_key_weight ON quick_parases(key,weight DESC);";
    for directory in [&resources, &dictionaries] {
        rusqlite::Connection::open(directory.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(fixture)
            .unwrap();
    }
    let create = || {
        let options = json!({ "api_version": 1, "resources": resources, "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries, "preferences_directory": state, "preferences": chinese_preferences() }).to_string();
        let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
        assert_eq!(created["ok"], true, "{created}");
        read(msime_client_destroy(
            created["value"]["session"].as_u64().unwrap(),
        ));
    };
    let updates = state.join("dictionary-updates");
    create();
    assert!(!updates.exists());

    std::fs::create_dir(&updates).unwrap();
    create();
    let loaded: Value =
        serde_json::from_slice(&std::fs::read(updates.join("loaded.json")).unwrap()).unwrap();
    assert_eq!(
        Path::new(loaded["resources"].as_str().unwrap()),
        std::fs::canonicalize(&resources).unwrap()
    );
}

/// 会话用下载的词库代次时，从建好到销毁一直占用它：设置宿主回收旧版本要取同一把锁的独占，取不到就不删。
#[test]
fn a_session_on_a_downloaded_generation_holds_it_until_destroyed() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let state = path("state");
    let generation = "d".repeat(64);
    let resources = path(&format!(
        "state/dictionary-updates/generations/{generation}"
    ));
    let dictionaries = path("dictionaries");
    let fixture = "CREATE TABLE tbl_1_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_n VALUES('ni','n','你',100);
                   CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);
                   CREATE TABLE quick_parases(key TEXT,value TEXT,weight INTEGER);
                   CREATE INDEX idx_quick_parases_key_weight ON quick_parases(key,weight DESC);";
    for directory in [&resources, &dictionaries] {
        rusqlite::Connection::open(directory.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(fixture)
            .unwrap();
    }
    let options = json!({ "api_version": 1, "resources": resources, "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries, "preferences_directory": state, "preferences": chinese_preferences() }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let lock = state.join(format!("dictionary-updates/generations/{generation}.lock"));
    let removable = || {
        msime_client_core::file_lock::try_exclusive(
            &msime_client_core::file_lock::open_private_lock_file(&lock).unwrap(),
        )
        .unwrap()
    };
    assert!(!removable());
    read(msime_client_destroy(
        created["value"]["session"].as_u64().unwrap(),
    ));
    assert!(removable());
}

/// 三端宿主都开着 `phrase_preedit`。14 键里选了只覆盖前一段的词，退格到只剩一个组码再退格，收回这次选词：组码要回到 14 键网格里，而不是变成全拼 26 键的 `bu`，之后的 14 键照样能打。
#[test]
fn retreating_a_fourteen_key_selection_keeps_composing_on_the_grid() {
    check_letter_key_selection_retreat(false, false, false);
    check_letter_key_selection_retreat(true, false, false);
}

#[test]
fn eighteen_key_shuangpin_selection_retreat_keeps_composing_on_the_grid() {
    check_letter_key_selection_retreat(true, true, false);
}

#[test]
fn eighteen_key_exact_selection_retreat_preserves_letter_constraints() {
    check_letter_key_selection_retreat(true, true, true);
}

fn check_letter_key_selection_retreat(shuangpin: bool, eighteen: bool, exact: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let resources = path("resources");
    let dictionaries = path("dictionaries");
    let fixture = "CREATE TABLE tbl_1_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_n VALUES('ni','n','你',100);
                   CREATE TABLE tbl_1_b(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_b VALUES('bu','b','不',90);
                   CREATE TABLE tbl_1_g(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_g VALUES('gao','g','高',80);
                   CREATE TABLE tbl_1_h(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_h VALUES('hao','h','好',100);
                   CREATE TABLE tbl_2_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_2_n VALUES('ni''hao','nh','你好',1000);
                   CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);
                   CREATE TABLE quick_parases(key TEXT,value TEXT,weight INTEGER);
                   CREATE INDEX idx_quick_parases_key_weight ON quick_parases(key,weight DESC);";
    for directory in [&resources, &dictionaries] {
        rusqlite::Connection::open(directory.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(fixture)
            .unwrap();
    }
    let preferences = Preferences {
        scheme: if shuangpin {
            InputScheme::Shuangpin
        } else {
            InputScheme::Quanpin
        },
        touch_keyboard_layout: TouchKeyboardLayout::FourteenKey,
        touch_shuangpin_layout: if eighteen {
            TouchShuangpinLayout::EighteenKey
        } else {
            TouchShuangpinLayout::FourteenKey
        },
        ..chinese_preferences()
    };
    let options = json!({ "api_version": 1, "resources": resources, "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries, "preferences": preferences, "phrase_preedit": true }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    read(msime_client_focus(handle, true));
    if shuangpin {
        assert_eq!(
            read(msime_client_set_key_grid(
                handle,
                if eighteen { 5 } else { 2 }
            ))["ok"],
            true
        );
    }

    // 全拼「你好」是 BN UI GH AS OP，小鹤双拼是 BN UI GH CV。
    let codes = if exact {
        "NiHC"
    } else if eighteen {
        "bihx"
    } else if shuangpin {
        "bugc"
    } else {
        "bugao"
    };
    let tail = if exact {
        "HC"
    } else if eighteen {
        "hx"
    } else if shuangpin {
        "gc"
    } else {
        "gao"
    };
    let mut view = Value::Null;
    for letter in codes.bytes() {
        let typed = read(msime_client_grid_key(handle, letter));
        assert_eq!(typed["value"]["handled"], true, "{typed}");
        view = typed["value"]["view"].clone();
    }
    assert_eq!(view["editing_text"], codes);
    let ni = view["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .position(|candidate| candidate["text"] == "你")
        .unwrap_or_else(|| panic!("{view}"));
    let generation = view["generation"].as_u64().unwrap();
    let held = read(msime_client_select(handle, generation, ni));
    assert_eq!(held["value"]["view"]["phrase_prefix"], "你", "{held}");
    assert_eq!(held["value"]["view"]["editing_text"], tail, "{held}");
    for _ in 1..tail.len() {
        read(msime_client_command(handle, 0));
    }
    assert_eq!(
        read(msime_client_view(handle))["value"]["editing_text"],
        if exact {
            "H"
        } else if eighteen {
            "h"
        } else {
            "g"
        }
    );

    let restored = read(msime_client_command(handle, 0));
    let view = &restored["value"]["view"];
    assert_eq!(
        view["key_grid"],
        if eighteen {
            "eighteen_key"
        } else {
            "fourteen_key"
        },
        "{restored}"
    );
    assert_eq!(
        view["editing_text"],
        if exact {
            "Ni"
        } else if eighteen {
            "bi"
        } else {
            "bu"
        },
        "{restored}"
    );
    assert!(
        view["phrase_prefix"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "{restored}"
    );
    assert!(
        !view["nine_key_spellings"].as_array().unwrap().is_empty(),
        "{restored}"
    );
    if exact {
        assert!(
            !view["nine_key_spellings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == "bi"),
            "{restored}"
        );
    }
    let next = read(msime_client_grid_key(
        handle,
        if exact {
            b'H'
        } else if eighteen {
            b'h'
        } else {
            b'g'
        },
    ));
    assert_eq!(next["value"]["handled"], true, "{next}");
    assert_eq!(
        next["value"]["view"]["editing_text"],
        if exact {
            "NiH"
        } else if eighteen {
            "bih"
        } else {
            "bug"
        },
        "{next}"
    );
    read(msime_client_destroy(handle));
}

#[test]
fn eighteen_key_grid_abi_numbers_keep_scheme_and_composition() {
    for (scheme, accepted, rejected) in
        [(InputScheme::Quanpin, 4, 5), (InputScheme::Shuangpin, 5, 4)]
    {
        let directory = tempfile::tempdir().unwrap();
        let handle = test_host_with_pinyin_fixture(
            directory.path(),
            Preferences {
                scheme,
                ..chinese_preferences()
            },
        );
        read(msime_client_focus(handle, true));
        let initial = read(msime_client_view(handle));
        assert_eq!(
            read(msime_client_set_key_grid(handle, rejected))["ok"],
            false
        );
        assert_eq!(read(msime_client_view(handle)), initial);
        let enabled = read(msime_client_set_key_grid(handle, accepted));
        assert_eq!(enabled["ok"], true, "{scheme:?}: {enabled}");
        assert_eq!(enabled["value"]["key_grid"], "eighteen_key");
        assert_eq!(
            read(msime_client_grid_key(handle, b'b'))["value"]["handled"],
            true
        );
        let composing = read(msime_client_view(handle));
        assert_eq!(
            read(msime_client_set_key_grid(handle, rejected))["ok"],
            false
        );
        assert_eq!(read(msime_client_view(handle)), composing);
        assert_eq!(read(msime_client_set_key_grid(handle, 0))["ok"], false);
        assert_eq!(read(msime_client_view(handle)), composing);
        read(msime_client_destroy(handle));
    }
}

#[test]
fn eighteen_key_shuangpin_preference_is_applied_by_android_and_ios() {
    let mut document = serde_json::to_value(chinese_preferences()).unwrap();
    document["scheme"] = "shuangpin".into();
    document["touch_shuangpin_layout"] = "eighteen_key".into();
    let preferences: Preferences =
        serde_json::from_value(document).expect("共享偏好必须接受 18 键双拼布局");
    for platform in [
        HostPlatform::Android,
        HostPlatform::Ios,
        HostPlatform::Harmony,
        HostPlatform::Macos,
        HostPlatform::Windows,
        HostPlatform::Linux,
    ] {
        let grid = layout_key_grid_for(platform, Some(SchemeType::Shuangpin), &preferences);
        assert_eq!(
            grid.is_some(),
            matches!(platform, HostPlatform::Android | HostPlatform::Ios),
            "{platform:?}"
        );
        if let Some(grid) = grid {
            assert_eq!(grid.code_of(b'e'), Some(b'w'));
            assert_eq!(grid.code_of(b't'), Some(b'r'));
            assert_eq!(grid.code_of(b'n'), Some(b'b'));
            assert_eq!(grid.code_of(b';'), Some(b'l'));
        }
    }
}

#[test]
fn eighteen_key_shuangpin_grid_commits_without_replacing_hardware_letters() {
    let directory = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Shuangpin,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(directory.path(), preferences);
    read(msime_client_focus(handle, true));
    let enabled = read(msime_client_set_key_grid(handle, 5));
    assert_eq!(enabled["ok"], true, "18 键网格必须可以显式开启：{enabled}");
    assert_eq!(enabled["value"]["key_grid"], "eighteen_key");
    assert_eq!(enabled["value"]["nine_key"], false);
    let hardware = type_letters(handle, b"nihc");
    assert_eq!(hardware["editing_text"], "nihc");
    assert_eq!(
        read(msime_client_grid_key(handle, b'b'))["value"]["handled"],
        false
    );
    read(msime_client_command(handle, 3));
    let mut touched = Value::Null;
    for code in b"bihx" {
        touched = read(msime_client_grid_key(handle, *code));
        assert_eq!(touched["value"]["handled"], true, "{touched}");
    }
    let view = &touched["value"]["view"];
    assert_eq!(view["key_grid"], "eighteen_key");
    assert_eq!(view["editing_text"], "bihx");
    let index = view["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .position(|candidate| candidate["text"] == "本地")
        .expect("18 键组码必须找到合成词库中的 ni'hao 词条");
    let selected = read(msime_client_select(
        handle,
        view["generation"].as_u64().unwrap(),
        index,
    ));
    assert_eq!(selected["value"]["commit"], "本地", "{selected}");
    read(msime_client_destroy(handle));
}

#[test]
fn fourteen_key_shuangpin_preference_is_applied_by_touch_hosts() {
    let preferences = Preferences {
        scheme: InputScheme::Shuangpin,
        touch_shuangpin_layout: TouchShuangpinLayout::FourteenKey,
        ..chinese_preferences()
    };
    for platform in [
        HostPlatform::Android,
        HostPlatform::Ios,
        HostPlatform::Harmony,
        HostPlatform::Macos,
        HostPlatform::Windows,
        HostPlatform::Linux,
    ] {
        let grid = layout_key_grid_for(platform, Some(SchemeType::Shuangpin), &preferences);
        assert_eq!(
            grid.is_some(),
            matches!(platform, HostPlatform::Android | HostPlatform::Ios),
            "{platform:?}"
        );
    }
}

#[test]
fn fourteen_key_wubi_preference_is_applied_by_touch_hosts() {
    let preferences = Preferences {
        scheme: InputScheme::Wubi,
        touch_wubi_layout: TouchWubiLayout::FourteenKey,
        ..chinese_preferences()
    };
    for platform in [
        HostPlatform::Android,
        HostPlatform::Ios,
        HostPlatform::Harmony,
        HostPlatform::Macos,
        HostPlatform::Windows,
        HostPlatform::Linux,
    ] {
        let grid = layout_key_grid_for(platform, Some(SchemeType::Wubi), &preferences);
        assert_eq!(
            grid,
            matches!(platform, HostPlatform::Android | HostPlatform::Ios)
                .then_some(KeyGrid::FourteenKey),
            "{platform:?}"
        );
        // 五笔 14 键只看自己的布局：26 键时不开网格，全拼的 14 键也不带进五笔。
        let twenty_six = Preferences {
            touch_wubi_layout: TouchWubiLayout::TwentySixKey,
            touch_keyboard_layout: TouchKeyboardLayout::FourteenKey,
            ..preferences.clone()
        };
        assert_eq!(
            layout_key_grid_for(platform, Some(SchemeType::Wubi), &twenty_six),
            None
        );
        // 手写布局不画字母键。
        let handwriting = Preferences {
            touch_keyboard_layout: TouchKeyboardLayout::Handwriting,
            ..preferences.clone()
        };
        assert_eq!(
            layout_key_grid_for(platform, Some(SchemeType::Wubi), &handwriting),
            None
        );
        // 五笔的布局不影响全拼。
        assert_eq!(
            layout_key_grid_for(platform, Some(SchemeType::Quanpin), &preferences),
            None
        );
    }
}

/// 鸿蒙经网格值 2 自己打开五笔 14 键；组码按组展开码表，硬件字母与 14 键组码不混在一次组字里。
#[test]
fn fourteen_key_wubi_grid_decodes_through_the_c_abi() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let resources = path("resources");
    let dictionaries = path("dictionaries");
    let fixture = "CREATE TABLE tbl_1_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_n VALUES('ni','n','你',3000000);
                   CREATE TABLE tbl_1_d(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_1_d VALUES('duo','d','多',1500000);
                   CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);
                   INSERT INTO wubi86 VALUES('qq','多',20),('wq','你',10),('w','人',20);
                   CREATE TABLE quick_parases(key TEXT,value TEXT,weight INTEGER);";
    for directory in [&resources, &dictionaries] {
        rusqlite::Connection::open(directory.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(fixture)
            .unwrap();
    }
    bundle_wubi_table(&resources);
    let preferences = Preferences {
        scheme: InputScheme::Wubi,
        ..chinese_preferences()
    };
    let options = json!({ "api_version": 1, "resources": resources, "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries, "preferences": preferences }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    read(msime_client_focus(handle, true));
    assert_eq!(read(msime_client_view(handle))["value"]["key_grid"], "none");
    // 没有网格时 14 键的键不处理。
    assert_eq!(
        read(msime_client_grid_key(handle, b'q'))["value"]["handled"],
        false
    );
    let opened = read(msime_client_set_key_grid(handle, 2));
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(opened["value"]["key_grid"], "fourteen_key");
    let mut view = Value::Null;
    for letter in b"qw" {
        let result = read(msime_client_grid_key(handle, *letter));
        assert_eq!(result["value"]["handled"], true, "{result}");
        view = result["value"]["view"].clone();
    }
    assert_eq!(view["candidates"][0]["text"], "你");
    assert_eq!(view["candidates"][1]["text"], "多");
    assert_eq!(view["nine_key_reading"], "wq");
    assert_eq!(view["editing_text"], "qq");
    let hardware = read(msime_client_character(handle, b'w', false));
    assert_eq!(hardware["value"]["handled"], false, "{hardware}");
    read(msime_client_command(handle, 3));
    // 组字结束后硬件字母照常打 26 键的码。
    assert_eq!(type_letters(handle, b"w")["candidates"][0]["text"], "人");
    assert_eq!(
        read(msime_client_grid_key(handle, b'q'))["value"]["handled"],
        false
    );
    // 别的网格不给五笔。
    read(msime_client_command(handle, 3));
    assert_eq!(read(msime_client_set_key_grid(handle, 4))["ok"], false);
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

/// 双拼布局独立持久化，创建、延迟应用、实体字母和非法自定义表回退使用同一方案。
#[test]
fn fourteen_key_shuangpin_layout_tracks_profile_changes_and_preserves_hardware_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Shuangpin,
        touch_shuangpin_layout: TouchShuangpinLayout::FourteenKey,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    let automatic_grid = if matches!(
        ffi::host::compiled_platform(),
        HostPlatform::Android | HostPlatform::Ios
    ) {
        "fourteen_key"
    } else {
        "none"
    };
    assert_eq!(
        read(msime_client_view(handle))["value"]["key_grid"],
        automatic_grid
    );
    assert_eq!(read(msime_client_set_key_grid(handle, 2))["ok"], true);
    let hardware = type_letters(handle, b"nihc");
    assert_eq!(hardware["editing_text"], "nihc");
    assert_eq!(
        read(msime_client_grid_key(handle, b'b'))["value"]["handled"],
        false
    );
    read(msime_client_command(handle, 3));
    let mut touched = Value::Null;
    for code in b"bugc" {
        touched = read(msime_client_grid_key(handle, *code));
        assert_eq!(touched["value"]["handled"], true, "{touched}");
    }
    assert!(
        touched["value"]["view"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["text"] == "本地"),
        "{touched}"
    );
    preferences.shuangpin_profile = ShuangpinProfile::Microsoft;
    let deferred = update(handle, 1, &preferences);
    assert_eq!(deferred["value"]["deferred"], true, "{deferred}");
    read(msime_client_command(handle, 3));
    assert_eq!(
        read(msime_client_view(handle))["value"]["shuangpin_profile"],
        "microsoft"
    );
    // 微软的 q;（qing）映射为 QW L；这里只核对候选读音，不依赖产品词库。
    assert_eq!(read(msime_client_set_key_grid(handle, 2))["ok"], true);
    read(msime_client_grid_key(handle, b'q'));
    let qing = read(msime_client_grid_key(handle, b'l'));
    assert!(
        qing["value"]["view"]["nine_key_spellings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row == "qing"),
        "{qing}"
    );
    read(msime_client_command(handle, 3));
    preferences.shuangpin_profile = ShuangpinProfile::Custom;
    let fallback = update(handle, 2, &preferences);
    assert_eq!(
        fallback["value"]["view"]["shuangpin_profile"], "xiaohe",
        "{fallback}"
    );
    assert_eq!(
        fallback["value"]["view"]["key_grid"], automatic_grid,
        "{fallback}"
    );
    preferences.shuangpin_custom_profile =
        builtin_shuangpin_custom_profile(ShuangpinProfile::Microsoft).unwrap();
    let custom = update(handle, 3, &preferences);
    assert_eq!(
        custom["value"]["view"]["shuangpin_profile"], "custom",
        "{custom}"
    );
    assert_eq!(read(msime_client_set_key_grid(handle, 2))["ok"], true);
    read(msime_client_grid_key(handle, b'q'));
    let qing = read(msime_client_grid_key(handle, b'l'));
    assert!(
        qing["value"]["view"]["nine_key_spellings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row == "qing"),
        "{qing}"
    );
    read(msime_client_command(handle, 3));
    preferences.touch_shuangpin_layout = TouchShuangpinLayout::TwentySixKey;
    assert_eq!(
        update(handle, 4, &preferences)["value"]["view"]["key_grid"],
        "none"
    );
    read(msime_client_destroy(handle));
}

/// 全拼布局不自动打开双拼 14 键；宿主显式开 14 键时根据当前方案选择解码表。
#[test]
fn a_fourteen_key_layout_starts_and_keeps_the_fourteen_key_grid() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        touch_keyboard_layout: TouchKeyboardLayout::FourteenKey,
        ..chinese_preferences()
    };
    let handle = test_host_preferences(dir.path(), preferences.clone());
    let view = read(msime_client_view(handle));
    assert_eq!(view["value"]["key_grid"], "fourteen_key");
    assert_eq!(view["value"]["nine_key"], false);
    assert_eq!(view["value"]["touch_keyboard_layout"], "fourteen_key");
    preferences.learning = !preferences.learning;
    assert_eq!(
        update(handle, 1, &preferences)["value"]["view"]["key_grid"],
        "fourteen_key"
    );
    // 宿主关掉网格（比如密码框画 26 键），同一布局下重建也不会再打开。
    read(msime_client_set_key_grid(handle, 0));
    preferences.learning = !preferences.learning;
    assert_eq!(
        update(handle, 2, &preferences)["value"]["view"]["key_grid"],
        "none"
    );
    preferences.touch_keyboard_layout = TouchKeyboardLayout::NineKey;
    assert_eq!(
        update(handle, 3, &preferences)["value"]["view"]["key_grid"],
        "nine_key"
    );
    preferences.touch_keyboard_layout = TouchKeyboardLayout::FourteenKey;
    assert_eq!(
        update(handle, 4, &preferences)["value"]["view"]["key_grid"],
        "fourteen_key"
    );
    preferences.scheme = InputScheme::Shuangpin;
    let shuangpin = update(handle, 5, &preferences);
    assert_eq!(
        shuangpin["value"]["view"]["key_grid"], "none",
        "{shuangpin}"
    );
    assert_eq!(read(msime_client_set_key_grid(handle, 2))["ok"], true);
    read(msime_client_destroy(handle));
}

#[test]
fn a_fallen_back_scheme_starts_in_the_nine_key_mode_a_rebuild_gives_it() {
    // Cantonese without its dictionary runs as Quanpin, so a nine-key layout starts nine-key at creation exactly as the next preferences rebuild would leave it.
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Cantonese,
        last_chinese_scheme: None,
        touch_keyboard_layout: TouchKeyboardLayout::NineKey,
        ..chinese_preferences()
    };
    let handle = test_host_preferences(dir.path(), preferences.clone());
    let view = read(msime_client_view(handle));
    assert_eq!(view["value"]["scheme"], 0);
    assert_eq!(view["value"]["nine_key"], true);
    let rebuilt = update(handle, 1, &preferences);
    assert_eq!(rebuilt["value"]["view"]["scheme"], 0);
    assert_eq!(rebuilt["value"]["view"]["nine_key"], true);
    read(msime_client_destroy(handle));
}

#[test]
fn nine_key_mode_follows_only_the_scheme_the_grid_spells() {
    // 九键能拼的方案是全拼和注音（`SchemeType::nine_key`）：九键布局在全拼里开启九键模式，在下面这些方案里都保持关闭，创建时和重建后都一样。注音不在这里测，因为这个宿主没有安装注音词库，运行时不会跑它。
    for scheme in [
        InputScheme::Quanpin,
        InputScheme::Shuangpin,
        InputScheme::Wubi,
        InputScheme::Japanese,
        InputScheme::Korean,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let preferences = Preferences {
            scheme,
            touch_keyboard_layout: TouchKeyboardLayout::NineKey,
            ..chinese_preferences()
        };
        let handle = test_host_preferences(dir.path(), preferences.clone());
        let expected = scheme == InputScheme::Quanpin;
        assert_eq!(
            read(msime_client_view(handle))["value"]["nine_key"],
            expected,
            "{scheme:?} at creation"
        );
        let rebuilt = update(
            handle,
            1,
            &Preferences {
                candidate_page_size: 4,
                ..preferences
            },
        );
        assert_eq!(
            rebuilt["value"]["view"]["nine_key"], expected,
            "{scheme:?} after a rebuild"
        );
        read(msime_client_destroy(handle));
    }
}

// 注音配九键布局：宿主接口按布局自动开启九键模式，数字和声调字母拼音节，候选读音和全拼九键一样经 `nine_key_spellings` 和 `msime_client_choose_nine_key_spelling` 往返。
#[test]
fn zhuyin_with_the_nine_key_layout_starts_in_nine_key_mode() {
    use msime_engine::language_dictionary::{FORMAT_VERSION, METADATA_FORMAT_VERSION, SCHEMA};
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let language_dictionaries = path("language-dictionaries");
    let connection =
        rusqlite::Connection::open(language_dictionaries.join("msime-zhuyin.db")).unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    connection
        .execute(
            "INSERT INTO metadata VALUES (?1, ?2)",
            (METADATA_FORMAT_VERSION, FORMAT_VERSION.to_string()),
        )
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO syllables VALUES ('ㄋㄧˇ'),('ㄌㄧˇ');\
             INSERT INTO entries VALUES ('ㄋㄧˇ','你',1000),('ㄌㄧˇ','李',1200);",
        )
        .unwrap();
    drop(connection);
    // The layout field alone is what a desktop host's Quanpin 九键 toggle leaves behind; Zhuyin there keeps the Dachen keys.
    let layout_only = Preferences {
        scheme: InputScheme::Zhuyin,
        touch_keyboard_layout: TouchKeyboardLayout::NineKey,
        ..chinese_preferences()
    };
    let options = json!({ "api_version": 1, "resources": path("resources"), "user_data": path("user"), "cache": path("cache"), "dictionaries": path("dictionaries"), "preferences": layout_only, "language_dictionaries": language_dictionaries }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let dachen = created["value"]["session"].as_u64().unwrap();
    read(msime_client_focus(dachen, true));
    assert_eq!(read(msime_client_view(dachen))["value"]["nine_key"], false);
    read(msime_client_destroy(dachen));

    let mut preferences = Preferences {
        scheme: InputScheme::Zhuyin,
        touch_keyboard_layout: TouchKeyboardLayout::NineKey,
        ..chinese_preferences()
    };
    preferences
        .touch_keyboard_schemes
        .enabled
        .insert(TouchKeyboardScheme::ZhuyinNineKey);
    preferences.touch_keyboard_schemes.selected = Some(TouchKeyboardScheme::ZhuyinNineKey);
    let options = json!({ "api_version": 1, "resources": path("resources"), "user_data": path("user"), "cache": path("cache"), "dictionaries": path("dictionaries"), "preferences": preferences, "language_dictionaries": language_dictionaries }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    read(msime_client_focus(handle, true));
    let view = read(msime_client_view(handle));
    assert_eq!(view["value"]["scheme"], 6);
    assert_eq!(view["value"]["nine_key"], true);
    assert_eq!(view["value"]["spelling_symbols"], "1234567890");

    let mut typed = Value::Null;
    for key in *b"28c" {
        typed = read(msime_client_character(handle, key, false));
        assert_eq!(typed["value"]["handled"], true, "{}", char::from(key));
    }
    let view = &typed["value"]["view"];
    assert_eq!(view["editing_text"], "28c");
    assert_eq!(view["nine_key_spellings"], json!(["ㄌㄧˇ", "ㄋㄧˇ"]));
    let generation = view["generation"].as_u64().unwrap();
    let chosen = read(msime_client_choose_nine_key_spelling(handle, generation, 1));
    assert_eq!(chosen["value"]["handled"], true);
    assert_eq!(chosen["value"]["view"]["nine_key_spellings"], json!([]));
    let committed = read(msime_client_command(handle, 9));
    assert_eq!(committed["value"]["commit"], "你");
    read(msime_client_destroy(handle));
}

// 宿主为全拼九键打开的覆盖不能带进注音：从全拼切到注音时按注音自己的布局判断，没选「注音 9 键」就是大千键位。
#[test]
fn a_quanpin_nine_key_override_does_not_carry_into_zhuyin() {
    use msime_engine::language_dictionary::{FORMAT_VERSION, METADATA_FORMAT_VERSION, SCHEMA};
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let language_dictionaries = path("language-dictionaries");
    let connection =
        rusqlite::Connection::open(language_dictionaries.join("msime-zhuyin.db")).unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    connection
        .execute(
            "INSERT INTO metadata VALUES (?1, ?2)",
            (METADATA_FORMAT_VERSION, FORMAT_VERSION.to_string()),
        )
        .unwrap();
    connection
        .execute_batch("INSERT INTO syllables VALUES ('ㄋㄧˇ'); INSERT INTO entries VALUES ('ㄋㄧˇ','你',1000);")
        .unwrap();
    drop(connection);
    let quanpin = Preferences {
        scheme: InputScheme::Quanpin,
        touch_keyboard_layout: TouchKeyboardLayout::NineKey,
        ..chinese_preferences()
    };
    let options = json!({ "api_version": 1, "resources": path("resources"), "user_data": path("user"), "cache": path("cache"), "dictionaries": path("dictionaries"), "preferences": quanpin, "language_dictionaries": language_dictionaries }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    read(msime_client_focus(handle, true));
    assert_eq!(
        read(msime_client_set_nine_key_mode(handle, true))["value"]["nine_key"],
        true
    );
    let zhuyin = update(
        handle,
        1,
        &Preferences {
            scheme: InputScheme::Zhuyin,
            ..quanpin
        },
    );
    assert_eq!(zhuyin["value"]["view"]["scheme"], 6, "{zhuyin}");
    assert_eq!(zhuyin["value"]["view"]["nine_key"], false, "{zhuyin}");
    read(msime_client_destroy(handle));
}

#[test]
fn the_repeat_gesture_arms_except_in_schemes_that_write_no_chinese_marks() {
    // The repeat gesture turns an ASCII mark into a Chinese one. It never arms in Korean or Vietnamese, which write only ASCII marks, or in Zhuyin, whose punctuation keys spell bopomofo; Japanese arms as it did before the new schemes. Cantonese, Zhuyin and Stroke are left out because this host installs no language dictionaries, so the runtime would not run them; Stroke writes Chinese marks like Cantonese and is not on the engine's disarm list.
    for (scheme, arms) in [
        (InputScheme::Quanpin, true),
        (InputScheme::Shuangpin, true),
        (InputScheme::Wubi, true),
        (InputScheme::Japanese, true),
        (InputScheme::Korean, false),
        (InputScheme::Vietnamese, false),
        (InputScheme::Tibetan, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let handle = test_host_preferences(
            dir.path(),
            Preferences {
                scheme,
                smart_punctuation: true,
                smart_punctuation_repeat: true,
                ..chinese_preferences()
            },
        );
        let arm = json!({"ascii": b'.', "commit": ".", "timestamp_ms": 1, "editor_generation": 1, "auto_closed_pair": false}).to_string();
        // SAFETY: the buffer outlives the call.
        let armed =
            read(unsafe { msime_client_smart_punctuation_arm(handle, arm.as_ptr(), arm.len()) });
        assert_eq!(armed["ok"], true);
        assert_eq!(!armed["value"]["repeat"].is_null(), arms, "{scheme:?}");
        read(msime_client_destroy(handle));
    }
}

#[test]
fn japanese_candidates_ask_for_no_translation() {
    // Only the schemes that show glosses (`SchemeType::shows_glosses`) plan a translation query; Japanese candidates never carry one. Quanpin under the same preferences is the positive control, so the gate tested here is the scheme predicate and nothing earlier. Quanpin types a Unicode code point because the test host installs no dictionary.
    let cases: [(InputScheme, &[u8], bool); 2] = [
        (InputScheme::Quanpin, b"U4e2d", true),
        (InputScheme::Japanese, b"a", false),
    ];
    for (scheme, keys, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        let handle = test_host_preferences(
            dir.path(),
            Preferences {
                scheme,
                candidate_translations: true,
                ..chinese_preferences()
            },
        );
        read(msime_client_focus(handle, true));
        let mut view = Value::Null;
        for byte in keys {
            view = read(msime_client_character(
                handle,
                *byte,
                byte.is_ascii_uppercase(),
            ))["value"]["view"]
                .clone();
        }
        assert!(
            !view["candidates"].as_array().unwrap().is_empty(),
            "{scheme:?} offered no candidates"
        );
        assert_eq!(
            !read(msime_client_translation_query(handle))["value"].is_null(),
            expected,
            "{scheme:?}"
        );
        read(msime_client_destroy(handle));
    }
}

// 网址可能带着私密路径和参数，网址模式不向翻译服务要候选翻译。对照组是同一偏好下的 Unicode 候选，确认拦下它的是网址模式本身。
#[test]
fn url_candidates_ask_for_no_translation() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            candidate_translations: true,
            ..chinese_preferences()
        },
    );
    read(msime_client_focus(handle, true));
    for byte in b"U4e2d" {
        read(msime_client_character(
            handle,
            *byte,
            byte.is_ascii_uppercase(),
        ));
    }
    assert!(!read(msime_client_translation_query(handle))["value"].is_null());
    read(msime_client_command(handle, 3));

    for byte in b"www" {
        read(msime_client_character(handle, *byte, false));
    }
    read(msime_client_punctuation(handle, b'.'));
    let view = read(msime_client_character(handle, b'a', false))["value"]["view"].clone();
    assert_eq!(view["local_mode"], "url");
    assert!(!view["candidates"].as_array().unwrap().is_empty());
    assert!(read(msime_client_translation_query(handle))["value"].is_null());
    read(msime_client_destroy(handle));
}

#[test]
fn the_c_header_aliases_the_candidate_list_command_and_extends_the_scheme_legend() {
    const HEADER: &str = include_str!("../include/msime_client.h");
    // One command number under two names: hosts written for Korean keep MSIME_CONVERT_HANJA, and the dispatch reads 16 either way.
    assert!(HEADER.contains("MSIME_CONVERT_HANJA = 16,"));
    assert!(HEADER.contains("MSIME_OPEN_CANDIDATE_LIST = 16,"));
    // platforms/android/check-host.sh greps the Korean legend line, so it stays as it was.
    assert!(HEADER.contains("4 korean (preferences scheme \"korean\")"));
    for legend in [
        "5 cantonese",
        "6 zhuyin",
        "7 vietnamese",
        "8 tibetan",
        "9 stroke",
    ] {
        assert!(HEADER.contains(legend), "{legend} missing from the legend");
    }
}

#[test]
fn the_c_header_declares_the_session_store_as_rust_reads_it() {
    const HEADER: &str = include_str!("../include/msime_client.h");
    assert!(HEADER.contains("#define MSIME_CLIENT_SESSION_STORE_VERSION 1\n"));
    assert!(HEADER.contains("#define MSIME_CLIENT_SESSION_STORE_SHARED 1u\n"));
    for item in [
        "MSIME_CLIENT_SESSION_ITEM_ACCOUNT = 1,",
        "MSIME_CLIENT_SESSION_ITEM_CLIPBOARD_KEYSTORE = 2,",
        "MSIME_CLIENT_SESSION_ITEM_ANONYMOUS = 3,",
        "MSIME_CLIENT_SESSION_ITEM_ANONYMOUS_IDENTITY = 4,",
    ] {
        assert!(HEADER.contains(item), "{item}");
    }
    // 字段顺序与 `ffi::session_store::SessionStoreV1` 相同；布局本身由那边的 size_of/offset_of 测试锁住。
    let start = HEADER
        .find("typedef struct msime_client_session_store {")
        .unwrap();
    let end = start
        + HEADER[start..]
            .find("} msime_client_session_store;")
            .unwrap();
    let mut position = start;
    for field in [
        "uint32_t version;",
        "uint32_t layout;",
        "uint32_t flags;",
        "const char *client_agent;",
        "void *context;",
        "intptr_t (*load)(void *context, uint32_t item, uint8_t *buffer, size_t capacity);",
        "int32_t (*save)(void *context, uint32_t item, const uint8_t *bytes, size_t length);",
        "int32_t (*remove)(void *context, uint32_t item);",
        "int32_t (*lock)(void *context, uint32_t item, uint32_t timeout_ms, uint64_t *token);",
        "void (*unlock)(void *context, uint32_t item, uint64_t token);",
        "void (*release)(void *context);",
    ] {
        let found = HEADER[position..end]
            .find(field)
            .unwrap_or_else(|| panic!("{field} missing or out of order"));
        position += found + field.len();
    }
}

#[test]
fn nine_key_digits_offer_ranked_english_across_the_host_boundary() {
    use msime_client_core::preferences::MixedInputPreferences;

    let dir = tempfile::tempdir().unwrap();
    let dictionaries = dir.path().join("dictionaries");
    std::fs::create_dir_all(&dictionaries).unwrap();
    let db = rusqlite::Connection::open(dictionaries.join("msime-english.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE english_words(word TEXT,display TEXT,weight INTEGER);
         INSERT INTO english_words VALUES('ok','ok',900);
         INSERT INTO english_words VALUES('old','old',1000);
         INSERT INTO english_words VALUES('older','older',800);",
    )
    .unwrap();
    drop(db);

    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            candidate_page_size: 2,
            learning: false,
            touch_keyboard_layout: TouchKeyboardLayout::NineKey,
            mixed_input: MixedInputPreferences {
                enabled: true,
                first_page_count: 1,
                english: true,
                minimum_prefix: 2,
                emoji: false,
                kaomoji: false,
                japanese: false,
                symbols: false,
                quick_phrase: false,
                quick_phrase_frequency: true,
            },
            ..chinese_preferences()
        },
    );
    read(msime_client_focus(handle, true));
    assert_eq!(
        read(msime_client_character(handle, b'6', false))["value"]["handled"],
        true
    );
    let typed = read(msime_client_character(handle, b'5', false));
    assert_eq!(typed["value"]["view"]["nine_key"], true);

    let complete = read(msime_client_all_candidates(handle));
    let candidates = complete["value"]["candidates"].as_array().unwrap();
    let position = |text: &str| {
        candidates
            .iter()
            .position(|candidate| candidate["text"] == text)
            .unwrap_or_else(|| panic!("missing synthetic candidate {text}"))
    };
    let ok = position("ok");
    assert!(ok < position("old"));
    assert!(ok < position("older"));

    let generation = complete["value"]["generation"].as_u64().unwrap();
    let index = candidates[ok]["id"]["index"].as_u64().unwrap() as usize;
    let selected = read(msime_client_select_any_candidate(handle, generation, index));
    assert_eq!(selected["value"]["handled"], true);
    assert_eq!(selected["value"]["commit"], "ok");
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

/// 横向候选条每键只取一批（`msime_client_candidate_range`）：首字母打下去时不把整张声母表放出来，身份与整代复制里同一座位的那一行相同；把放出来的取完再要，才放出暂扣的尾部并换一代。
#[test]
fn candidate_range_takes_one_batch_and_releases_the_initial_tail_only_past_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let resources = path("resources");
    let dictionaries = path("dictionaries");
    // 声母 c 有 30 个字，比首字母先给的 24 行多。
    let mut fixture =
        String::from("CREATE TABLE tbl_1_c(key TEXT,jp TEXT,value TEXT,weight INTEGER);");
    let words: Vec<char> = "才菜参残草测层茶查差柴产长常场唱超朝车成城程吃池出初除处川穿"
        .chars()
        .collect();
    assert_eq!(words.len(), 30);
    for (index, word) in words.iter().enumerate() {
        fixture.push_str(&format!(
            "INSERT INTO tbl_1_c VALUES('c{index:02}','c','{word}',{});",
            1000 - index * 10
        ));
    }
    fixture.push_str("CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);");
    for directory in [&resources, &dictionaries] {
        rusqlite::Connection::open(directory.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(&fixture)
            .unwrap();
    }
    bundle_wubi_table(&resources);
    let preferences = Preferences {
        learning: false,
        ..chinese_preferences()
    };
    let options = json!({ "api_version": 1, "resources": resources, "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries, "preferences": preferences }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    let handle = created["value"]["session"].as_u64().unwrap();
    read(msime_client_focus(handle, true));
    let typed = read(msime_client_character(handle, b'c', false));
    let generation = typed["value"]["view"]["generation"].as_u64().unwrap();

    let first = read(msime_client_candidate_range(handle, 0, 48))["value"].clone();
    assert_eq!(first["generation"], generation, "{first}");
    let total = first["total"].as_u64().unwrap() as usize;
    assert!(total < words.len(), "the initial tail stays held: {first}");
    assert_eq!(first["candidates"].as_array().unwrap().len(), total);
    assert_eq!(
        read(msime_client_view(handle))["value"]["generation"],
        generation
    );
    let window = read(msime_client_candidate_range(handle, 2, 3))["value"].clone();
    let window = window["candidates"].as_array().unwrap();
    assert_eq!(window.len(), 3);
    assert_eq!(window[0]["id"]["index"], 2);
    assert_eq!(window[0]["id"]["generation"], generation);
    assert_eq!(window[0]["text"], first["candidates"][2]["text"]);

    // 取到放出来的末尾之后再要：放出尾部、换一代，视图跟着换。
    let more = read(msime_client_candidate_range(handle, total, 48))["value"].clone();
    let released = more["generation"].as_u64().unwrap();
    assert!(released > generation, "{more}");
    let released_total = more["total"].as_u64().unwrap() as usize;
    assert!(released_total >= words.len(), "{more}");
    assert_eq!(
        read(msime_client_view(handle))["value"]["generation"],
        released
    );
    // 新的一代里每个座位都和整代复制同一身份、同一候选；取到末尾以后同一代不再给。
    let whole = read(msime_client_candidate_range(handle, 0, released_total))["value"].clone();
    let complete = read(msime_client_all_candidates(handle))["value"].clone();
    assert_eq!(complete["generation"], released);
    assert_eq!(whole["candidates"], complete["candidates"]);
    let exhausted = read(msime_client_candidate_range(handle, released_total, 48))["value"].clone();
    assert_eq!(exhausted["generation"], released);
    assert_eq!(exhausted["candidates"].as_array().unwrap().len(), 0);

    // 选中范围里取来的候选和整代复制里的一样可用。
    let index = whole["candidates"][released_total - 1]["id"]["index"]
        .as_u64()
        .unwrap() as usize;
    let selected = read(msime_client_select_any_candidate(handle, released, index));
    assert_eq!(selected["value"]["handled"], true, "{selected}");
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

/// 宿主按宽度排出的一行跨过固定页：报出显示页后，`;` 选的是这一行的第二个，翻译查询列的也是这一行。
#[test]
fn display_page_routes_second_third_shortcut_and_translation_query_through_the_abi() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let resources = path("resources");
    let dictionaries = path("dictionaries");
    let mut fixture =
        String::from("CREATE TABLE tbl_2_c(key TEXT,jp TEXT,value TEXT,weight INTEGER);");
    for (index, word) in [
        "测甲", "测乙", "测丙", "测丁", "测戊", "测己", "测庚", "测辛", "测壬", "测癸",
    ]
    .iter()
    .enumerate()
    {
        fixture.push_str(&format!(
            "INSERT INTO tbl_2_c VALUES('ce''shi','cs','{word}',{});",
            1000 - index * 10
        ));
    }
    fixture.push_str(
        "CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);
         CREATE TABLE quick_parases(key TEXT,value TEXT,weight INTEGER);
         CREATE INDEX idx_quick_parases_key_weight ON quick_parases(key,weight DESC);",
    );
    for directory in [&resources, &dictionaries] {
        rusqlite::Connection::open(directory.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(&fixture)
            .unwrap();
    }
    bundle_wubi_table(&resources);
    let preferences = Preferences {
        candidate_page_size: 3,
        learning: false,
        // 翻译查询只在释义或翻译开着时列候选；离线英文释义不出本机。
        candidate_english_gloss: true,
        second_third_candidate: msime_client_core::preferences::SecondThirdCandidatePreferences {
            enabled: true,
            ..Default::default()
        },
        ..chinese_preferences()
    };
    let options = json!({ "api_version": 1, "resources": resources, "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries, "preferences": preferences }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    let handle = created["value"]["session"].as_u64().unwrap();
    read(msime_client_focus(handle, true));
    for byte in *b"ceshi" {
        read(msime_client_character(handle, byte, false));
    }
    let complete = read(msime_client_all_candidates(handle));
    let generation = complete["value"]["generation"].as_u64().unwrap();
    let candidates = complete["value"]["candidates"].as_array().unwrap();
    let index = |text: &str| {
        candidates
            .iter()
            .find(|candidate| candidate["text"] == text)
            .unwrap_or_else(|| panic!("missing synthetic candidate {text}"))["id"]["index"]
            .as_u64()
            .unwrap() as usize
    };
    // 固定页每页 3 个；显示的一行是第二页末尾到第三页开头的 测丁 测戊 测己 测庚。
    let shown = ["测丁", "测戊", "测己", "测庚"].map(index);
    let set = |generation: u64, seats: &[usize]| {
        read(unsafe {
            msime_client_set_display_page(handle, generation, seats.as_ptr(), seats.len())
        })
    };
    assert_eq!(set(generation + 1, &shown)["value"]["applied"], false);
    assert_eq!(
        set(generation, &[candidates.len()])["value"]["applied"],
        false
    );
    assert_eq!(set(generation, &shown)["value"]["applied"], true);
    let query = read(msime_client_translation_query(handle));
    let texts: Vec<_> = query["value"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| candidate["text"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(texts, ["测丁", "测戊", "测己", "测庚"]);
    let selected = read(msime_client_character(handle, b';', false));
    assert_eq!(selected["value"]["commit"], "测戊");
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

#[test]
fn helpcode_defaults_follow_preferences_for_each_pinyin_scheme() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        // 全拼默认开；方案名与注释开关各自保持出厂值。
        assert!(session.options.helpcode);
        assert_eq!(session.options.helpcode_schema, "ziranma");
        assert!(!session.options.show_helpcode);
    });

    let shuangpin = Preferences {
        scheme: InputScheme::Shuangpin,
        ..chinese_preferences()
    };
    assert_eq!(update(handle, 1, &shuangpin)["value"]["deferred"], false);
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert!(!session.options.helpcode);
        assert_eq!(session.options.helpcode_schema, "lantian");
        assert!(session.options.show_helpcode);
    });
    read(msime_client_destroy(handle));
}

#[test]
fn english_completion_boundary_is_read_only_and_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    let dictionaries = dir.path().join("dictionaries");
    std::fs::create_dir_all(&dictionaries).unwrap();
    let db = rusqlite::Connection::open(dictionaries.join("msime-english.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE english_words(word TEXT,display TEXT,weight INTEGER);
         INSERT INTO english_words VALUES('hello','hello',1000);
         INSERT INTO english_words VALUES('help','help',900);
         INSERT INTO english_words VALUES('helium','helium',800);",
    )
    .unwrap();
    drop(db);
    let handle = test_host(dir.path());
    let before = read(msime_client_view(handle))["value"].clone();
    let prefix = b"He";
    let result =
        read(unsafe { msime_client_english_completions(handle, prefix.as_ptr(), prefix.len(), 2) });
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"]["completions"], json!(["hello", "help"]));
    assert_eq!(read(msime_client_view(handle))["value"], before);
    let invalid = read(unsafe { msime_client_english_completions(handle, b"he!".as_ptr(), 3, 2) });
    assert_eq!(invalid["ok"], false);
    read(msime_client_destroy(handle));
}

#[test]
fn shuangpin_manual_tab_is_consumed_only_by_supported_hosts() {
    let modes = msime_client_core::preferences::ShuangpinHelpcodeModes {
        manual_tab: true,
        ..Default::default()
    };
    let effective = shuangpin_helpcode_modes(modes);
    assert_eq!(effective.manual_tab, cfg!(target_os = "macos"));
    assert!(effective.direct && effective.trigger_semicolon);
    assert!(modes.manual_tab);
}

/// 触屏键盘的符号面板和外接键盘把 `;` 当标点送来；它此刻是末尾间接触发键时要记成标记，而不是按首选上屏再打一个分号。
#[test]
fn shuangpin_end_trigger_sent_as_punctuation_marks_helpcode() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    preferences.scheme = InputScheme::Shuangpin;
    preferences.shuangpin_helpcode.enabled = true;
    preferences.shuangpin_helpcode.modes.direct = false;
    let handle = test_host_preferences(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    let view = type_letters(handle, b"nihc");
    assert!(
        view["spelling_symbols"].as_str().unwrap().contains(';'),
        "{view}"
    );
    let result = read(msime_client_punctuation_with_context(handle, b';', 0));
    assert_eq!(result["value"]["handled"], true, "{result}");
    assert_eq!(result["value"]["view"]["editing_text"], "nihc|");
    assert!(result["value"]["commit"].is_null(), "{result}");
    read(msime_client_destroy(handle));
}

#[test]
#[cfg(target_os = "macos")]
fn shuangpin_manual_tab_roundtrips_through_host_character() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    preferences.scheme = InputScheme::Shuangpin;
    preferences.shuangpin_helpcode.enabled = true;
    preferences.shuangpin_helpcode.modes.manual_tab = true;
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    let view = type_letters(handle, b"nihc");
    assert!(
        view["spelling_symbols"].as_str().unwrap().contains('\t'),
        "{view}"
    );
    let result = read(msime_client_character(handle, b'\t', false));
    assert_eq!(result["value"]["handled"], true, "{result}");
    assert_eq!(result["value"]["view"]["editing_text"], "nihc|");
    type_letters(handle, b"x");
    read(msime_client_command(handle, 3));
    preferences.shuangpin_helpcode.modes.manual_tab = false;
    assert_eq!(update(handle, 1, &preferences)["ok"], true);
    let view = type_letters(handle, b"nihc");
    assert!(!view["spelling_symbols"].as_str().unwrap().contains('\t'));
    read(msime_client_destroy(handle));
}

#[test]
#[cfg(target_os = "macos")]
fn shuangpin_manual_tab_survives_held_phrase_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let resources = path("resources");
    let dictionaries = path("dictionaries");
    let fixture = "CREATE TABLE tbl_1_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
        INSERT INTO tbl_1_n VALUES('ni','n','你',100);
        CREATE TABLE tbl_1_h(key TEXT,jp TEXT,value TEXT,weight INTEGER);
        INSERT INTO tbl_1_h VALUES('hao','h','好',100);
        CREATE TABLE tbl_2_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
        INSERT INTO tbl_2_n VALUES('ni''hao','nh','你好',1000);";
    for root in [&resources, &dictionaries] {
        rusqlite::Connection::open(root.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(fixture)
            .unwrap();
    }
    let mut preferences = chinese_preferences();
    preferences.scheme = InputScheme::Shuangpin;
    preferences.shuangpin_helpcode.enabled = true;
    preferences.shuangpin_helpcode.modes.manual_tab = true;
    let options = json!({"api_version":1, "resources": resources, "dictionaries":dictionaries,
        "user_data":path("user"), "cache":path("cache"), "preferences":preferences,
        "phrase_preedit":true})
    .to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    read(msime_client_focus(handle, true));
    let view = type_letters(handle, b"nihc");
    let index = view["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .position(|candidate| candidate["text"] == "你")
        .unwrap();
    let held = read(msime_client_select(
        handle,
        view["generation"].as_u64().unwrap(),
        index,
    ));
    let view = &held["value"]["view"];
    assert_eq!(view["phrase_prefix"], "你");
    assert_eq!(view["editing_text"], "hc");
    assert!(
        view["spelling_symbols"].as_str().unwrap().contains('\t'),
        "{view}"
    );
    let tab = read(msime_client_character(handle, b'\t', false));
    assert_eq!(tab["value"]["handled"], true, "{tab}");
    assert_eq!(tab["value"]["view"]["editing_text"], "hc|");
    read(msime_client_destroy(handle));
}

#[test]
fn helpcode_settings_switch_independently_after_composition() {
    use msime_client_core::preferences::{HelpcodePreferences, HelpcodeSchema};
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'U', true));
    let before = read(msime_client_view(handle))["value"].clone();
    let mut preferences = Preferences {
        quanpin_helpcode: HelpcodePreferences {
            trigger_key: ";".into(),
            enabled: false,
            schema: HelpcodeSchema::Xiaohe,
            show_in_candidate_window: false,
            strict_filter: false,
            grid: false,
            modes: msime_client_core::preferences::ShuangpinHelpcodeModes::default(),
        },
        shuangpin_helpcode: HelpcodePreferences {
            trigger_key: "/".into(),
            enabled: true,
            schema: HelpcodeSchema::Shouyou2,
            show_in_candidate_window: true,
            strict_filter: false,
            grid: false,
            modes: msime_client_core::preferences::ShuangpinHelpcodeModes::default(),
        },
        ..Preferences::default()
    };
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
    assert_eq!(read(msime_client_view(handle))["value"], before);
    SESSIONS.with(|sessions| assert!(sessions.borrow()[&handle].options.helpcode));
    read(msime_client_command(handle, 3));
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].options.helpcode);
        assert_eq!(sessions.borrow()[&handle].options.helpcode_schema, "xiaohe");
        assert!(!sessions.borrow()[&handle].options.show_helpcode);
        assert_eq!(
            sessions.borrow()[&handle].options.helpcode_trigger_key,
            Some(b';')
        );
    });
    preferences.scheme = InputScheme::Shuangpin;
    assert_eq!(update(handle, 2, &preferences)["value"]["deferred"], false);
    SESSIONS.with(|sessions| {
        assert!(sessions.borrow()[&handle].options.helpcode);
        assert!(sessions.borrow()[&handle].options.show_helpcode);
        assert_eq!(
            sessions.borrow()[&handle].options.helpcode_trigger_key,
            Some(b'/')
        );
        assert_eq!(
            sessions.borrow()[&handle].options.helpcode_schema,
            "shouyou2_0"
        );
    });
    preferences.scheme = InputScheme::Quanpin;
    update(handle, 3, &preferences);
    SESSIONS.with(|sessions| assert!(!sessions.borrow()[&handle].options.helpcode));
    read(msime_client_destroy(handle));
}

#[test]
fn autocorrect_update_waits_for_composition_end() {
    let dir = tempfile::tempdir().unwrap();
    let disabled_preferences = Preferences {
        quanpin: msime_client_core::preferences::QuanpinPreferences {
            autocorrect_transposition: false,
            autocorrect_neighbor: false,
        },
        ..chinese_preferences()
    };
    let handle = test_host_preferences(dir.path(), disabled_preferences.clone());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'U', true));
    let before = read(msime_client_view(handle))["value"].clone();
    let preferences = Preferences {
        quanpin: msime_client_core::preferences::QuanpinPreferences {
            autocorrect_transposition: true,
            autocorrect_neighbor: true,
        },
        ..Preferences::default()
    };
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
    assert_eq!(read(msime_client_view(handle))["value"], before);
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].options.autocorrect_transposition);
        assert!(!sessions.borrow()[&handle].options.autocorrect_neighbor);
    });
    read(msime_client_command(handle, 3));
    SESSIONS.with(|sessions| {
        assert!(sessions.borrow()[&handle].options.autocorrect_transposition);
        assert!(sessions.borrow()[&handle].options.autocorrect_neighbor);
    });
    assert_eq!(
        update(handle, 2, &disabled_preferences)["value"]["deferred"],
        false
    );
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow()[&handle].options.autocorrect_transposition);
        assert!(!sessions.borrow()[&handle].options.autocorrect_neighbor);
    });
    read(msime_client_destroy(handle));
}

#[test]
#[cfg(not(target_os = "android"))]
fn skin_catalog_reaches_native_presenters_without_the_settings_shell() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("skins");
    let scan = |path: &str| read(unsafe { msime_client_skin_catalog(path.as_ptr(), path.len()) });
    let path = root.to_str().unwrap().to_owned();
    // An absent root is an empty catalog, not a failure the presenter shows.
    assert_eq!(
        scan(&path),
        json!({"ok": true, "value": {"packages": [], "issues": []}})
    );
    std::fs::create_dir_all(root.join("sample")).unwrap();
    std::fs::write(
        root.join("sample/skin.toml"),
        "schema_version = 1\nid = 'sample'\nname = 'Sample'\nversion = '1.0'\n\
         base = 'system'\n[supports]\nlayouts = ['vertical']\nthemes = ['light']\n\
         [candidate_window]\nmin_width_dip = 10\n[candidate_window.decoration]\n\
         top_inset_dip = 0\nwidth_dip = 0\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("broken")).unwrap();
    std::fs::write(root.join("broken/skin.toml"), "not a manifest").unwrap();
    let catalog = scan(&path);
    assert_eq!(catalog["ok"], true);
    assert_eq!(catalog["value"]["packages"][0]["id"], "sample");
    assert_eq!(catalog["value"]["packages"][0]["minWidthDip"], 10.0);
    assert_eq!(catalog["value"]["packages"][0]["layouts"][0], "vertical");
    assert_eq!(catalog["value"]["packages"].as_array().unwrap().len(), 1);
    // A package that fails validation is reported, never offered for rendering.
    assert_eq!(catalog["value"]["issues"][0]["folder"], "broken");
    assert_eq!(
        catalog["value"],
        serde_json::to_value(msime_client_services::skin::catalog::scan(&root)).unwrap()
    );
    let relative = "skins";
    assert_eq!(scan(relative)["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_skin_catalog(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
fn custom_helpcode_schemas_reach_native_presenters_without_the_settings_shell() {
    let directory = tempfile::tempdir().unwrap();
    let resources = directory.path().join("resources");
    let scan = |path: &str| {
        read(unsafe {
            msime_client_helpcode_schemas(path.as_ptr(), path.len(), std::ptr::null(), 0)
        })
    };
    let path = resources.to_str().unwrap().to_owned();

    // Missing optional assets are an empty catalog, not an unavailable settings surface.
    assert_eq!(scan(&path), json!({"ok": true, "value": []}));

    let custom = resources.join("helpcodes/custom");
    std::fs::create_dir_all(&custom).unwrap();
    std::fs::write(
        custom.join("synthetic.txt"),
        "# name: Synthetic helper\n# name_en: Synthetic\nword=ab\n",
    )
    .unwrap();
    std::fs::write(custom.join("README.md"), "# name: ignored\n").unwrap();
    assert_eq!(
        scan(&path),
        json!({
            "ok": true,
            "value": [{
                "schema": "custom/synthetic",
                "file_stem": "synthetic",
                "name": "Synthetic helper",
                "name_en": "Synthetic"
            }]
        })
    );

    assert_eq!(scan("relative/resources")["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_helpcode_schemas(std::ptr::null(), 0, std::ptr::null(), 0) })
            ["ok"],
        false
    );

    // 状态根下的表一起列出，同一主干取状态根那份；相对的状态根被拒绝。
    let state = directory.path().join("state");
    let user = state.join("helpcodes/custom");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(user.join("synthetic.txt"), "# name: Mine\nword=ab\n").unwrap();
    std::fs::write(user.join("upgraded.txt"), "word=ab\n").unwrap();
    let with_state = |state: &str| {
        read(unsafe {
            msime_client_helpcode_schemas(path.as_ptr(), path.len(), state.as_ptr(), state.len())
        })
    };
    let listed = with_state(state.to_str().unwrap());
    assert_eq!(listed["ok"], true);
    let listed: Vec<_> = listed["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|schema| (schema["schema"].clone(), schema["name"].clone()))
        .collect();
    assert_eq!(
        listed,
        [
            (json!("custom/synthetic"), json!("Mine")),
            (json!("custom/upgraded"), json!(""))
        ]
    );
    assert_eq!(with_state("relative/state")["ok"], false);
}

#[test]
fn discovered_custom_helpcode_extensions_are_loadable_by_the_engine() {
    let directory = tempfile::tempdir().unwrap();
    let resources = directory.path().join("resources");
    let custom = resources.join("helpcodes/custom");
    std::fs::create_dir_all(&custom).unwrap();
    std::fs::write(custom.join("synthetic.TXT"), "# name: uppercase\n你=ab\n").unwrap();
    let path = resources.to_str().unwrap();
    let catalog = read(unsafe {
        msime_client_helpcode_schemas(path.as_ptr(), path.len(), std::ptr::null(), 0)
    });
    assert_eq!(catalog["ok"], true);
    assert_eq!(catalog["value"].as_array().unwrap().len(), 1);
    let schema = catalog["value"][0]["schema"].as_str().unwrap();
    let loaded = msime_engine::load_helpcode_keymap(&resources, schema).unwrap();
    assert_eq!(loaded.code("你"), Some("ab"));
}

#[test]
fn a_picked_skin_folder_is_copied_in_through_the_c_abi() {
    let files = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let root = state.path().join("skins");
    let source = files.path().join("sakura");
    std::fs::create_dir_all(&source).unwrap();
    // Import checks the whole manifest, so the folder carries a complete one.
    std::fs::write(
        source.join("skin.toml"),
        "schema_version = 1\nid = 'sakura'\nname = 'Sakura'\nversion = '1.0'\nbase = 'system'\n\
         [supports]\nlayouts = ['vertical']\nthemes = ['light']\n[candidate_window]\nmin_width_dip = 10\n",
    )
    .unwrap();
    let call = |request: String| {
        read(unsafe { msime_client_skin_import(request.as_ptr(), request.len()) })
    };
    let imported = call(json!({"source": source, "directory": root}).to_string());
    assert_eq!(imported, json!({"ok": true, "value": {"id": "sakura"}}));
    assert!(root.join("sakura/skin.toml").is_file());
    let bare = files.path().join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    assert_eq!(
        call(json!({"source": bare, "directory": root}).to_string())["error"],
        "skin_manifest"
    );
    assert_eq!(
        call(json!({"source": "sakura", "directory": root}).to_string())["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_skin_import(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
fn an_installed_skin_is_deleted_through_the_c_abi_unless_a_slot_holds_it() {
    use msime_client_core::preferences::PreferencesStore;
    let files = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let root = state.path().join("skins");
    for id in ["sakura", "dusk"] {
        let source = files.path().join(id);
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("skin.toml"),
            format!(
                "schema_version = 1\nid = '{id}'\nname = '{id}'\nversion = '1.0'\nbase = 'system'\n\
                 [supports]\nlayouts = ['vertical']\nthemes = ['light']\n[candidate_window]\nmin_width_dip = 10\n"
            ),
        )
        .unwrap();
        let request = json!({"source": source, "directory": root}).to_string();
        assert_eq!(
            read(unsafe { msime_client_skin_import(request.as_ptr(), request.len()) })["ok"],
            true
        );
    }
    // 偏好与皮肤根目录在同一个状态目录下，与 Windows 设置和 Tauri 相同；深色槽位放着 dusk。
    let store = PreferencesStore::new(state.path());
    let current = store.load().unwrap();
    let mut preferences = current.preferences;
    preferences.custom_theme.candidate_skin_dark = Some("dusk".into());
    store.save(current.revision, preferences).unwrap();
    let call = |request: String| {
        read(unsafe { msime_client_skin_delete(request.as_ptr(), request.len()) })
    };
    let delete = |id: &str| {
        call(
            json!({"directory": root, "preferences_directory": state.path(), "id": id}).to_string(),
        )
    };

    let in_use = delete("dusk");
    assert_eq!(in_use["ok"], false);
    assert_eq!(in_use["error"], "candidate_skin_in_use");
    assert!(root.join("dusk/skin.toml").is_file());

    // 删掉以后回复重新扫描的目录，宿主直接拿它画卡片。
    let deleted = delete("sakura");
    assert_eq!(deleted["ok"], true, "{deleted}");
    assert_eq!(deleted["value"]["synced"], false);
    assert_eq!(
        deleted["value"]["directory"],
        root.to_string_lossy().as_ref()
    );
    let listed: Vec<&str> = deleted["value"]["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|package| package["id"].as_str().unwrap())
        .collect();
    assert_eq!(listed, ["dusk"]);
    assert!(!root.join("sakura").exists());

    assert_eq!(delete("sakura")["error"], "candidate_skin_not_installed");
    assert_eq!(delete("night")["error"], "candidate_skin_package");
    let relative =
        json!({"directory": "skins", "preferences_directory": state.path(), "id": "dusk"});
    assert_eq!(call(relative.to_string())["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_skin_delete(std::ptr::null(), 0) })["ok"],
        false
    );
    assert!(root.join("dusk/skin.toml").is_file());
}

#[test]
fn the_skin_editor_checks_and_installs_drafts_through_the_c_abi() {
    // 1×1 的合成 PNG。
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABpfZFQAAAAABJRU5ErkJggg==";
    let state = tempfile::tempdir().unwrap();
    let root = state.path().join("skins");
    let call = |request: Value| {
        let request = request.to_string();
        read(unsafe { msime_client_candidate_skin_editor(request.as_ptr(), request.len()) })
    };
    let manifest = "schema_version = 1\nid = 'drafted'\nname = '草稿'\nversion = '1.0'\nbase = 'wechat'\npreview = 'preview.png'\n[supports]\nlayouts = ['horizontal']\nthemes = ['light', 'dark']\n[candidate_window]\nmin_width_dip = 0\n";
    // `check` 不写皮肤目录，回复扫描会列出的那一项，外观的颜色已经补齐。
    let checked = call(json!({
        "operation": "check",
        "package_id": "drafted",
        "manifest": manifest,
        "files": {"preview.png": PNG},
    }));
    assert_eq!(checked["ok"], true);
    assert_eq!(checked["value"]["candidate"]["light"]["accent"], "#07C160");
    assert_eq!(checked["value"]["minWidthDip"], 0.0);
    assert!(!root.exists());
    let create = |replace: bool| {
        call(json!({
            "operation": "create",
            "directory": root,
            "package_id": "drafted",
            "manifest": manifest,
            "files": {"preview.png": PNG},
            "replace": replace,
        }))
    };
    assert_eq!(
        create(false),
        json!({"ok": true, "value": {"id": "drafted", "replaced": false}})
    );
    assert!(root.join("drafted/skin.toml").is_file());
    assert_eq!(
        create(false),
        json!({"ok": false, "error": "candidate_skin_exists"})
    );
    assert_eq!(create(true)["value"]["replaced"], true);
    // 清单没通过扫描时带上扫描给出的原因。
    let refused = call(json!({
        "operation": "check",
        "package_id": "drafted",
        "manifest": manifest.replace("min_width_dip = 0", "min_width_dip = 5000"),
    }));
    assert_eq!(
        refused,
        json!({"ok": false, "error": "candidate_skin_package", "detail": "invalid min_width_dip"})
    );
    // 页面不能写到相对路径，也不能发出未知的操作或字段。
    for request in [
        json!({"operation": "create", "directory": "skins", "package_id": "drafted", "manifest": manifest}),
        json!({"operation": "create", "package_id": "drafted", "manifest": manifest}),
        json!({"operation": "delete", "package_id": "drafted", "manifest": manifest}),
        json!({"operation": "check", "package_id": "drafted", "manifest": manifest, "path": "/"}),
    ] {
        assert_eq!(call(request)["error"], "candidate_skin_package");
    }
    assert_eq!(
        read(unsafe { msime_client_candidate_skin_editor(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
#[cfg(not(target_os = "android"))]
fn skin_package_resolves_one_manifest_with_the_catalog_loader() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("skins");
    let call =
        |request: &str| read(unsafe { msime_client_skin_package(request.as_ptr(), request.len()) });
    let request = |id: &str| json!({"directory": root, "id": id}).to_string();
    assert_eq!(
        read(unsafe { msime_client_skin_package(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(call("not json")["ok"], false);
    assert_eq!(
        call(&json!({"directory": "skins", "id": "sample"}).to_string())["ok"],
        false
    );
    assert_eq!(call(&request("sample"))["ok"], false);
    std::fs::create_dir_all(root.join("sample")).unwrap();
    // Literal strings, a multi-line array and an inline table: full TOML, as the settings page and Windows toml++ read it.
    std::fs::write(
        root.join("sample/skin.toml"),
        "schema_version = 1\nid = 'sample'\nname = 'Sample'\nversion = '1.0'\n\
         base = 'system'\n[supports]\nlayouts = [\n  'vertical',\n]\nthemes = ['light']\n\
         [candidate_window]\nmin_width_dip = 1_0\n\
         decoration = { top_inset_dip = 0, width_dip = 0 }\n[candidate.light]\naccent = '#123456'\n",
    )
    .unwrap();
    let package = call(&request("sample"));
    assert_eq!(package["ok"], true, "{package}");
    assert_eq!(package["value"]["minWidthDip"], 10.0);
    assert_eq!(package["value"]["candidate"]["light"]["accent"], "#123456");
    let catalog = read(unsafe {
        let path = root.to_str().unwrap();
        msime_client_skin_catalog(path.as_ptr(), path.len())
    });
    assert_eq!(package["value"], catalog["value"]["packages"][0]);
    assert_eq!(call(&request("night"))["error"], "invalid skin id");
    std::fs::write(root.join("sample/skin.toml"), "schema_version = '1'\n").unwrap();
    assert_eq!(
        call(&request("sample"))["error"],
        "unsupported schema_version"
    );
}

#[test]
fn theme_catalog_lists_every_theme_in_picker_order() {
    let catalog = read(msime_client_theme_catalog());
    assert_eq!(catalog["ok"], true);
    assert_eq!(catalog["value"]["default"], "system");
    let ids: Vec<_> = catalog["value"]["themes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|theme| theme["id"].as_str().unwrap().to_owned())
        .collect();
    // 测试在桌面平台上编译运行，目录里没有只在 iOS 提供的 `native`。
    assert_eq!(
        ids,
        ["system", "shuishan", "light", "paper", "night", "ink", "custom"]
    );
    assert_eq!(catalog["value"]["themes"][0]["platforms"], Value::Null);
    let night = &catalog["value"]["themes"][4];
    assert_eq!(night["title"], "夜青");
    assert_eq!(night["appearance"], "dark");
    assert_eq!(night["candidate"]["accent"], "#4FD1C5");
    assert_eq!(catalog["value"]["themes"][0]["candidate"], Value::Null);
    assert_eq!(catalog["value"]["themes"][6]["keyboard"], Value::Null);
}

#[test]
fn image_toolbar_layout_bridge_uses_cached_metadata_and_rejects_invalid_geometry() {
    let cases: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared/skins/toolbar-image-layout-cases.json"
    )))
    .unwrap();
    let case = &cases[0];
    let call = |value: &Value| {
        let request = value.to_string();
        read(unsafe { msime_client_skin_toolbar_image_layout(request.as_ptr(), request.len()) })
    };
    let valid = json!({"toolbar": case["skin"], "shown": case["shown"]});
    let result = call(&valid);
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], case["expected"]);
    for (pointer, replacement) in [
        ("/toolbar/imageWidthDip", json!(-1)),
        ("/toolbar/imageHeightDip", json!(1e50)),
        ("/toolbar/image", json!("../outside.png")),
        ("/toolbar/buttons/status/xDip", json!(-1)),
        ("/toolbar/buttons/status/widthDip", json!(501)),
        ("/toolbar/buttonDefaults/iconSizeDip", json!(0)),
        ("/toolbar/buttonFlow/gapDip", json!(501)),
        ("/toolbar/buttonFlow/direction", json!("diagonal")),
        ("/toolbar/buttons/status/images/cn", json!("bar.svg")),
        ("/shown", json!(vec![["status", "cn"]; 8])),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(call(&invalid)["ok"], false, "{pointer}");
    }
    assert_eq!(
        read(unsafe { msime_client_skin_toolbar_image_layout(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
fn app_theme_catalog_and_resolution_have_the_documented_shape() {
    let catalog = read(msime_client_app_theme_catalog());
    assert_eq!(catalog["ok"], true);
    assert_eq!(catalog["value"]["default"], "siji");
    let themes = catalog["value"]["app_themes"].as_array().unwrap();
    let ids: Vec<_> = themes
        .iter()
        .map(|theme| theme["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["siji", "chunya", "xiayin", "qiushan", "dongxue"]);
    assert_eq!(themes[0]["season"], Value::Null);
    assert_eq!(themes[0]["seasonal"], true);
    assert_eq!(themes[1]["season"], "spring");
    for theme in themes {
        for mode in ["light", "dark"] {
            for key in [
                "accent",
                "accent_soft",
                "on_accent",
                "background",
                "card",
                "hair",
                "candidate_accent",
            ] {
                assert!(theme[mode][key].is_string(), "{} {mode} {key}", theme["id"]);
            }
        }
    }

    let call = |request: Value| {
        let request = request.to_string();
        read(unsafe { msime_client_resolve_app_theme(request.as_ptr(), request.len()) })
    };
    let autumn = call(json!({"app_theme": "siji", "month": 10, "dark": true}));
    assert_eq!(autumn["ok"], true, "{autumn}");
    assert_eq!(
        autumn["value"],
        json!({
            "id": "siji", "season": "autumn", "accent": "#F3C14F", "accent_soft": "#F3C14F40",
            "on_accent": "#3D3014", "background": "#1C1910", "card": "#28241A", "hair": "#2A2F2A",
            "candidate_accent": "#F3C14F"
        })
    );
    let fixed = call(json!({"app_theme": "chunya", "month": 10, "dark": false}));
    assert_eq!(fixed["value"]["season"], "spring");
    assert_eq!(fixed["value"]["accent"], "#4E9A3A");
    assert_eq!(fixed["value"]["on_accent"], "#FFFFFF");
    assert_eq!(fixed["value"]["candidate_accent"], "#267308");
    let without_month = call(json!({"app_theme": "siji", "dark": false}));
    assert_eq!(without_month["ok"], true);
    for (request, why) in [
        (
            json!({"app_theme": "siji", "month": 13, "dark": false}),
            "month",
        ),
        (json!({"app_theme": "seasons", "dark": false}), "id"),
        (json!({"app_theme": "siji"}), "no mode"),
        (
            json!({"app_theme": "siji", "dark": false, "colour": 1}),
            "unknown key",
        ),
    ] {
        let refused = call(request);
        assert_eq!(refused["ok"], false, "{why}");
        assert_eq!(refused["error"], "invalid app theme request", "{why}");
    }
    assert_eq!(
        read(unsafe { msime_client_resolve_app_theme(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
fn restoring_default_preferences_is_a_locked_compare_and_swap() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferencesStore::new(dir.path());
    let mut edited = Preferences {
        candidate_page_size: 8,
        clipboard_history: true,
        ..Preferences::default()
    };
    edited.voice_input.asr_token = "fixture-asr-token".into();
    let saved = store.save(0, edited).unwrap();
    let path = dir.path().to_str().unwrap();
    let call = |revision: u64| {
        read(unsafe {
            msime_client_restore_default_preferences(path.as_ptr(), path.len(), revision)
        })
    };
    let stale = call(saved.revision + 5);
    assert_eq!(stale["ok"], false);
    assert_eq!(stale["error"], "preferences changed; reload before saving");
    assert_eq!(store.load().unwrap(), saved);
    let restored = call(saved.revision);
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(restored["value"]["revision"], saved.revision + 1);
    assert_eq!(restored["value"]["preferences"]["candidate_page_size"], 6);
    assert_eq!(
        restored["value"]["preferences"]["voice_input"]["asr_token"],
        "fixture-asr-token"
    );
    let loaded = store.load().unwrap();
    assert_eq!(loaded.preferences.candidate_page_size, 6);
    assert!(!loaded.preferences.clipboard_history);
    let relative = "relative/preferences";
    assert_eq!(
        read(unsafe {
            msime_client_restore_default_preferences(relative.as_ptr(), relative.len(), 0)
        })["error"],
        "preferences directory must be absolute"
    );
    assert_eq!(
        read(unsafe { msime_client_restore_default_preferences(std::ptr::null(), 0, 0) })["ok"],
        false
    );
}

#[test]
fn text_trackpad_options_publish_the_shared_defaults_and_slider_ranges() {
    let options = read(msime_client_text_trackpad_options());
    assert_eq!(options["ok"], true);
    assert_eq!(
        options["value"]["horizontal_dp"],
        json!({"minimum": 4, "maximum": 48})
    );
    assert_eq!(
        options["value"]["vertical_dp"],
        json!({"minimum": 8, "maximum": 96})
    );
    assert_eq!(
        options["value"]["defaults"],
        serde_json::to_value(msime_client_core::preferences::TextTrackpadPreferences::default())
            .unwrap()
    );
}

#[test]
fn exported_settings_import_as_a_compare_and_swap_that_keeps_local_services() {
    let source = tempfile::tempdir().unwrap();
    let source_store = PreferencesStore::new(source.path());
    let mut exported = Preferences {
        candidate_page_size: 8,
        ..Preferences::default()
    };
    exported.voice_input.asr_token = "fixture-exported-token".into();
    source_store.save(0, exported).unwrap();
    let source_path = source.path().to_str().unwrap();
    let document =
        read(unsafe { msime_client_export_settings(source_path.as_ptr(), source_path.len()) });
    assert_eq!(document["ok"], true, "{document}");
    let text = document["value"].as_str().unwrap().to_owned();
    assert!(!text.contains("fixture-exported-token"));

    let target = tempfile::tempdir().unwrap();
    let target_store = PreferencesStore::new(target.path());
    let mut local = Preferences::default();
    local.voice_input.asr_token = "fixture-local-token".into();
    // 本机开着剪贴板历史、存着记录；导出的文件里剪贴板历史是默认的关闭，导入后本机仍开着，记录也还在。
    local.clipboard_history = true;
    let saved = target_store.save(0, local).unwrap();
    assert!(target_store
        .capture_clipboard_text("synthetic local history".into())
        .unwrap());
    let history = target.path().join("clipboard_history.json");
    let original_history = std::fs::read(&history).unwrap();
    let target_path = target.path().to_str().unwrap();
    let import = |revision: u64, bytes: &[u8]| {
        read(unsafe {
            msime_client_import_settings(
                target_path.as_ptr(),
                target_path.len(),
                revision,
                bytes.as_ptr(),
                bytes.len(),
            )
        })
    };
    let stale = import(saved.revision + 3, text.as_bytes());
    assert_eq!(stale["error"], "settings_conflict");
    assert_eq!(target_store.load().unwrap(), saved);
    assert_eq!(
        import(saved.revision, b"{\"format\":\"elsewhere\"}")["error"],
        "settings_document_invalid"
    );
    let imported = import(saved.revision, text.as_bytes());
    assert_eq!(imported["ok"], true, "{imported}");
    assert_eq!(imported["value"]["revision"], saved.revision + 1);
    let loaded = target_store.load().unwrap();
    assert_eq!(loaded.preferences.candidate_page_size, 8);
    assert_eq!(
        loaded.preferences.voice_input.asr_token,
        "fixture-local-token"
    );
    assert!(loaded.preferences.clipboard_history);
    assert_eq!(std::fs::read(&history).unwrap(), original_history);
}

#[test]
#[cfg(not(target_os = "android"))]
fn canvas_panel_palette_is_opt_in_and_keeps_picker_precedence() {
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("panel-fixture");
    std::fs::create_dir(&package).unwrap();
    // 合成的 1×1 PNG，不含用户内容。
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==")
        .unwrap();
    std::fs::write(package.join("panel.png"), png).unwrap();
    std::fs::write(
        package.join("skin.toml"),
        r##"
schema_version = 2
id = "panel-fixture"
name = "合成贴图"
version = "1.0"
base = "canvas"
[supports]
layouts = ["vertical"]
themes = ["light", "dark"]
[candidate_window.border_image]
source = "panel.png"
slice_dip = [0]
[candidate.light]
text = "#ABCDEF"
number = "#FEDCBA"
selected = "#112233"
selected_text = "#AABBCC"
selected_number = "#DDEEFF"
preedit_caret = "#778899"
preedit_text = "#123"
preedit_background = "rgba(1, 2, 3, 0.5)"
preedit_divider = "transparent"
selected_translation = "#334455"
[candidate.light.menu]
background = "#abc"
text = "#def"
hover = "#456"
border = "#789"
[candidate.dark]
text = "#123456"
"##,
    )
    .unwrap();
    let call = |drawn: Option<bool>, dark: bool, picked: bool, layout: &str| {
        let mut request = json!({
            "global_theme": "custom", "dark": dark, "layout": layout,
            "skins_directory": directory.path(),
            "custom_theme": {"candidate_skin": "panel-fixture"},
        });
        if let Some(drawn) = drawn {
            request["canvas_panel_drawn"] = json!(drawn);
        }
        if picked {
            request["custom_theme"]["candidate_colors"] =
                json!({"text": "#010203", "selected": "#FFFFFF"});
        }
        let raw = request.to_string();
        let reply = read(unsafe { msime_client_resolve_theme(raw.as_ptr(), raw.len()) });
        assert_eq!(reply["ok"], true, "{reply}");
        reply["value"].clone()
    };
    let fallback = call(None, false, false, "vertical");
    assert_eq!(fallback, call(Some(false), false, false, "vertical"));
    assert_eq!(fallback["candidate_skin"], "panel-fixture");
    assert_ne!(fallback["candidate"]["text"], "#ABCDEF");
    assert_eq!(
        call(Some(true), false, false, "vertical")["candidate"]["text"],
        "#ABCDEF"
    );
    assert_eq!(
        call(Some(true), true, false, "vertical")["candidate"]["text"],
        "#123456"
    );
    let picked = call(Some(true), false, true, "vertical");
    assert_eq!(picked["candidate"]["text"], "#010203");
    assert_eq!(picked["candidate"]["selected"], "#FFFFFF");
    assert_eq!(picked["candidate"]["selected_text"], "#000000");
    assert_eq!(picked["candidate_details"]["preedit_text"], "#010203");
    assert_eq!(
        picked["candidate_details"]["selected_translation"],
        "#0000009D"
    );
    let authored = call(Some(true), false, false, "vertical");
    assert_eq!(authored["candidate"]["selected_text"], "#AABBCC");
    assert_eq!(authored["candidate"]["selected_number"], "#DDEEFF");
    assert_eq!(authored["candidate_details"]["preedit_caret"], "#778899");
    assert_eq!(authored["candidate_details"]["preedit_text"], "#112233");
    assert_eq!(
        authored["candidate_details"]["preedit_background"],
        "#01020380"
    );
    assert_eq!(
        authored["candidate_details"]["preedit_divider"],
        "#00000000"
    );
    for (key, expected) in [
        ("menu_surface", "#AABBCC"),
        ("menu_text", "#DDEEFF"),
        ("menu_hover", "#445566"),
        ("menu_border", "#778899"),
    ] {
        assert_eq!(authored["candidate_details"][key], expected);
        assert_eq!(fallback["candidate_details"][key], expected);
    }
    assert!(call(Some(true), false, false, "horizontal")["candidate_skin"].is_null());
}

#[test]
#[cfg(not(target_os = "android"))]
fn resolve_theme_reads_the_package_from_either_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("skins");
    std::fs::create_dir_all(root.join("sakura")).unwrap();
    std::fs::write(
        root.join("sakura/skin.toml"),
        "schema_version = 1\nid = 'sakura'\nname = 'Sakura'\nversion = '1.0'\n\
         base = 'paper'\n[supports]\nlayouts = ['vertical']\nthemes = ['light']\n\
         [candidate_window]\nmin_width_dip = 0\n\
         decoration = { top_inset_dip = 0, width_dip = 0 }\n[candidate.light]\naccent = '#123456'\n",
    )
    .unwrap();
    let call = |request: Value| {
        let request = request.to_string();
        read(unsafe { msime_client_resolve_theme(request.as_ptr(), request.len()) })
    };
    let custom = json!({"candidate_skin": "sakura", "candidate_colors": {"text": "#010203"}});

    let from_root = call(json!({
        "global_theme": "custom", "custom_theme": custom, "dark": false, "layout": "vertical", "skins_directory": root,
    }));
    assert_eq!(from_root["ok"], true, "{from_root}");
    let value = &from_root["value"];
    assert_eq!(value["source"], "custom");
    assert_eq!(value["appearance"], "light", "{value}");
    assert_eq!(value["candidate_skin"], "sakura");
    assert_eq!(value["candidate"]["accent"], "#123456");
    assert_eq!(value["candidate"]["selected_text"], "#123456");
    assert_eq!(value["candidate"]["text"], "#010203");
    // Paper's own panel shows through where neither the package nor a picker says otherwise.
    assert_eq!(value["candidate"]["surface"], "#F7F5F0");
    assert!(value["keyboard"].is_object());

    // The Linux hosts pass the entry they read from candidate_skin_catalog instead of the root.
    let entry = msime_client_services::skin::catalog::host_candidate_catalog(
        &msime_client_services::skin::catalog::scan(&root),
        &root,
        &["sakura"],
    )["packages"][0]
        .clone();
    let from_entry = call(json!({
        "global_theme": "custom", "custom_theme": custom, "dark": false, "layout": "vertical", "package": entry,
    }));
    assert_eq!(from_entry["value"], from_root["value"]);

    // paper 底的皮肤是浅色皮肤，只画在浅色模式：深色宿主没设深色槽位时不画它，自定义主题落回底和取色器。
    let dark = call(json!({
        "global_theme": "custom", "custom_theme": custom, "dark": true, "layout": "vertical", "skins_directory": root,
    }));
    assert_eq!(dark["ok"], true, "{dark}");
    assert_eq!(dark["value"]["candidate_skin"], Value::Null);
    assert_eq!(dark["value"]["candidate"]["text"], "#010203");

    // A missing package leaves the theme to resolve over the platform tokens.
    let missing = call(json!({
        "global_theme": "custom",
        "custom_theme": {"candidate_skin": "absent"},
        "dark": false,
        "layout": "vertical",
        "skins_directory": root,
    }));
    assert_eq!(missing["value"]["candidate"], Value::Null);
    assert_eq!(missing["value"]["candidate_skin"], Value::Null);

    let builtin = call(json!({"global_theme": "ink", "dark": false, "layout": "horizontal"}));
    assert_eq!(builtin["value"]["source"], "builtin");
    assert_eq!(builtin["value"]["keyboard"]["on_accent"], "#000000");
    // A custom theme with no package and no design still draws its base, keyboard included.
    let based = call(json!({
        "global_theme": "custom", "custom_theme": {"base": "night"}, "dark": false, "layout": "horizontal",
    }));
    assert_eq!(based["value"]["appearance"], "dark");
    assert_eq!(based["value"]["candidate"]["surface"], "#16262F");
    assert_eq!(based["value"]["keyboard"]["background"], "#0F1B22");

    // The package declares only the vertical layout, so a horizontal candidate window draws its base and the pickers without it, from either source.
    for source in [json!({"skins_directory": root}), json!({"package": entry})] {
        let mut request = json!({"global_theme": "custom", "custom_theme": custom, "dark": false, "layout": "horizontal"});
        request
            .as_object_mut()
            .unwrap()
            .extend(source.as_object().unwrap().clone());
        let horizontal = call(request);
        assert_eq!(
            horizontal["value"]["candidate_skin"],
            Value::Null,
            "{horizontal}"
        );
        assert_eq!(horizontal["value"]["candidate"]["accent"], "#2C7A4B");
        assert_eq!(horizontal["value"]["candidate"]["text"], "#010203");
    }

    // A SkinSummary is not a catalog entry: passing one as `package` is refused, not read without its declared modes.
    let summary = serde_json::to_value(
        msime_client_services::skin::catalog::load_package(&root, "sakura").unwrap(),
    )
    .unwrap();
    let refused = call(json!({
        "global_theme": "custom", "custom_theme": custom, "dark": false, "layout": "vertical", "package": summary,
    }));
    assert_eq!(
        refused["error"], "invalid candidate skin catalog entry",
        "{refused}"
    );

    for (request, error) in [
        (json!({"global_theme": "night"}), "invalid theme request"),
        (
            json!({"global_theme": "night", "dark": true}),
            "invalid theme request",
        ),
        (
            json!({"global_theme": "night", "dark": true, "layout": "diagonal"}),
            "invalid theme request",
        ),
        (
            json!({"global_theme": "fluent", "dark": true, "layout": "vertical"}),
            "invalid theme request",
        ),
        (
            json!({"global_theme": "Night", "dark": true, "layout": "vertical"}),
            "invalid theme request",
        ),
        (
            json!({"global_theme": "night", "dark": true, "candidate_skin": "sakura", "layout": "vertical"}),
            "invalid theme request",
        ),
        (
            json!({"global_theme": "custom", "dark": true, "layout": "vertical", "custom_theme": {"candidate_skin": "night"}}),
            "custom theme candidate skin identifier is invalid",
        ),
        (
            json!({"global_theme": "custom", "dark": true, "layout": "vertical", "custom_theme": {"base": "custom"}}),
            "custom theme base must be system or a built-in theme",
        ),
        (
            json!({"global_theme": "custom", "dark": true, "layout": "vertical", "skins_directory": "skins"}),
            "skin directory must be absolute",
        ),
        (
            json!({"global_theme": "custom", "dark": true, "layout": "vertical", "skins_directory": root, "package": {}}),
            "theme request names both a skins directory and a package",
        ),
    ] {
        assert_eq!(call(request)["error"], error);
    }
    assert_eq!(
        read(unsafe { msime_client_resolve_theme(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
#[cfg(not(target_os = "android"))]
fn custom_skin_library_reaches_a_c_abi_host_without_a_second_store() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_str().unwrap().to_owned();
    let call = |request: String| {
        read(unsafe { msime_client_custom_skin_library(request.as_ptr(), request.len()) })
    };
    let read_library = || call(json!({"directory": root}).to_string());
    // An untouched library is an empty list rather than a failure to show.
    assert_eq!(read_library(), json!({"ok": true, "value": []}));

    let design =
        serde_json::to_value(msime_client_core::preferences::TouchKeyboardSkinDesign::default())
            .unwrap();
    let created = call(
        json!({
            "directory": root,
            "action": {"operation": "create", "name": "晨雾", "design": design},
        })
        .to_string(),
    );
    assert_eq!(created["ok"], true);
    assert_eq!(created["value"][0]["name"], "晨雾");
    // A mutation answers with the whole library, so the page redraws from one reply.
    assert_eq!(created["value"], read_library()["value"]);
    let id = created["value"][0]["id"].as_str().unwrap().to_owned();

    // The codes are the ones the shared community pages have wording for; a host forwarding the
    // Display text would put an English log sentence into a Chinese dialog.
    let duplicate = call(
        json!({
            "directory": root,
            "action": {"operation": "create", "name": "晨雾", "design": design},
        })
        .to_string(),
    );
    assert_eq!(duplicate["ok"], false);
    assert_eq!(duplicate["error"], "community_skin_duplicate_name");
    let missing = call(
        json!({
            "directory": root,
            "action": {"operation": "delete", "id": "10000000-0000-4000-8000-000000000009"},
        })
        .to_string(),
    );
    assert_eq!(missing["error"], "community_not_found");
    let blank = call(
        json!({
            "directory": root,
            "action": {"operation": "rename", "id": id, "name": "   "},
        })
        .to_string(),
    );
    assert_eq!(blank["error"], "community_skin_invalid_name");

    let renamed = call(
        json!({
            "directory": root,
            "action": {"operation": "rename", "id": id, "name": "竹影"},
        })
        .to_string(),
    );
    assert_eq!(renamed["value"][0]["name"], "竹影");
    assert_eq!(
        call(json!({"directory": root, "action": {"operation": "delete", "id": id}}).to_string()),
        json!({"ok": true, "value": []})
    );

    // A relative root and a null buffer are refused rather than resolved against whatever the
    // process happens to have as its working directory.
    assert_eq!(call(json!({"directory": "skins"}).to_string())["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_custom_skin_library(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(call("not json".to_owned())["error"], "community_invalid");
}

#[test]
#[cfg(not(target_os = "android"))]
fn installing_a_community_skin_is_one_step_so_a_failed_import_ends_its_trial() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_str().unwrap().to_owned();
    let install = |request: String| {
        read(unsafe { msime_client_community_skin_install(request.as_ptr(), request.len()) })
    };
    let trial = |request: String| {
        read(unsafe { msime_client_keyboard_skin_trial(request.as_ptr(), request.len()) })
    };
    let preferences = msime_client_core::preferences::PreferencesStore::new(directory.path());
    let design = serde_json::to_value(msime_client_core::preferences::TouchKeyboardSkinDesign {
        background: 0x102030,
        ..Default::default()
    })
    .unwrap();
    let id = "10000000-0000-4000-8000-000000000001";

    let installed =
        install(json!({"directory": root, "id": id, "name": "晨雾", "design": design}).to_string());
    assert_eq!(installed["ok"], true);
    assert_eq!(installed["value"]["skin"]["id"], id);
    assert_eq!(installed["value"]["skin"]["name"], "晨雾");
    assert_eq!(installed["value"]["trial"]["name"], "晨雾");
    // The design is on the keyboard, not merely in the library: a gallery that saved a skin
    // without wearing it would make "试用" mean nothing.
    let applied = preferences.load().unwrap();
    assert_eq!(
        applied.preferences.global_theme,
        msime_client_services::skin::theme::GlobalTheme::Custom
    );
    assert_eq!(
        applied
            .preferences
            .custom_theme
            .keyboard
            .as_ref()
            .map(|design| design.background),
        Some(0x102030)
    );

    // Declining puts the previous skin back, which is the whole reason the trial exists.
    let trial_id = installed["value"]["trial"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let restored = trial(
        json!({
            "directory": root,
            "action": {"operation": "finish", "id": trial_id, "keep": false},
        })
        .to_string(),
    );
    assert_eq!(restored["ok"], true);
    let reverted = preferences.load().unwrap();
    assert_ne!(
        reverted.preferences.global_theme,
        msime_client_services::skin::theme::GlobalTheme::Custom
    );
    // The library keeps it: declining the trial is declining to wear it now, not to own it.
    let library = json!({"directory": root}).to_string();
    let saved = read(unsafe { msime_client_custom_skin_library(library.as_ptr(), library.len()) });
    assert_eq!(saved["value"][0]["id"], id);

    // Recovery is safe with nothing pending, because that is exactly when it runs: at startup,
    // before anyone knows whether the last session ended mid-trial.
    assert_eq!(
        trial(json!({"directory": root, "action": {"operation": "restore_pending"}}).to_string())
            ["ok"],
        true
    );

    assert_eq!(
        install(
            json!({"directory": root, "id": "not-a-uuid", "name": "x", "design": design})
                .to_string()
        )["error"],
        "community_invalid"
    );
    assert_eq!(
        install(json!({"directory": "skins", "id": id, "name": "x", "design": design}).to_string())
            ["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_community_skin_install(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_keyboard_skin_trial(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
#[cfg(not(target_os = "android"))]
fn the_reply_library_a_keyboard_rereads_is_written_through_its_own_store() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory
        .path()
        .join("CommunityLibrary.json")
        .to_str()
        .unwrap()
        .to_owned();
    let call = |request: String| {
        read(unsafe { msime_client_community_resource_library(request.as_ptr(), request.len()) })
    };
    let template = |id: &str, prompt: &str| {
        json!({
            "id": id,
            "kind": "reply",
            "name": "高情商",
            "description": "",
            "author": "作者",
            "content": {"prompt": prompt},
            "revision": 1,
            "saves": 0,
            "saved": true,
            "owned": false,
            "rating_count": 0,
            "rating_average": 0.0,
            "my_rating": 0,
        })
    };
    let id = "10000000-0000-4000-8000-000000000001";

    // An untouched library is an empty list, which is what a first run looks like.
    assert_eq!(
        call(json!({"file": file, "action": {"operation": "load"}}).to_string()),
        json!({"ok": true, "value": []})
    );
    let saved = call(
        json!({
            "file": file,
            "action": {"operation": "save_reply", "item": template(id, "换个说法")},
        })
        .to_string(),
    );
    assert_eq!(saved["ok"], true);
    assert_eq!(saved["value"][0]["id"], id);
    assert_eq!(saved["value"][0]["content"]["prompt"], "换个说法");

    // Keeping the same template twice replaces it rather than growing the list: the id is the
    // publication, and a second copy would show up twice in the keyboard's menu.
    let replaced = call(
        json!({
            "file": file,
            "action": {"operation": "save_reply", "item": template(id, "更客气一点")},
        })
        .to_string(),
    );
    assert_eq!(replaced["value"].as_array().unwrap().len(), 1);
    assert_eq!(replaced["value"][0]["content"]["prompt"], "更客气一点");

    // A dictionary is not a reply template. The keyboard's parser skips what it does not recognise,
    // so a host that wrote one here would produce a library that silently lost an entry.
    let wrong_kind = json!({
        "file": file,
        "action": {
            "operation": "save_reply",
            "item": {
                "id": "10000000-0000-4000-8000-000000000002",
                "kind": "dictionary",
                "name": "词库",
                "description": "",
                "author": "作者",
                "content": {"entries": [{"kind": "pinyin", "code": "ni", "word": "你", "weight": 1}]},
                "revision": 1,
                "saves": 0,
                "saved": true,
                "owned": false,
                "rating_count": 0,
                "rating_average": 0.0,
                "my_rating": 0,
            },
        },
    });
    assert_eq!(
        call(wrong_kind.to_string())["error"],
        "community_resource_library_format"
    );

    assert_eq!(
        call(json!({"file": file, "action": {"operation": "remove", "id": id}}).to_string()),
        json!({"ok": true, "value": []})
    );
    // Forgetting something that is not there is not a failure; the page may have been showing a
    // library another process already changed.
    assert_eq!(
        call(json!({"file": file, "action": {"operation": "remove", "id": id}}).to_string())["ok"],
        true
    );
    assert_eq!(
        call(json!({"file": file, "action": {"operation": "remove", "id": "nope"}}).to_string())
            ["error"],
        "community_invalid"
    );
    assert_eq!(
        call(json!({"file": "CommunityLibrary.json", "action": {"operation": "load"}}).to_string())
            ["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_community_resource_library(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
#[cfg(not(target_os = "android"))]
fn ai_skin_planning_keeps_the_instruction_and_the_parser_together() {
    let call = |request: String| {
        read(unsafe { msime_client_ai_skin_plan(request.as_ptr(), request.len()) })
    };

    // The host does not write the instruction. It names the exact document the parser accepts, so
    // a host composing its own would be asking for something the parser was not written against.
    let composed = call(
        json!({"operation": "compose", "prompt": "晨雾里的竹林", "model": "fast"}).to_string(),
    );
    assert_eq!(composed["ok"], true);
    assert_eq!(composed["value"]["path"], "/v1/chat/completions");
    assert_eq!(composed["value"]["body"]["model"], "fast");
    assert_eq!(composed["value"]["body"]["stream"], false);
    assert_eq!(composed["value"]["body"]["messages"][0]["role"], "system");
    assert_eq!(
        composed["value"]["body"]["messages"][0]["content"],
        msime_client_services::skin::ai::AI_SKIN_SYSTEM_PROMPT
    );
    assert_eq!(
        composed["value"]["body"]["messages"][1]["content"],
        "晨雾里的竹林"
    );

    // The bounds are the ones `generate` applies before it spends anything: a prompt refused here
    // is one the service would have refused after four requests.
    assert_eq!(
        call(json!({"operation": "compose", "prompt": "", "model": "fast"}).to_string())["error"],
        "ai_skin_invalid"
    );
    assert_eq!(
        call(json!({"operation": "compose", "prompt": "线\u{7}索", "model": "fast"}).to_string())
            ["error"],
        "ai_skin_invalid"
    );
    assert_eq!(
        call(
            json!({"operation": "compose", "prompt": "x".repeat(501), "model": "fast"}).to_string()
        )["error"],
        "ai_skin_invalid"
    );

    let design = |shape: &str, material: &str, accent: &str, prompt: &str| {
        json!({
            "name": "晨雾",
            "description": "竹林里的薄雾",
            "artworkPrompt": prompt,
            "background": "#E8F0EB",
            "keyBackground": "#FFFFFF",
            "keyForeground": "#17251D",
            "accent": accent,
            "actionBackground": accent,
            "gradientEnd": null,
            "gradientHorizontal": false,
            "keyShape": shape,
            "keyMaterial": material,
            "cornerRadius": 8,
            "borderWidth": 0,
            "shadow": 0.1,
            "pattern": 0,
            "monospaced": false,
        })
    };
    let scene = |suffix: &str| {
        format!(
            "{}{suffix}",
            "远山薄雾中的竹林与流水，主体靠近画面边缘，中央留白".repeat(2)
        )
    };
    let answer = json!({
        "skins": [
            design("pebble", "raised", "#185C47", &scene("甲")),
            design("capsule", "glass", "#1C3F6E", &scene("乙")),
            design("ticket", "paper", "#6E2C1C", &scene("丙")),
        ]
    })
    .to_string();
    let parsed = call(json!({"operation": "parse", "text": answer}).to_string());
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["value"].as_array().unwrap().len(), 3);
    assert_eq!(parsed["value"][0]["artworkPrompt"], scene("甲"));
    assert_eq!(parsed["value"][1]["design"]["keyShape"], "capsule");

    // Three proposals that share a shape are not three skins to choose between, which is what the
    // instruction asked the model for.
    let same_shape = json!({
        "skins": [
            design("pebble", "raised", "#185C47", &scene("甲")),
            design("pebble", "glass", "#1C3F6E", &scene("乙")),
            design("ticket", "paper", "#6E2C1C", &scene("丙")),
        ]
    })
    .to_string();
    assert_eq!(
        call(json!({"operation": "parse", "text": same_shape}).to_string())["error"],
        "ai_skin_response"
    );
    assert_eq!(
        call(json!({"operation": "parse", "text": "not json"}).to_string())["error"],
        "ai_skin_response"
    );

    // A returned picture is checked by the shared client, header and all: a host that trusted the
    // declared type would render whatever arrived under it.
    // base64 of b"\x89PNG\r\n\x1A\nrest" and b"\xFF\xD8\xFFrest", written out so this crate does
    // not take a base64 dependency for two fixtures.
    let png = "iVBORw0KGgpyZXN0";
    assert_eq!(
        call(
            json!({
                "operation": "artwork",
                "artwork": {"b64_json": png, "mime_type": "image/png", "width": 512, "height": 512},
            })
            .to_string()
        )["ok"],
        true
    );
    let jpeg_header_on_a_png_claim = "/9j/cmVzdA==";
    assert_eq!(
        call(
            json!({
                "operation": "artwork",
                "artwork": {
                    "b64_json": jpeg_header_on_a_png_claim,
                    "mime_type": "image/png",
                    "width": 512,
                    "height": 512,
                },
            })
            .to_string()
        )["error"],
        "ai_skin_response"
    );
    assert_eq!(
        read(unsafe { msime_client_ai_skin_plan(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
#[cfg(not(target_os = "android"))]
fn skin_resource_bridge_revalidates_kind_and_package_containment() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("skins");
    let skin = root.join("sample");
    std::fs::create_dir_all(skin.join("images")).unwrap();
    std::fs::write(
        skin.join("skin.toml"),
        "schema_version = 1\nid = 'sample'\nname = 'Sample'\nversion = '1.0'\n\
         base = 'system'\ntoolbar_stylesheet = 'toolbar.css'\npreview = 'images/preview.png'\n\
         [supports]\nlayouts = ['vertical']\nthemes = ['dark']\n\
         [candidate_window]\nmin_width_dip = 10\n\
         [candidate_window.decoration]\ntop_inset_dip = 1\nwidth_dip = 10\n",
    )
    .unwrap();
    std::fs::write(skin.join("images/preview.png"), [1_u8, 2, 3]).unwrap();
    std::fs::write(skin.join("font.woff2"), [4_u8, 5, 6]).unwrap();
    std::fs::write(skin.join("toolbar.css"), ".sample { color: red; }").unwrap();
    let directory = root.to_str().unwrap().to_owned();
    let call = |request: Value| {
        let document = request.to_string();
        read(unsafe { msime_client_skin_resource(document.as_ptr(), document.len()) })
    };
    let image = call(json!({
        "directory": directory,
        "id": "sample",
        "relative": "images/preview.png",
        "kind": "image"
    }));
    assert_eq!(image["ok"], true);
    assert_eq!(image["value"]["contentType"], "image/png");
    assert_eq!(image["value"]["bytes"], json!([1, 2, 3]));
    // The iOS keyboard asks for base64 so a large image does not cost a JSON number per byte.
    let encoded = call(json!({
        "directory": directory,
        "id": "sample",
        "relative": "images/preview.png",
        "kind": "image",
        "encoding": "base64"
    }));
    assert_eq!(
        encoded,
        json!({"ok": true, "value": {"contentType": "image/png", "bytesBase64": "AQID"}})
    );
    let unknown_encoding = call(json!({
        "directory": directory,
        "id": "sample",
        "relative": "images/preview.png",
        "kind": "image",
        "encoding": "hex"
    }));
    assert_eq!(
        unknown_encoding,
        json!({"ok": false, "error": "unsupported skin resource encoding"})
    );
    let font = call(json!({
        "directory": root.to_str().unwrap(),
        "id": "sample",
        "relative": "font.woff2",
        "kind": "font"
    }));
    assert_eq!(font["ok"], true);
    assert_eq!(font["value"]["contentType"], "font/woff2");
    let mismatch = call(json!({
        "directory": root.to_str().unwrap(),
        "id": "sample",
        "relative": "toolbar.css",
        "kind": "image"
    }));
    assert_eq!(mismatch["ok"], false);
    let escaped = call(json!({
        "directory": root.to_str().unwrap(),
        "id": "sample",
        "relative": "../toolbar.css",
        "kind": "image"
    }));
    assert_eq!(escaped["ok"], false);
    let stylesheet_request = json!({
        "directory": root.to_str().unwrap(),
        "id": "sample"
    });
    let stylesheet_document = stylesheet_request.to_string();
    let stylesheet = read(unsafe {
        msime_client_skin_toolbar_stylesheet(
            stylesheet_document.as_ptr(),
            stylesheet_document.len(),
        )
    });
    assert_eq!(
        stylesheet,
        json!({"ok": true, "value": ".sample { color: red; }"})
    );
    let invalid = read(unsafe { msime_client_skin_resource(std::ptr::null(), 0) });
    assert_eq!(invalid["ok"], false);
}
#[test]
#[cfg(not(target_os = "android"))]
fn try_preferences_reader_reports_contention_without_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let load = || read(unsafe { msime_client_try_load_preferences(path.as_ptr(), path.len()) });
    let initial = load();
    assert_eq!(initial["ok"], true);
    assert_eq!(initial["value"]["revision"], 0);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("preferences.lock"))
        .unwrap();
    lock.lock().unwrap();
    assert_eq!(load(), json!({"ok": true, "value": null}));
    drop(lock);
    assert_eq!(load(), initial);
    std::fs::write(directory.path().join("preferences.json"), "broken").unwrap();
    assert_eq!(load()["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_try_load_preferences(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
#[cfg(not(target_os = "android"))]
fn recover_preferences_backs_up_malformed_documents_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let recover = || read(unsafe { msime_client_recover_preferences(path.as_ptr(), path.len()) });
    let document = directory.path().join("preferences.json");

    // Missing: nothing is written.
    let missing = recover();
    assert_eq!(missing["ok"], true);
    assert_eq!(missing["value"]["recovered"], false);
    assert_eq!(missing["value"]["snapshot"]["revision"], 0);
    assert!(!document.exists());

    // Malformed: backed up verbatim, then replaced by a document load accepts.
    std::fs::write(&document, "{\"format_version\":1,").unwrap();
    let repaired = recover();
    assert_eq!(repaired["ok"], true, "{repaired}");
    let value = &repaired["value"];
    assert_eq!(value["recovered"], true);
    assert_eq!(value["salvaged"], false);
    let backup = std::path::PathBuf::from(value["backup_path"].as_str().unwrap());
    assert_eq!(backup.parent().unwrap(), directory.path());
    assert_eq!(
        value["backup_name"].as_str().unwrap(),
        backup.file_name().unwrap().to_str().unwrap()
    );
    assert_eq!(std::fs::read(&backup).unwrap(), b"{\"format_version\":1,");
    let loaded = read(unsafe { msime_client_load_preferences(path.as_ptr(), path.len()) });
    assert_eq!(loaded["value"], value["snapshot"]);

    // Valid now: a second call is a no-op.
    let again = recover();
    assert_eq!(again["value"]["recovered"], false);
    assert_eq!(again["value"]["snapshot"], value["snapshot"]);

    // Well-formed but from a newer build: refused and left as it is.
    let future = json!({"format_version": 2, "revision": 3, "preferences": {}}).to_string();
    std::fs::write(&document, &future).unwrap();
    assert_eq!(recover()["ok"], false);
    assert_eq!(std::fs::read_to_string(&document).unwrap(), future);

    assert_eq!(
        read(unsafe { msime_client_recover_preferences(std::ptr::null(), 0) })["ok"],
        false
    );
    let relative = "relative";
    assert_eq!(
        read(unsafe { msime_client_recover_preferences(relative.as_ptr(), relative.len()) })["ok"],
        false
    );
}

#[test]
#[cfg(not(target_os = "android"))]
fn repair_preferences_keeps_what_an_older_build_knows_of_a_newer_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let repair = || read(unsafe { msime_client_repair_preferences(path.as_ptr(), path.len()) });
    let document = directory.path().join("preferences.json");

    // Missing: nothing is written.
    assert_eq!(repair()["value"]["recovered"], false);
    assert!(!document.exists());

    // A newer build added a setting this one does not know. Loading refuses the whole document, and the malformed-only recovery leaves it alone; the explicit repair backs it up verbatim and keeps the settings this schema accepts.
    let newer = json!({
        "format_version": 1,
        "revision": 7,
        "preferences": {"learning": false, "a_setting_from_a_newer_build": true},
    })
    .to_string();
    std::fs::write(&document, &newer).unwrap();
    assert_eq!(
        read(unsafe { msime_client_load_preferences(path.as_ptr(), path.len()) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_recover_preferences(path.as_ptr(), path.len()) })["ok"],
        false
    );
    let repaired = repair();
    assert_eq!(repaired["ok"], true, "{repaired}");
    let value = &repaired["value"];
    assert_eq!(value["recovered"], true);
    assert_eq!(value["salvaged"], true);
    assert_eq!(value["snapshot"]["preferences"]["learning"], false);
    let backup = std::path::PathBuf::from(value["backup_path"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), newer);
    let loaded = read(unsafe { msime_client_load_preferences(path.as_ptr(), path.len()) });
    assert_eq!(loaded["value"], value["snapshot"]);

    // Readable now: a second call is a no-op.
    assert_eq!(repair()["value"]["recovered"], false);

    assert_eq!(
        read(unsafe { msime_client_repair_preferences(std::ptr::null(), 0) })["ok"],
        false
    );
}

#[test]
#[cfg(not(target_os = "android"))]
fn save_preferences_uses_compare_and_swap_and_rejects_invalid_snapshots() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_string_lossy().into_owned();
    let snapshot = serde_json::to_string(&PreferencesSnapshot::default()).unwrap();
    let save = |revision, document: &str| {
        read(unsafe {
            msime_client_save_preferences(
                path.as_ptr(),
                path.len(),
                revision,
                document.as_ptr(),
                document.len(),
            )
        })
    };

    let saved = save(0, &snapshot);
    assert_eq!(saved["ok"], true);
    assert_eq!(saved["value"]["revision"], 1);
    let file = directory.path().join("preferences.json");
    let original = std::fs::read_to_string(&file).unwrap();

    let conflict = save(0, &snapshot);
    assert_eq!(conflict["ok"], false);
    assert!(conflict["error"].as_str().unwrap().contains("changed"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);

    let mut invalid = serde_json::to_value(PreferencesSnapshot::default()).unwrap();
    invalid["format_version"] = json!(2);
    let invalid = invalid.to_string();
    let rejected = save(1, &invalid);
    assert_eq!(rejected["ok"], false);
    assert!(rejected["error"].as_str().unwrap().contains("unsupported"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);

    // 提交来的偏好不走读盘时的宽松读法：重复的动作、本版本不认识的方案和规则都拒绝，文件不动。
    for (pointer, value, error) in [
        (
            "/preferences/touch_text_edit_button_order",
            json!([
                "copy",
                "copy",
                "paste",
                "select_all",
                "cut",
                "space",
                "number_pad",
                "delete"
            ]),
            "text edit button order",
        ),
        (
            "/preferences/touch_text_edit_button_order",
            json!([
                "copy",
                "paste",
                "select_all",
                "cut",
                "space",
                "number_pad",
                "delete",
                "select_word"
            ]),
            "invalid preferences snapshot",
        ),
        (
            "/preferences/touch_keyboard_schemes",
            json!({"enabled": ["quanpin", "typo"]}),
            "invalid preferences snapshot",
        ),
        (
            "/preferences/fuzzy_pinyin/rules",
            json!(["z-zh", "typo"]),
            "invalid preferences snapshot",
        ),
    ] {
        let mut submitted = serde_json::to_value(PreferencesSnapshot::default()).unwrap();
        *submitted.pointer_mut(pointer).unwrap() = value.clone();
        let rejected = save(1, &submitted.to_string());
        assert_eq!(rejected["ok"], false, "{value}");
        assert!(
            rejected["error"].as_str().unwrap().contains(error),
            "{value}: {rejected}"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    }
}
#[test]
#[cfg(not(target_os = "android"))]
fn a_document_holding_a_custom_skin_photo_can_still_be_saved_and_applied() {
    // base64 of a PNG signature followed by zero bytes: about 40 KiB, well past the 16 KiB other buffers are held to.
    let photo = format!("iVBORw0KGgoA{}", "AAAA".repeat(10_000));
    let mut preferences = Preferences::default();
    preferences.custom_theme.keyboard =
        Some(msime_client_core::preferences::TouchKeyboardSkinDesign {
            photo: Some(photo.clone()),
            ..msime_client_core::preferences::TouchKeyboardSkinDesign::default()
        });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_string_lossy().into_owned();
    let snapshot = serde_json::to_string(&PreferencesSnapshot {
        format_version: 1,
        revision: 0,
        preferences: preferences.clone(),
    })
    .unwrap();
    assert!(snapshot.len() > 16_384);
    let saved = read(unsafe {
        msime_client_save_preferences(
            path.as_ptr(),
            path.len(),
            0,
            snapshot.as_ptr(),
            snapshot.len(),
        )
    });
    assert_eq!(saved["ok"], true, "{saved}");
    let loaded = PreferencesStore::new(directory.path()).load().unwrap();
    assert_eq!(
        loaded
            .preferences
            .custom_theme
            .keyboard
            .and_then(|design| design.photo),
        Some(photo)
    );

    let handle = test_host(&directory.path().join("host"));
    assert_eq!(update(handle, 1, &preferences)["ok"], true);
    read(msime_client_destroy(handle));

    // HostOptions carries the same preference object. Session creation must
    // accept the saved design too; the old 16 KiB boundary rejected this
    // otherwise valid configuration before the Engine was even opened.
    let oversized_options = serde_json::to_string(&json!({
        "api_version": 1,
        "resources": directory.path().join("oversized-host/resources"),
        "user_data": directory.path().join("oversized-host/user"),
        "cache": directory.path().join("oversized-host/cache"),
        "dictionaries": directory.path().join("oversized-host/dictionaries"),
        "preferences": preferences,
    }))
    .unwrap();
    for name in ["resources", "user", "cache", "dictionaries"] {
        std::fs::create_dir_all(directory.path().join("oversized-host").join(name)).unwrap();
    }
    assert!(oversized_options.len() > 16_384);
    let oversized_handle =
        read(unsafe { msime_client_create(oversized_options.as_ptr(), oversized_options.len()) });
    assert_eq!(oversized_handle["ok"], true, "{oversized_handle}");
    let oversized_handle = oversized_handle["value"]["session"].as_u64().unwrap();
    read(msime_client_destroy(oversized_handle));
}
#[test]
fn background_preferences_reader_uses_shared_store_and_preserves_bad_files() {
    let directory = tempfile::tempdir().unwrap();
    let saved = PreferencesStore::new(directory.path())
        .save(0, Preferences::default())
        .unwrap();
    let path = directory.path().to_str().unwrap().to_owned();
    let load = |path: String| {
        std::thread::spawn(move || {
            read(unsafe { msime_client_load_preferences(path.as_ptr(), path.len()) })
        })
        .join()
        .unwrap()
    };
    assert_eq!(
        load(path.clone())["value"],
        serde_json::to_value(&saved).unwrap()
    );
    // 文件没变时再读得到同一份快照；保存过之后读到新的那份。
    assert_eq!(
        load(path.clone())["value"],
        serde_json::to_value(&saved).unwrap()
    );
    let resaved = PreferencesStore::new(directory.path())
        .save(
            saved.revision,
            Preferences {
                learning: false,
                ..Preferences::default()
            },
        )
        .unwrap();
    assert_eq!(
        load(path.clone())["value"],
        serde_json::to_value(resaved).unwrap()
    );
    let file = directory.path().join("preferences.json");
    std::fs::write(&file, "broken").unwrap();
    assert_eq!(load(path)["ok"], false);
    assert_eq!(std::fs::read_to_string(file).unwrap(), "broken");
    assert_eq!(load("relative".into())["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_load_preferences(std::ptr::null(), 0) })["ok"],
        false
    );
}
#[test]
fn typing_statistics_boundary_persists_only_aggregate_counts() {
    let directory = tempfile::tempdir().unwrap();
    let call = |action: Value| {
        let request = serde_json::to_vec(&json!({
            "directory": directory.path(),
            "action": action,
        }))
        .unwrap();
        read(unsafe { msime_client_typing_statistics(request.as_ptr(), request.len()) })
    };
    // Statistics ship off, so a fresh directory records nothing until asked. That is the
    // boundary's behaviour too, and it is asserted before turning them on.
    assert_eq!(
        call(json!({"operation": "load"}))["value"]["enabled"],
        false
    );
    assert_eq!(
        call(json!({
            "operation": "record",
            "text": "ignored",
            "source": "handwriting",
            "day": "2026-09-12",
        }))["value"]["recorded"],
        0
    );
    assert_eq!(
        call(json!({"operation": "set_enabled", "enabled": true}))["value"]["enabled"],
        true
    );
    let recorded = call(json!({
        "operation": "record",
        "text": "synthetic 🌲",
        "source": "handwriting",
        "day": "2026-09-12",
    }));
    assert_eq!(recorded["value"]["recorded"], 10);
    let loaded = call(json!({"operation": "load"}));
    assert_eq!(loaded["value"]["total"], 10);
    assert_eq!(loaded["value"]["detail"]["characters"]["latin"], 9);
    assert_eq!(loaded["value"]["detail"]["characters"]["emoji"], 1);
    assert_eq!(loaded["value"]["detail"]["sources"]["handwriting"], 10);
    let persisted =
        std::fs::read_to_string(directory.path().join("typing-statistics.json")).unwrap();
    assert!(!persisted.contains("synthetic"));
    assert_eq!(
        call(json!({"operation": "set_enabled", "enabled": false}))["value"]["enabled"],
        false
    );
    assert_eq!(
        call(json!({
            "operation": "record",
            "text": "ignored",
            "source": "english",
            "day": "2026-09-12",
        }))["value"]["recorded"],
        0
    );
    let reset = call(json!({"operation": "reset"}));
    assert_eq!(reset["value"]["total"], 0);
    assert_eq!(reset["value"]["enabled"], false);
    assert_eq!(
        read(unsafe { msime_client_typing_statistics(std::ptr::null(), 0) })["ok"],
        false
    );
}
#[test]
fn typing_statistics_boundary_answers_the_settings_status() {
    let directory = tempfile::tempdir().unwrap();
    let call = |action: Value, status: bool| {
        let request = serde_json::to_vec(&json!({
            "directory": directory.path(),
            "action": action,
            "status": status,
        }))
        .unwrap();
        read(unsafe { msime_client_typing_statistics(request.as_ptr(), request.len()) })
    };
    let fresh = call(json!({"operation": "load"}), true);
    assert_eq!(fresh["value"]["availability"], "neverWritten");
    assert_eq!(fresh["value"]["lastWrittenMs"], Value::Null);
    assert_eq!(fresh["value"]["statistics"]["enabled"], false);
    let enabled = call(json!({"operation": "set_enabled", "enabled": true}), true);
    assert_eq!(enabled["value"]["availability"], "ready");
    assert!(enabled["value"]["lastWrittenMs"]
        .as_u64()
        .is_some_and(|ms| ms > 0));
    assert_eq!(enabled["value"]["statistics"]["enabled"], true);
    let retention = call(
        json!({"operation": "set_retention", "retention": "30d", "day": "2026-10-11"}),
        true,
    );
    assert_eq!(retention["value"]["statistics"]["retention"], "30d");
    let reset = call(json!({"operation": "reset"}), true);
    assert_eq!(reset["value"]["statistics"]["total"], 0);
    assert_eq!(reset["value"]["availability"], "ready");
    // 不带 status 时回答与以前一样，只有统计本身。
    assert_eq!(
        call(json!({"operation": "load"}), false)["value"]["enabled"],
        true
    );
    // 记录类操作没有状态可答，整个请求被拒绝，也不写入。
    let rejected = call(
        json!({"operation": "record_keys", "day": "2026-10-11", "keys": {"KeyA": 1}}),
        true,
    );
    assert_eq!(rejected["ok"], false);
    assert_eq!(
        call(json!({"operation": "load"}), false)["value"]["dailyKeys"],
        json!({})
    );
}
#[test]
fn typing_statistics_boundary_prepares_uploads_for_hosts_that_send_them() {
    let directory = tempfile::tempdir().unwrap();
    let call = |action: Value| {
        let request = serde_json::to_vec(&json!({
            "directory": directory.path(),
            "action": action,
        }))
        .unwrap();
        read(unsafe { msime_client_typing_statistics(request.as_ptr(), request.len()) })
    };
    let prepare =
        json!({"operation": "upload_request", "day": "2026-10-17", "platform": "android"});
    assert_eq!(
        call(json!({"operation": "upload_status"}))["value"],
        json!({"enabled": false, "lastUploadMs": null})
    );
    assert_eq!(call(prepare.clone())["value"], json!({"skip": "disabled"}));
    assert_eq!(
        call(json!({"operation": "set_upload_enabled", "enabled": true}))["value"],
        json!({"enabled": true, "lastUploadMs": null})
    );
    assert_eq!(call(prepare.clone())["value"], json!({"skip": "nothing"}));
    call(json!({"operation": "set_enabled", "enabled": true}));
    call(
        json!({"operation": "record", "text": "合成", "source": "quanpin", "day": "2026-10-17", "hour": 9}),
    );
    let request = call(prepare.clone())["value"].clone();
    assert_eq!(request["method"], "PUT");
    assert_eq!(request["path"], "/v1/users/me/typing-statistics");
    let device = request["device"].as_str().unwrap().to_owned();
    assert_eq!(device.len(), 32);
    assert_eq!(request["body"]["device"], device.as_str());
    assert_eq!(request["body"]["platform"], "android");
    assert_eq!(request["body"]["days"][0]["day"], "2026-10-17");
    assert_eq!(request["body"]["days"][0]["characters"], 2);
    // 准备不算上传：没记成功时下一次照样交回请求。
    assert_eq!(call(prepare.clone())["value"]["method"], "PUT");
    // 别的设备标识不记；记了以后十分钟内不再交回请求，force 时照样交回。
    assert_eq!(
        call(json!({"operation": "upload_succeeded", "device": "other"}))["value"]["lastUploadMs"],
        Value::Null
    );
    assert!(
        call(json!({"operation": "upload_succeeded", "device": device}))["value"]["lastUploadMs"]
            .as_u64()
            .is_some_and(|ms| ms > 0)
    );
    assert_eq!(call(prepare.clone())["value"], json!({"skip": "not_due"}));
    let mut forced = prepare.clone();
    forced["force"] = json!(true);
    assert_eq!(call(forced)["value"]["method"], "PUT");
    // 关掉后清掉时间；状态文件里没有打出的内容。
    assert_eq!(
        call(json!({"operation": "set_upload_enabled", "enabled": false}))["value"],
        json!({"enabled": false, "lastUploadMs": null})
    );
    let persisted =
        std::fs::read_to_string(directory.path().join("typing-statistics-upload.json")).unwrap();
    assert!(!persisted.contains("合成"));
    // status:true 不用于这几个操作。
    let request = serde_json::to_vec(&json!({
        "directory": directory.path(),
        "action": {"operation": "upload_status"},
        "status": true,
    }))
    .unwrap();
    assert_eq!(
        read(unsafe { msime_client_typing_statistics(request.as_ptr(), request.len()) })["ok"],
        false
    );
}
#[test]
fn typing_statistics_boundary_records_key_press_counts() {
    let directory = tempfile::tempdir().unwrap();
    let call = |action: Value| {
        let request = serde_json::to_vec(&json!({
            "directory": directory.path(),
            "action": action,
        }))
        .unwrap();
        read(unsafe { msime_client_typing_statistics(request.as_ptr(), request.len()) })
    };
    let batch = json!({
        "operation": "record_keys",
        "day": "2026-09-30",
        "keys": {"KeyA": 3, "Space": 2, "Nine4": 1},
    });
    // Off by default: the batch is dropped and nothing is written.
    assert_eq!(call(batch.clone())["value"]["recorded"], 0);
    assert!(!directory.path().join("typing-statistics.json").exists());
    call(json!({"operation": "set_enabled", "enabled": true}));
    assert_eq!(call(batch.clone())["value"]["recorded"], 6);
    assert_eq!(call(batch)["value"]["recorded"], 6);
    let loaded = call(json!({"operation": "load"}));
    assert_eq!(
        loaded["value"]["dailyKeys"],
        json!({"2026-09-30": {"KeyA": 6, "Nine4": 2, "Space": 4}})
    );
    // An unknown id rejects the whole batch, and so does a malformed request shape.
    let rejected = call(json!({
        "operation": "record_keys",
        "day": "2026-09-30",
        "keys": {"KeyA": 1, "a": 1},
    }));
    assert_eq!(rejected["ok"], false);
    for malformed in [
        json!({"operation": "record_keys", "day": "2026-09-30"}),
        json!({"operation": "record_keys", "day": "2026-09-30", "keys": {"KeyA": -1}}),
        json!({"operation": "record_keys", "day": "2026-09-30", "keys": {}, "hour": 9}),
    ] {
        assert_eq!(call(malformed.clone())["ok"], false, "{malformed}");
    }
    assert_eq!(
        call(json!({"operation": "load"}))["value"]["dailyKeys"]["2026-09-30"]["KeyA"],
        6
    );
    let reset = call(json!({"operation": "reset"}));
    assert_eq!(reset["value"]["dailyKeys"], json!({}));
}
/// A session over the two-candidate `nihao` fixture, focused, with typing statistics switched as asked, and the store it writes to.
fn selection_statistics_host(
    root: &std::path::Path,
    enabled: bool,
) -> (u64, TypingStatisticsStore) {
    let store = TypingStatisticsStore::new(root);
    store.set_enabled(enabled).unwrap();
    let handle = statistics_test_host(root, chinese_preferences());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    (handle, store)
}
/// Type `nihao` and commit the candidate at `index` by position, as a click or a tap does.
fn commit_candidate_by_position(handle: u64, index: usize) {
    let mut view = Value::Null;
    for byte in b"nihao" {
        view = read(msime_client_character(handle, *byte, false))["value"]["view"].clone();
    }
    let generation = view["generation"].as_u64().unwrap();
    let selected = read(msime_client_select(handle, generation, index));
    assert!(selected["value"]["commit"].is_string(), "{selected}");
}
#[test]
fn selection_statistics_reach_the_store_at_focus_out() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), true);
    for index in [0, 1, 1] {
        commit_candidate_by_position(handle, index);
    }
    // Held in the session until the field ends, which is the point: no document cycle per selection.
    assert_eq!(store.load().unwrap().selections.total(), 0);
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    let selections = store.load().unwrap().selections;
    assert_eq!(selections.ranks[0], 1);
    assert_eq!(selections.ranks[1], 2);
    assert_eq!(selections.total(), 3);
    // A second focus-out has nothing left to write and must not count anything twice.
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    assert_eq!(store.load().unwrap().selections.total(), 3);
    read(msime_client_destroy(handle));
    assert_eq!(store.load().unwrap().selections.total(), 3);
}
/// 选词位置写进宿主记字数和按键的那份统计（`preferences_directory`），不写进 Engine 的 `user_data`：之前写的是 `user_data`，那里的统计文件不存在、按关闭处理，所有宿主的选词位置都一条没记下，上传的候选位次也一直是空的。
#[test]
fn selection_statistics_land_in_the_hosts_statistics_directory() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), true);
    for index in [0, 1] {
        commit_candidate_by_position(handle, index);
    }
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    let statistics = store.load().unwrap();
    assert_eq!(statistics.selections.total(), 2);
    assert_eq!(statistics.efficiency.commits, 2);
    assert!(!dir.path().join("user/typing-statistics.json").exists());
    read(msime_client_destroy(handle));
}
/// 没有 `preferences_directory` 的会话不知道统计在哪里，选词位置和效率都不记，也不会退回 `user_data` 去写。
#[test]
fn a_session_without_a_statistics_directory_records_no_selections() {
    let dir = tempfile::tempdir().unwrap();
    let user = TypingStatisticsStore::new(dir.path().join("user"));
    user.set_enabled(true).unwrap();
    let handle = test_host_with_pinyin_fixture(dir.path(), chinese_preferences());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    for round in 0..SELECTION_BATCH as usize + 1 {
        commit_candidate_by_position(handle, round % 2);
    }
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    read(msime_client_destroy(handle));
    let statistics = user.load().unwrap();
    assert_eq!(statistics.selections.total(), 0);
    assert_eq!(statistics.efficiency.commits, 0);
}
/// 选项里写了 `selection_statistics: false` 的镜像会话（Windows TIP 跟着 Server 选同一个词）不记选词位置和效率，否则同一次选词会算两遍；同一目录里正常的会话照常记。
#[test]
fn a_mirror_session_leaves_selection_statistics_to_the_session_it_follows() {
    let dir = tempfile::tempdir().unwrap();
    let store = TypingStatisticsStore::new(dir.path());
    store.set_enabled(true).unwrap();
    let mirror = pinyin_fixture_host(
        dir.path(),
        chinese_preferences(),
        json!({ "preferences_directory": dir.path(), "selection_statistics": false }),
    );
    assert_eq!(read(msime_client_focus(mirror, true))["ok"], true);
    for round in 0..SELECTION_BATCH as usize + 1 {
        commit_candidate_by_position(mirror, round % 2);
    }
    assert_eq!(read(msime_client_focus(mirror, false))["ok"], true);
    read(msime_client_destroy(mirror));
    let statistics = store.load().unwrap();
    assert_eq!(statistics.selections.total(), 0);
    assert_eq!(statistics.efficiency.commits, 0);

    // 词库另放一处，统计仍是同一份。
    let other = tempfile::tempdir().unwrap();
    let handle = pinyin_fixture_host(
        other.path(),
        chinese_preferences(),
        json!({ "preferences_directory": dir.path() }),
    );
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    commit_candidate_by_position(handle, 0);
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    read(msime_client_destroy(handle));
    assert_eq!(store.load().unwrap().selections.total(), 1);
}
/// 用户自己关掉学习不影响统计；宿主标出的隐私会话（Android 的隐私模式和不允许学习的输入框）不计选词位置，取消标记后照常计数。
#[test]
fn only_a_private_session_counts_no_selection_statistics() {
    let dir = tempfile::tempdir().unwrap();
    let store = TypingStatisticsStore::new(dir.path());
    store.set_enabled(true).unwrap();
    let preferences = Preferences {
        learning: false,
        ..chinese_preferences()
    };
    let handle = statistics_test_host(dir.path(), preferences);
    SESSIONS.with(|sessions| assert!(!sessions.borrow()[&handle].options.learning));
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    for index in [0, 1] {
        commit_candidate_by_position(handle, index);
    }
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    assert_eq!(store.load().unwrap().selections.total(), 2);

    assert_eq!(
        read(msime_client_set_private_session(handle, true))["value"],
        true
    );
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    commit_candidate_by_position(handle, 0);
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    assert_eq!(store.load().unwrap().selections.total(), 2);

    assert_eq!(
        read(msime_client_set_private_session(handle, false))["value"],
        false
    );
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    commit_candidate_by_position(handle, 0);
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    assert_eq!(store.load().unwrap().selections.total(), 3);
    read(msime_client_destroy(handle));
}

/// 上屏效率：一次带输入码的选择计一次上屏和按下的字母数，宿主标出的隐私会话什么都不计。
#[test]
fn a_committed_selection_counts_its_efficiency_and_a_private_session_counts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), true);
    commit_candidate_by_position(handle, 0);
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    let efficiency = store.load().unwrap().efficiency;
    assert_eq!(efficiency.commits, 1);
    assert_eq!(efficiency.typed_keys, 5);
    // 你好按全拼也是 nihao 五个键：不少按，也不多按。
    assert_eq!(efficiency.spelled_keys, 5);
    assert_eq!(efficiency.prediction_commits, 0);
    read(msime_client_destroy(handle));

    let dir = tempfile::tempdir().unwrap();
    let store = TypingStatisticsStore::new(dir.path());
    store.set_enabled(true).unwrap();
    let handle = statistics_test_host(dir.path(), chinese_preferences());
    assert_eq!(
        read(msime_client_set_private_session(handle, true))["ok"],
        true
    );
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    commit_candidate_by_position(handle, 0);
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    assert_eq!(store.load().unwrap().efficiency.commits, 0);
    read(msime_client_destroy(handle));
}

/// 双拼的「少按键」也以全拼为基准：小鹤双拼 nihc 四个键打出的词，按全拼要 nihao 五个键。
#[test]
fn a_shuangpin_commit_counts_its_keys_against_quanpin() {
    let dir = tempfile::tempdir().unwrap();
    let store = TypingStatisticsStore::new(dir.path());
    store.set_enabled(true).unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Shuangpin,
        shuangpin_profile: ShuangpinProfile::Xiaohe,
        ..chinese_preferences()
    };
    let handle = statistics_test_host(dir.path(), preferences);
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    let mut view = Value::Null;
    for byte in b"nihc" {
        view = read(msime_client_character(handle, *byte, false))["value"]["view"].clone();
    }
    let generation = view["generation"].as_u64().unwrap();
    let selected = read(msime_client_select(handle, generation, 0));
    assert!(selected["value"]["commit"].is_string(), "{selected}");
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    let efficiency = store.load().unwrap().efficiency;
    assert_eq!(efficiency.commits, 1);
    assert_eq!(efficiency.typed_keys, 4);
    assert_eq!(efficiency.spelled_keys, 5);
    read(msime_client_destroy(handle));
}

/// 后台写效率只有一个线程：写的时候排进来的批次由同一个线程接着写，一批也不丢。
#[test]
fn background_efficiency_batches_share_one_writer_and_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), true);
    let options = SESSIONS.with(|sessions| sessions.borrow()[&handle].options.clone());
    read(msime_client_destroy(handle));
    let directory = store.directory().to_path_buf();
    let commit = || EfficiencyCandidate {
        text: "ok".into(),
        typed_keys: 2,
        sentence: false,
        prediction: false,
    };
    for _ in 0..5 {
        record_efficiency_in_background(
            options.clone(),
            directory.clone(),
            vec![commit(), commit()],
        );
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        {
            let queue = efficiency_queue();
            if !queue.running && queue.pending.is_empty() {
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "efficiency writer never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let efficiency = store.load().unwrap().efficiency;
    assert_eq!(efficiency.commits, 10);
    assert_eq!(efficiency.typed_keys, 20);
}

/// 统计关闭时效率一项都不计。
#[test]
fn efficiency_is_not_counted_while_statistics_are_off() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), false);
    commit_candidate_by_position(handle, 0);
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    // 写这一批时读到了关着的开关，直接留作下一批的缓存，不在输入线程上再读一遍文件。
    SESSIONS.with(|sessions| {
        assert_eq!(sessions.borrow()[&handle].statistics_enabled, Some(false));
    });
    assert_eq!(store.load().unwrap().efficiency.commits, 0);
    read(msime_client_destroy(handle));
}

/// 算式结果是生成的候选：通过宿主偏好开启模式后，选择结果既不学习，也不计入选择统计。
#[test]
fn generated_mode_selections_stay_out_of_selection_statistics() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    preferences.local_modes.expression = true;
    let store = TypingStatisticsStore::new(dir.path());
    store.set_enabled(true).unwrap();
    let handle = statistics_test_host(dir.path(), preferences);
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    let entered = read(msime_client_character(handle, b'V', true));
    assert_eq!(entered["value"]["view"]["local_mode"], "expression");
    assert_eq!(
        entered["value"]["view"]["spelling_symbols"],
        "0123456789+-*/.()%^#$&"
    );
    let mut view = Value::Null;
    for byte in b"1+2" {
        view = read(msime_client_character(handle, *byte, false))["value"]["view"].clone();
    }
    let generation = view["generation"].as_u64().unwrap();
    let selected = read(msime_client_select(handle, generation, 1));
    assert_eq!(selected["value"]["commit"], "1+2=3");
    assert_eq!(
        selected["value"]["commit_context"]["local_mode"],
        "expression"
    );
    assert_eq!(
        selected["value"]["commit_context"]["typing_statistics"],
        false
    );
    commit_candidate_by_position(handle, 1);
    assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
    let selections = store.load().unwrap().selections;
    assert_eq!(selections.ranks[1], 1);
    assert_eq!(selections.total(), 1);
    read(msime_client_destroy(handle));
}
/// 拆字反查的开关：偏好里没有这个键时按宿主形态决定（触屏开、桌面关；Windows `radical_mode` 出厂开与本仓桌面缺省不同，差异已记录），用户明确打开后 Shift+C 进入；繁体输出开着时，转成同一个繁体字形的行只留一行。
#[test]
fn chaizi_follows_the_host_form_until_the_user_flips_it() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = chinese_preferences();
    assert_eq!(preferences.local_modes.chaizi, None);
    let handle = test_host_with_pinyin_fixture(&dir.path().join("unset"), preferences);
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    let entered = read(msime_client_character(handle, b'C', true));
    let expected = if TOUCH_KEYBOARD_HOST {
        "chaizi"
    } else {
        "none"
    };
    assert_eq!(entered["value"]["view"]["local_mode"], expected);
    read(msime_client_destroy(handle));

    let mut preferences = chinese_preferences();
    preferences.local_modes.chaizi = Some(true);
    preferences.traditional_chinese_output = true;
    let handle = test_host_with_pinyin_fixture(&dir.path().join("on"), preferences);
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    let entered = read(msime_client_character(handle, b'C', true));
    assert_eq!(entered["value"]["view"]["local_mode"], "chaizi");
    let mut view = Value::Null;
    for byte in b"yanyi" {
        view = read(msime_client_character(handle, *byte, false))["value"]["view"].clone();
    }
    let texts: Vec<&str> = view["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| candidate["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts.first(), Some(&"议"), "{texts:?}");
    assert!(!texts.contains(&"議"), "{texts:?}");
    read(msime_client_destroy(handle));

    let mut preferences = chinese_preferences();
    preferences.local_modes.chaizi = Some(false);
    let handle = test_host_with_pinyin_fixture(&dir.path().join("off"), preferences);
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    let entered = read(msime_client_character(handle, b'C', true));
    assert_eq!(entered["value"]["view"]["local_mode"], "none");
    read(msime_client_destroy(handle));
}

/// 拆字表的繁体字形列是生成器按 OpenCC s2t 规则算的；宿主上屏时用的是 client-base 的同一套转换。两边对每个字都必须得出同一个字形，Engine 按这一列去重才和宿主看到的一致。
#[test]
fn chaizi_traditional_forms_match_the_shared_conversion() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../engine/src/local/chaizi-chars.tsv");
    let table = std::fs::read_to_string(&path).unwrap();
    let mut rows = 0;
    for line in table.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 4, "{line}");
        let expected = if fields[3].is_empty() {
            fields[0]
        } else {
            fields[3]
        };
        assert_eq!(
            msime_client_core::chinese_conversion::simplified_to_traditional(fields[0]),
            expected,
            "{line}"
        );
        rows += 1;
    }
    assert!(rows > 20_000, "{rows}");
}

#[test]
fn selection_statistics_reach_the_store_when_the_session_is_destroyed() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), true);
    commit_candidate_by_position(handle, 1);
    commit_candidate_by_position(handle, 0);
    assert_eq!(store.load().unwrap().selections.total(), 0);
    // No focus-out first: a host tearing the session down directly still keeps what it counted.
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
    let selections = store.load().unwrap().selections;
    assert_eq!(selections.ranks[0], 1);
    assert_eq!(selections.ranks[1], 1);
    assert_eq!(selections.total(), 2);
}
/// personal_ngram_store.cpp:254-255 wrote the queued personal context from `atexit`. A host that quits without a focus-out or destroy (macOS `[NSApp terminate:]`) calls `msime_client_flush_all` first, which writes the selection counts its sessions hold and every queued personal-context transition.
#[test]
fn flush_all_writes_what_live_sessions_still_hold() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), true);
    commit_candidate_by_position(handle, 1);
    commit_candidate_by_position(handle, 0);
    assert_eq!(store.load().unwrap().selections.total(), 0);
    assert_eq!(read(msime_client_flush_all())["ok"], true);
    assert_eq!(store.load().unwrap().selections.total(), 2);
    let pairs: i64 = rusqlite::Connection::open(dir.path().join("user/msime_user.db"))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM personal_bigram WHERE previous=char(1) AND word='拟好'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(pairs > 0, "the queued personal context was not written");
    // The session is untouched and a second call has nothing left to count twice.
    assert_eq!(read(msime_client_flush_all())["ok"], true);
    assert_eq!(store.load().unwrap().selections.total(), 2);
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
    assert_eq!(store.load().unwrap().selections.total(), 2);
}
#[test]
fn selection_statistics_are_written_once_a_batch_fills() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), true);
    let batch = SELECTION_BATCH as usize;
    for round in 0..batch - 1 {
        commit_candidate_by_position(handle, round % 2);
    }
    assert_eq!(store.load().unwrap().selections.total(), 0);
    // The selection that fills the batch hands it to the process's statistics batch, with no focus-out, which bounds what a killed process loses. `load` writes that batch first, as every reader of the statistics does.
    commit_candidate_by_position(handle, 0);
    assert_eq!(store.load().unwrap().selections.total(), SELECTION_BATCH);
    // 填满这一批的那次上屏，效率也在同一批里写下。
    assert_eq!(store.load().unwrap().efficiency.commits, SELECTION_BATCH);
    commit_candidate_by_position(handle, 1);
    assert_eq!(store.load().unwrap().selections.total(), SELECTION_BATCH);
    read(msime_client_focus(handle, false));
    let selections = store.load().unwrap().selections;
    assert_eq!(selections.total(), SELECTION_BATCH + 1);
    assert_eq!(selections.ranks[0], SELECTION_BATCH / 2 + 1);
    assert_eq!(selections.ranks[1], SELECTION_BATCH / 2);
    read(msime_client_destroy(handle));
}
/// 交出选词在宿主的输入线程上，只把计数攒进本进程的统计批次，不读写统计文件：这一路要是读了文件，读不出的那一批就丢了。批次随后由失焦或收尾的 flush 写出，一次也不少。
#[test]
fn selections_handed_over_on_the_input_thread_wait_for_the_statistics_flush() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), true);
    let path = store.directory().join("typing-statistics.json");
    let readable = std::fs::read(&path).unwrap();
    // 文件读不出来：之前那样在选词的线程上读写，就会把这一批丢掉；别的测试这期间做的 flush 也读不出，只能留着它。
    std::fs::write(&path, b"not statistics").unwrap();
    for round in 0..SELECTION_BATCH as usize {
        commit_candidate_by_position(handle, round % 2);
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"not statistics");
    std::fs::write(&path, &readable).unwrap();
    // 和宿主失焦、销毁会话时一样把本进程的批次写出去，再直接看文件。
    msime_client_services::typing_statistics::flush_pending_commits();
    let written: msime_client_services::typing_statistics::TypingStatistics =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written.selections.total(), SELECTION_BATCH);
    assert_eq!(written.selections.ranks[0], SELECTION_BATCH / 2);
    assert_eq!(written.selections.ranks[1], SELECTION_BATCH / 2);
    read(msime_client_destroy(handle));
    assert_eq!(store.load().unwrap().selections.total(), SELECTION_BATCH);
}
#[test]
fn selection_statistics_write_nothing_while_switched_off() {
    let dir = tempfile::tempdir().unwrap();
    let (handle, store) = selection_statistics_host(dir.path(), false);
    let path = store.directory().join("typing-statistics.json");
    let before = std::fs::read(&path).unwrap();
    let written = std::fs::metadata(&path).unwrap().modified().unwrap();
    for round in 0..SELECTION_BATCH as usize + 3 {
        commit_candidate_by_position(handle, round % 2);
    }
    read(msime_client_focus(handle, false));
    read(msime_client_destroy(handle));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        written
    );
    assert_eq!(store.load().unwrap().selections.total(), 0);
}
#[test]
fn clipboard_reader_respects_preferences_and_preserves_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let load = || read(unsafe { msime_client_load_clipboard_history(path.as_ptr(), path.len()) });
    let store = PreferencesStore::new(directory.path());
    let saved = store
        .save(
            0,
            Preferences {
                clipboard_history: true,
                ..Preferences::default()
            },
        )
        .unwrap();
    assert_eq!(load()["value"]["entries"], serde_json::json!([]));
    let file = directory.path().join("clipboard_history.json");
    let fixture = r#"["synthetic alpha","synthetic beta","synthetic alpha"]"#;
    std::fs::write(&file, fixture).unwrap();
    assert_eq!(
        load()["value"]["entries"],
        serde_json::json!(["synthetic alpha", "synthetic beta"])
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), fixture);
    std::fs::write(&file, "broken synthetic fixture").unwrap();
    assert_eq!(load()["error"], "clipboard history unavailable");
    let mut preferences = saved.preferences;
    preferences.clipboard_history = false;
    store.save(saved.revision, preferences).unwrap();
    assert_eq!(
        load()["value"],
        serde_json::json!({"enabled": false, "entries": []})
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "broken synthetic fixture"
    );
    // 偏好里别的设置读不了时面板照常可用，同意与否取文档里写的值；整份读不了时按默认的关闭处理，不列出任何条目。
    let mut document = serde_json::to_value(PreferencesSnapshot {
        format_version: 1,
        revision: 9,
        preferences: Preferences {
            clipboard_history: true,
            ..Preferences::default()
        },
    })
    .unwrap();
    document["preferences"]["candidate_layout"] = json!("future_value");
    std::fs::write(&file, fixture).unwrap();
    std::fs::write(
        directory.path().join("preferences.json"),
        document.to_string(),
    )
    .unwrap();
    assert!(store.load().is_err());
    assert_eq!(
        load()["value"]["entries"],
        serde_json::json!(["synthetic alpha", "synthetic beta"])
    );
    std::fs::write(directory.path().join("preferences.json"), "broken").unwrap();
    assert_eq!(
        load()["value"],
        serde_json::json!({"enabled": false, "entries": []})
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), fixture);
    assert_eq!(
        read(unsafe { msime_client_load_clipboard_history(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_load_clipboard_history(b"relative".as_ptr(), 8) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_load_clipboard_history([255u8].as_ptr(), 1) })["ok"],
        false
    );
}

#[test]
fn mobile_clipboard_uses_structured_actions() {
    let directory = tempfile::tempdir().unwrap();
    let call = |action: Value| {
        let request = serde_json::to_vec(&json!({
            "directory": directory.path(),
            "action": action,
        }))
        .unwrap();
        read(unsafe { msime_client_mobile_clipboard_history(request.as_ptr(), request.len()) })
    };
    let initial = call(json!({"operation": "load"}));
    assert_eq!(initial["value"]["entries"], json!([]));
    assert_eq!(initial["value"]["limit"], 50);
    assert!(!directory.path().join("preferences.json").exists());

    assert_eq!(
        call(json!({"operation": "capture", "text": "synthetic older"}))["value"]["captured"],
        true
    );
    assert_eq!(
        call(json!({"operation": "capture", "text": "synthetic current"}))["value"]["captured"],
        true
    );
    assert_eq!(
        call(json!({
            "operation": "set_pinned",
            "text": "synthetic older",
            "pinned": true
        }))["value"]["updated"],
        true
    );
    let pinned = call(json!({"operation": "load"}));
    assert_eq!(pinned["value"]["entries"][0]["text"], "synthetic older");
    assert_eq!(pinned["value"]["entries"][1]["text"], "synthetic current");
    let exported = call(json!({"operation": "export_text"}));
    assert_eq!(exported["value"]["count"], 2);
    let text = exported["value"]["text"].as_str().unwrap();
    assert!(text.contains("1 · 已置顶 ==========\nsynthetic older\n"));
    assert!(text.contains("2 ==========\nsynthetic current\n"));
    assert_eq!(
        call(json!({"operation": "remove", "text": "synthetic current"}))["value"]["removed"],
        true
    );
    assert_eq!(
        call(json!({"operation": "clear"}))["value"]["cleared"],
        true
    );
    assert_eq!(
        call(json!({"operation": "load"}))["value"]["entries"],
        json!([])
    );

    // A corrupt shared history is refused rather than replaced.
    let shared = directory.path().join("MSIME/clipboard_history.json");
    let corrupt = b"invalid synthetic current history";
    std::fs::write(&shared, corrupt).unwrap();
    let refused = call(json!({"operation": "capture", "text": "synthetic rejected"}));
    assert_eq!(refused["ok"], false);
    assert_eq!(std::fs::read(&shared).unwrap(), corrupt);
    assert_eq!(call(json!({"operation": "export_text"}))["ok"], false);

    let null = read(unsafe { msime_client_mobile_clipboard_history(std::ptr::null(), 0) });
    assert_eq!(null["ok"], false);
    let relative = serde_json::to_vec(&json!({
        "directory": "relative",
        "action": {"operation": "load"}
    }))
    .unwrap();
    assert_eq!(
        read(unsafe { msime_client_mobile_clipboard_history(relative.as_ptr(), relative.len()) })
            ["ok"],
        false
    );
}

#[test]
fn mobile_clipboard_uses_separate_preferences_directory_for_capacity() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("bootstrap/state");
    let store = PreferencesStore::new(&state);
    let snapshot = store.load().unwrap();
    let mut preferences = snapshot.preferences;
    preferences.clipboard_history_limit = 100;
    store.save(snapshot.revision, preferences).unwrap();

    for history in [
        directory.path().to_path_buf(),
        directory.path().join("clipboard-visitor"),
    ] {
        let call = |action: Value| {
            let request = serde_json::to_vec(&json!({
                "directory": history,
                "preferences_directory": state,
                "action": action,
            }))
            .unwrap();
            read(unsafe { msime_client_mobile_clipboard_history(request.as_ptr(), request.len()) })
        };
        for index in 0..60 {
            let result =
                call(json!({"operation": "capture", "text": format!("synthetic-{index:03}")}));
            assert_eq!(result["value"]["captured"], true);
        }
        let loaded = call(json!({"operation": "load"}));
        assert_eq!(loaded["value"]["limit"], 100);
        assert_eq!(loaded["value"]["entries"].as_array().unwrap().len(), 60);
        let restarted = call(json!({"operation": "load"}));
        assert_eq!(restarted["value"]["entries"].as_array().unwrap().len(), 60);
        assert!(history.join("MSIME/clipboard_history.json").exists());
    }
}

#[test]
fn mobile_clipboard_categories_validate_and_preserve_entry_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let call = |action: Value| {
        let request =
            serde_json::to_vec(&json!({"directory": directory.path(), "action": action})).unwrap();
        read(unsafe { msime_client_mobile_clipboard_history(request.as_ptr(), request.len()) })
    };
    let limits = call(json!({"operation": "categories"}));
    assert_eq!(limits["value"]["maxCategories"], 5);
    assert_eq!(limits["value"]["maxNameCharacters"], 12);
    for name in ["", "未分类", "带\n换行", "超过十二个字的分类名称不允许保存"]
    {
        assert_eq!(
            call(json!({"operation": "add_category", "name": name}))["ok"],
            false
        );
    }
    assert_eq!(
        call(json!({"operation": "add_category", "name": "合成分类"}))["value"]["updated"],
        true
    );
    assert_eq!(
        call(json!({"operation": "add_category", "name": "合成分类"}))["ok"],
        false
    );
    let category = call(json!({"operation": "categories"}))["value"]["categories"][0].clone();
    let id = category["id"].as_str().unwrap();
    call(json!({"operation": "capture", "text": "synthetic category entry"}));
    assert_eq!(
        call(json!({"operation": "set_category", "text": "synthetic category entry", "id": id}))
            ["value"]["updated"],
        true
    );
    assert_eq!(
        call(json!({"operation": "rename_category", "id": id, "name": "改名"}))["value"]["updated"],
        true
    );
    let snapshot = call(json!({"operation": "load"}));
    assert_eq!(snapshot["value"]["categories"][0]["name"], "改名");
    assert_eq!(snapshot["value"]["entries"][0]["categoryId"], id);
    assert_eq!(
        call(
            json!({"operation": "set_category", "text": "synthetic category entry", "id": "absent"})
        )["ok"],
        false
    );
    assert_eq!(
        call(json!({"operation": "remove_category", "id": id}))["value"]["updated"],
        true
    );
    let snapshot = call(json!({"operation": "load"}));
    assert_eq!(
        snapshot["value"]["entries"][0]["text"],
        "synthetic category entry"
    );
    assert!(snapshot["value"]["entries"][0].get("categoryId").is_none());
    assert_eq!(
        call(json!({"operation": "set_category", "text": "synthetic category entry"}))["value"]
            ["updated"],
        true
    );
}

#[test]
fn mobile_clipboard_replace_edits_in_place_and_reports_why_it_did_not() {
    let directory = tempfile::tempdir().unwrap();
    let call = |action: Value| {
        let request = serde_json::to_vec(&json!({
            "directory": directory.path(),
            "action": action,
        }))
        .unwrap();
        read(unsafe { msime_client_mobile_clipboard_history(request.as_ptr(), request.len()) })
    };
    for text in ["synthetic older", "synthetic current"] {
        assert_eq!(
            call(json!({"operation": "capture", "text": text}))["value"]["captured"],
            true
        );
    }
    let older_timestamp =
        call(json!({"operation": "load"}))["value"]["entries"][1]["timestampMs"].clone();

    // 原地改字：仍排在原来的位置，时间戳不变。
    let replaced = call(json!({
        "operation": "replace",
        "text": "synthetic older",
        "replacement": "synthetic older, trimmed"
    }));
    assert_eq!(replaced["ok"], true);
    assert_eq!(replaced["value"]["replaced"], true);
    assert_eq!(replaced["value"]["merged"], false);
    assert!(replaced["value"]["reason"].is_null());
    assert_eq!(
        replaced["value"]["entries"][1]["text"],
        "synthetic older, trimmed"
    );
    assert_eq!(
        replaced["value"]["entries"][1]["timestampMs"],
        older_timestamp
    );

    // 改成另一条已有的文字：两条合并成一条。
    let merged = call(json!({
        "operation": "replace",
        "text": "synthetic older, trimmed",
        "replacement": "synthetic current"
    }));
    assert_eq!(merged["value"]["replaced"], true);
    assert_eq!(merged["value"]["merged"], true);
    assert_eq!(
        merged["value"]["entries"],
        call(json!({"operation": "load"}))["value"]["entries"]
    );
    assert_eq!(merged["value"]["entries"].as_array().unwrap().len(), 1);

    // 旧条目已经不在了、新文字不合规：不改，说明原因。
    let missing = call(json!({
        "operation": "replace",
        "text": "synthetic absent",
        "replacement": "synthetic replacement"
    }));
    assert_eq!(missing["value"]["replaced"], false);
    assert_eq!(missing["value"]["reason"], "not_found");
    let invalid = call(json!({
        "operation": "replace",
        "text": "synthetic current",
        "replacement": " \n\t"
    }));
    assert_eq!(invalid["value"]["replaced"], false);
    assert_eq!(invalid["value"]["reason"], "invalid");
    assert_eq!(
        call(json!({"operation": "load"}))["value"]["entries"][0]["text"],
        "synthetic current"
    );

    // 请求本身不合规的拒收：缺字段、多字段、空的原文。
    for action in [
        json!({"operation": "replace", "text": "synthetic current"}),
        json!({
            "operation": "replace",
            "text": "synthetic current",
            "replacement": "synthetic next",
            "pinned": true
        }),
        json!({"operation": "replace", "text": "", "replacement": "synthetic next"}),
    ] {
        assert_eq!(call(action)["ok"], false);
    }
}

#[test]
fn history_removal_is_exact_idempotent_and_respects_disabled_setting() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("clipboard_history.json");
    let store = PreferencesStore::new(directory.path());
    let saved = store
        .save(
            0,
            Preferences {
                clipboard_history: true,
                ..Preferences::default()
            },
        )
        .unwrap();
    let remove = |path: &std::path::Path, text: &str| {
        let request = serde_json::to_vec(&json!({"directory": path, "text": text})).unwrap();
        read(unsafe { msime_client_remove_clipboard_history(request.as_ptr(), request.len()) })
    };
    std::fs::write(&file, br#"["synthetic first","synthetic second"]"#).unwrap();
    assert_eq!(
        remove(directory.path(), "synthetic first")["value"]["removed"],
        true
    );
    let upgraded: Vec<msime_client_core::clipboard::ClipboardHistoryEntry> =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(upgraded.len(), 1);
    assert_eq!(upgraded[0].text, "synthetic second");
    assert_eq!(
        remove(directory.path(), "synthetic first")["value"]["removed"],
        false
    );
    assert_eq!(remove(directory.path(), "")["ok"], false);
    assert_eq!(
        remove(std::path::Path::new("relative"), "synthetic")["ok"],
        false
    );
    let maximum = msime_client_core::clipboard::MAX_TEXT_BYTES;
    assert_eq!(remove(directory.path(), &"x".repeat(maximum))["ok"], true);
    assert_eq!(
        remove(directory.path(), &"x".repeat(maximum + 1))["ok"],
        false
    );
    let mut preferences = saved.preferences;
    preferences.clipboard_history = false;
    store.save(saved.revision, preferences).unwrap();
    assert_eq!(
        remove(directory.path(), "synthetic second")["error"],
        "clipboard history disabled"
    );
    let preserved: Vec<msime_client_core::clipboard::ClipboardHistoryEntry> =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(preserved, upgraded);
    assert_eq!(
        read(unsafe { msime_client_remove_clipboard_history(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_remove_clipboard_history(b"{".as_ptr(), 1) })["ok"],
        false
    );
}

#[test]
fn history_capture_bridge_validates_and_returns_no_text() {
    let directory = tempfile::tempdir().unwrap();
    let capture = |request: Value| {
        let bytes = serde_json::to_vec(&request).unwrap();
        read(unsafe { msime_client_capture_clipboard_history(bytes.as_ptr(), bytes.len()) })
    };
    let request = json!({"directory": directory.path(), "text": "synthetic capture"});
    assert_eq!(
        capture(request.clone())["value"],
        json!({"captured": false})
    );
    assert!(!directory.path().join("clipboard_history.json").exists());
    PreferencesStore::new(directory.path())
        .save(
            0,
            Preferences {
                clipboard_history: true,
                ..Preferences::default()
            },
        )
        .unwrap();
    assert_eq!(capture(request.clone())["value"], json!({"captured": true}));
    assert_eq!(
        capture(json!({"directory": "relative", "text": "synthetic"}))["ok"],
        false
    );
    let maximum = msime_client_core::clipboard::MAX_TEXT_BYTES;
    assert_eq!(
        capture(json!({"directory": directory.path(), "text": "x".repeat(maximum)}))["ok"],
        true
    );
    assert_eq!(
        capture(json!({"directory": directory.path(), "text": "x".repeat(maximum + 1)}))["ok"],
        false
    );
    assert_eq!(capture(json!({"directory": directory.path()}))["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_capture_clipboard_history(std::ptr::null(), 0) })["ok"],
        false
    );
    std::fs::write(directory.path().join("preferences.json"), "broken").unwrap();
    assert_eq!(
        capture(request)["error"],
        "clipboard history capture failed"
    );
}

pub(super) fn test_host(root: &std::path::Path) -> u64 {
    test_host_preferences(root, chinese_preferences())
}
/// 夹具的资源目录带上一个五笔码表文件（空的 SQLite 库）。macOS 和 Android 的发布包可以不带 `msime-wubi.db`，资源目录里没有它时 host-api 把五笔当作没装（`wubi_dictionary_missing`），这些夹具要的是一个五笔照常可用的安装。
fn bundle_wubi_table(resources: &std::path::Path) {
    std::fs::write(resources.join("msime-wubi.db"), b"").unwrap();
}
fn chinese_preferences() -> Preferences {
    Preferences {
        default_ime_mode: msime_client_core::preferences::DefaultImeMode::Chinese,
        ..Preferences::default()
    }
}
fn test_host_preferences(root: &std::path::Path, preferences: Preferences) -> u64 {
    let path = |name| {
        let path = root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    bundle_wubi_table(&path("resources"));
    let options = json!({ "api_version": 1, "resources": path("resources"), "user_data": path("user"), "cache": path("cache"), "dictionaries": path("dictionaries"), "preferences": preferences }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true);
    created["value"]["session"].as_u64().unwrap()
}
fn test_host_with_pinyin_fixture(root: &std::path::Path, preferences: Preferences) -> u64 {
    pinyin_fixture_host(root, preferences, json!({}))
}

/// 同 [`test_host_with_pinyin_fixture`]，并像各平台宿主那样把 `root` 作为 `preferences_directory`：打字统计就在 `root` 下。
fn statistics_test_host(root: &std::path::Path, preferences: Preferences) -> u64 {
    pinyin_fixture_host(root, preferences, json!({ "preferences_directory": root }))
}

/// `extra` 的字段原样并进会话选项。
fn pinyin_fixture_host(root: &std::path::Path, preferences: Preferences, extra: Value) -> u64 {
    let path = |name| {
        let path = root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let resources = path("resources");
    let dictionaries = path("dictionaries");
    let fixture = "CREATE TABLE tbl_2_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_2_n VALUES('ni''hao','nh','本地',100),('ni''hao','nh','拟好',80);
                   CREATE TABLE tbl_2_z(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                   INSERT INTO tbl_2_z VALUES('zheng''fu','zf','政府',100),('zheng''fu','zf','正负',80);
                   CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);
                   CREATE TABLE quick_parases(key TEXT,value TEXT,weight INTEGER);
                   CREATE INDEX idx_quick_parases_key_weight ON quick_parases(key,weight DESC);";
    for directory in [&resources, &dictionaries] {
        rusqlite::Connection::open(directory.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(fixture)
            .unwrap();
    }
    bundle_wubi_table(&resources);
    let mut options = json!({ "api_version": 1, "resources": resources, "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries, "preferences": preferences });
    for (key, value) in extra.as_object().unwrap() {
        options[key] = value.clone();
    }
    let options = options.to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true);
    created["value"]["session"].as_u64().unwrap()
}

fn test_host_with_multilingual_fixture(root: &std::path::Path, preferences: Preferences) -> u64 {
    let path = |name| {
        let path = root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let resources = path("resources");
    let dictionaries = path("dictionaries");
    for directory in [&resources, &dictionaries] {
        rusqlite::Connection::open(directory.join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(
                "CREATE TABLE tbl_2_w(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                 INSERT INTO tbl_2_w VALUES('wang''luo','wl','网络',100);",
            )
            .unwrap();
        let english = directory.join("msime-english.db");
        msime_engine::ensure_english_schema(&english).unwrap();
        rusqlite::Connection::open(english)
            .unwrap()
            .execute_batch("INSERT INTO english_words VALUES('hello','hello',100);")
            .unwrap();
    }
    bundle_wubi_table(&resources);
    let options = json!({
        "api_version": 1,
        "resources": resources,
        "user_data": path("user"),
        "cache": path("cache"),
        "dictionaries": dictionaries,
        "preferences": preferences
    })
    .to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    created["value"]["session"].as_u64().unwrap()
}

#[test]
fn multilingual_preferences_reach_host_session_and_rebuild() {
    use msime_client_core::preferences::MultilingualLanguage;

    let directory = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    preferences.multilingual_input.enabled = true;
    preferences.traditional_chinese_output = true;
    preferences.multilingual_input.languages =
        vec![MultilingualLanguage::Chinese, MultilingualLanguage::English];
    let handle = test_host_with_multilingual_fixture(directory.path(), preferences.clone());
    SESSIONS.with(|sessions| {
        assert_eq!(
            sessions.borrow()[&handle].options.multilingual_languages,
            vec![
                msime_engine::multilingual::Language::Chinese,
                msime_engine::multilingual::Language::English,
            ]
        );
    });
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    let mut typed = Value::Null;
    for byte in b"hellowangluo" {
        typed = read(msime_client_character(handle, *byte, false));
    }
    let mixed = typed["value"]["view"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["source"] == 18)
        .cloned()
        .expect("the enabled preference should expose a multilingual candidate");
    assert_eq!(mixed["text"], "hello網絡");
    preferences.multilingual_input.enabled = false;
    preferences.traditional_chinese_output = false;
    let changed = update(handle, 1, &preferences);
    assert_eq!(changed["value"]["deferred"], true);
    assert!(changed["value"]["view"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["source"] == 18 && row["text"] == "hello网络"));
    let selected = read(msime_client_select(
        handle,
        mixed["id"]["generation"].as_u64().unwrap(),
        mixed["id"]["index"].as_u64().unwrap() as usize,
    ));
    assert_eq!(selected["value"]["commit"], "hello网络");
    assert_eq!(
        selected["value"]["commit_context"]["script_conversion"],
        false
    );

    SESSIONS.with(|sessions| {
        assert!(sessions.borrow()[&handle]
            .options
            .multilingual_languages
            .is_empty());
    });
    preferences.multilingual_input.enabled = true;
    assert_eq!(update(handle, 2, &preferences)["value"]["deferred"], false);
    let simplified = type_letters(handle, b"hellowangluo");
    let mixed = simplified["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["source"] == 18 && row["text"] == "hello网络")
        .unwrap();
    preferences.candidate_page_size = 7;
    assert_eq!(update(handle, 3, &preferences)["value"]["deferred"], true);
    let switched = read(msime_client_set_multilingual_output(handle, true));
    assert!(switched["value"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["source"] == 18 && row["text"] == "hello網絡"));
    let selected = read(msime_client_select(
        handle,
        mixed["id"]["generation"].as_u64().unwrap(),
        mixed["id"]["index"].as_u64().unwrap() as usize,
    ));
    assert_eq!(selected["value"]["commit"], "hello網絡");
    let rebuilt = type_letters(handle, b"hellowangluo");
    assert!(rebuilt["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["source"] == 18 && row["text"] == "hello網絡"));
    // 临时宿主投影跨越空闲重建保留，下一次显式偏好更新才接管输出。
    let reset = update(handle, 4, &preferences);
    assert!(reset["value"]["view"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["source"] == 18 && row["text"] == "hello网络"));
    // 同一份偏好原样再交一次（Android 键盘显示期间每秒一次）同样把输出交还给偏好：Windows 宿主每次更新偏好后也按偏好重置自己的繁简状态。
    let switched = read(msime_client_set_multilingual_output(handle, true));
    assert!(switched["value"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["source"] == 18 && row["text"] == "hello網絡"));
    let resent = update(handle, 4, &preferences);
    assert_eq!(resent["ok"], true, "{resent}");
    assert!(resent["value"]["view"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["source"] == 18 && row["text"] == "hello网络"));

    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

#[test]
fn session_reads_live_preferences_when_the_options_copy_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = directory.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let options = |preferences_directory: &std::path::Path, preferences: &serde_json::Value| {
        json!({ "api_version": 1, "resources": path("resources"), "user_data": path("user"), "cache": path("cache"), "dictionaries": path("dictionaries"), "preferences_directory": preferences_directory, "preferences": preferences }).to_string()
    };
    let create =
        |options: String| read(unsafe { msime_client_create(options.as_ptr(), options.len()) });

    // #2830 退役 `autocorrect` 后，带着它的副本曾让每个 macOS 会话都被拒。偏好结构现在不拒绝不认识的键，这样的副本自己就读得进来。
    let mut retired = serde_json::to_value(chinese_preferences()).unwrap();
    retired["autocorrect"] = json!(true);
    let missing = directory.path().join("missing");
    let created = create(options(&missing, &retired));
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
    assert!(!missing.exists());

    // 副本本身读不了（取值类型不对）时，由实时的 preferences.json 顶替。
    let mut stale = serde_json::to_value(chinese_preferences()).unwrap();
    stale["candidate_page_size"] = json!("six");
    let stale_options =
        |preferences_directory: &std::path::Path| options(preferences_directory, &stale);

    // 没有实时文档可以顶替：在内存里救下副本认得的设置照常建会话，读不了的那个键取默认值；看一眼不会创建目录。
    let created = create(stale_options(&missing));
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
    assert!(!missing.exists());

    // 副本解析得了却通不过校验（#9802：旧版本准备时写下的副本，新版本的校验更严）时同样不再拒绝。
    let mut stricter = serde_json::to_value(chinese_preferences()).unwrap();
    stricter["candidate_page_size"] = json!(99);
    let created = create(options(&missing, &stricter));
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);

    let state = path("state");
    PreferencesStore::new(&state)
        .save(0, chinese_preferences())
        .unwrap();
    let created = create(stale_options(&state));
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

#[test]
fn settled_rerank_without_movement_omits_the_unused_view() {
    let directory = tempfile::tempdir().unwrap();
    let handle = test_host(directory.path());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    let result = read(msime_client_rerank_settled(handle));
    assert_eq!(result["ok"], true);
    assert_eq!(result["value"]["moved"], false);
    assert!(result["value"].get("view").is_none());
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

#[test]
fn start_settled_rerank_without_a_model_finishes_on_the_caller() {
    let directory = tempfile::tempdir().unwrap();
    let handle = test_host(directory.path());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    let result = read(msime_client_start_settled_rerank(handle));
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"]["status"], "done");
    assert_eq!(result["value"]["moved"], false);
    assert_eq!(result["value"]["current"], true);
    assert!(result["value"].get("view").is_none());

    let missing = read(msime_client_poll_settled_rerank(handle, 0, true));
    assert_eq!(missing["value"]["status"], "done");
    assert_eq!(missing["value"]["moved"], false);
    assert_eq!(missing["value"]["current"], false);
    let unknown = read(msime_client_poll_settled_rerank(handle, 9, true));
    assert_eq!(unknown["value"]["status"], "done");
    assert_eq!(unknown["value"]["current"], false);
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

#[test]
fn destroying_a_session_drops_its_unfinished_settled_job() {
    let directory = tempfile::tempdir().unwrap();
    let handle = test_host(directory.path());
    let other_directory = tempfile::tempdir().unwrap();
    let other = test_host(other_directory.path());
    let before = ffi::settled::test_settled_job_count();
    let token = ffi::settled::test_hold_settled_job(handle);
    assert_eq!(ffi::settled::test_settled_job_count(), before + 1);

    let pending = read(msime_client_poll_settled_rerank(handle, token, true));
    assert_eq!(pending["value"]["status"], "pending");
    assert_eq!(pending["value"]["token"], token);
    let again = read(msime_client_start_settled_rerank(handle));
    assert_eq!(again["value"]["status"], "pending");
    assert_eq!(again["value"]["token"], token);

    let stolen = read(msime_client_poll_settled_rerank(other, token, true));
    assert_eq!(stolen["value"]["status"], "done");
    assert_eq!(stolen["value"]["current"], false);
    assert_eq!(
        read(msime_client_poll_settled_rerank(handle, token, true))["value"]["status"],
        "pending"
    );

    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
    assert_eq!(ffi::settled::test_settled_job_count(), before);
    assert_eq!(read(msime_client_destroy(other))["ok"], true);
}

/// The desktop model preference switches the runtime's settled rerank, at creation and on a later update. The keyboard model switch (整句联想「增强」) never reaches the Engine, created or rebuilt: it mounts only the runtime's per-keystroke reranker.
#[test]
fn sentence_model_switches_follow_the_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    preferences.sentence_association.neural_desktop = true;
    preferences.sentence_association.neural_keyboard = true;
    let handle = test_host_preferences(directory.path(), preferences.clone());
    let switches = || {
        SESSIONS.with(|sessions| {
            let session = &sessions.borrow()[&handle];
            (
                session.runtime.settled_rerank_enabled(),
                session.options.sentence_association.neural_keyboard,
            )
        })
    };
    assert_eq!(switches(), (true, false));

    preferences.sentence_association.neural_desktop = false;
    preferences.sentence_association.neural_keyboard = false;
    let updated = update(handle, 1, &preferences);
    assert_eq!(updated["value"]["deferred"], false, "{updated}");
    assert_eq!(switches(), (false, false));

    preferences.sentence_association.neural_desktop = true;
    preferences.sentence_association.neural_keyboard = true;
    assert_eq!(update(handle, 2, &preferences)["ok"], true);
    assert_eq!(switches(), (true, false));
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

/// 「增强」只决定 runtime 的逐键重排器：偏好打开且版本带键盘模型时才挂，交给 Engine 的选项里引擎那层键盘模型始终关着。
#[test]
fn enhanced_mounts_the_runtime_reranker_without_the_engine_layer() {
    // 「增强」默认开；选回「标准」时不挂。
    let mut association = msime_client_core::preferences::SentenceAssociationPreferences::default();
    assert!(keyboard_reranker_enabled(&association, Edition::full()));
    association.neural_keyboard = false;
    assert!(!keyboard_reranker_enabled(&association, Edition::full()));
    association.neural_keyboard = true;
    assert!(keyboard_reranker_enabled(&association, Edition::full()));
    assert!(!engine_sentence_association(&association, None).neural_keyboard);
    // 不带键盘模型的版本（五笔版）不挂。
    let wubi = Edition::by_id("wubi").unwrap();
    assert!(!wubi.features.neural_keyboard);
    assert!(!keyboard_reranker_enabled(&association, wubi));
    // 关着时连模型文件都不去读。
    assert!(ffi::keyboard_reranker(false, "", None, None, None, None, None).is_none());
}

/// 会话的那一份把上屏后的上文交给后台线程编码：紧接着按键（线程可能还没做完、也可能做完了）与连着几次上屏只编码最后那段，分数都与不经后台线程时一样；会话放开这一份时线程跟着结束，不挂住进程。
#[cfg(feature = "onnx-rerank")]
#[test]
fn the_session_scorer_primes_the_context_off_the_keystroke() {
    use msime_input_runtime::CandidateScorer as _;
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../onnx-rerank/tests/fixtures");
    let load = || {
        msime_onnx_rerank::OnnxScorer::load_file(
            &fixtures.join("lm_step.ort"),
            &std::fs::read_to_string(fixtures.join("model.json")).unwrap(),
            &std::fs::read_to_string(fixtures.join("vocab.json")).unwrap(),
            1,
            true,
        )
        .unwrap()
    };
    let candidates = ["去北京", "去背景", "去"];
    let context = "我们今天下午";
    let expected = load().log_probabilities(context, &candidates);
    assert_eq!(expected.len(), candidates.len());
    let shared = std::sync::Arc::new(std::sync::Mutex::new(load()));
    let mut session = ffi::SharedOnnxScorer::new(std::sync::Arc::clone(&shared));
    for partial in ["我们", "我们今天", context] {
        session.context_changed(partial);
    }
    let scored = session.log_probabilities(context, &candidates);
    for (got, want) in scored.iter().zip(&expected) {
        assert!((got - want).abs() < 2e-2, "{scored:?} vs {expected:?}");
    }
    assert_eq!(scored.len(), expected.len());
    drop(session);
    // 线程结束后只剩测试这一份引用。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::sync::Arc::strong_count(&shared) > 1 {
        assert!(
            std::time::Instant::now() < deadline,
            "the primer thread outlived its session"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// 跑不进逐键预算的 ONNX 模型只在停顿时重解的宿主（鸿蒙，HostOptions 的 `pause_rerank`）上延后到停顿；只挂按键的宿主照旧，超过逐键上限就不挂。
#[cfg(feature = "onnx-rerank")]
#[test]
fn only_a_pausing_host_defers_a_model_too_slow_for_keys() {
    use ffi::{defers_to_pause, RerankerMount};
    let slow = ffi::PAUSE_HOST_KEYSTROKE_PAGE_MS + 1.0;
    let fast = ffi::PAUSE_HOST_KEYSTROKE_PAGE_MS;
    assert!(defers_to_pause(RerankerMount::KeystrokeOrPause, slow));
    assert!(!defers_to_pause(RerankerMount::KeystrokeOrPause, fast));
    assert!(!defers_to_pause(RerankerMount::Keystroke, slow));
    assert!(!defers_to_pause(RerankerMount::Off, slow));
    // 停顿时重解的宿主对加载放得更宽：设备刚装完应用、忙的那一刻自测会超过逐键上限。
    const { assert!(ffi::PAUSE_RERANK_LIMIT_MS > ffi::KEYSTROKE_RERANK_LIMIT_MS) };
}

/// 随包资源目录旁边有 `rerank-model/` 时，「增强」的逐键重排用它（ONNX Runtime 后端）；再装了 `rerank-model-small/` 时按自测速度在两者之间选；ONNX 模型都读不了就退回随包的句子模型。用 `crates/onnx-rerank` 测试用的小模型，两个目录里放同一份图，只把小模型 `model.json` 的 `context_chars` 改成 48，好从 `context_length`（上文字数加 16）看出挂的是哪一个：大的是 80，小的是 64。
///
/// 模型放在资源目录的兄弟目录里而不是资源目录里：资源目录要与词库锁逐项相符，多一个目录校验就失败（最后一段断言这一点）。`resources` 换成下载的词库代次时按 `bundle`（随包资源目录）找模型。
#[cfg(feature = "onnx-rerank")]
#[test]
fn keyboard_reranker_prefers_the_onnx_model_when_one_is_installed() {
    use msime_input_runtime::CandidateScorer as _;
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../onnx-rerank/tests/fixtures");
    let install = |directory: &std::path::Path, context_chars: u64| {
        std::fs::create_dir(directory).unwrap();
        for name in ["lm_step.ort", "vocab.json"] {
            std::fs::copy(fixtures.join(name), directory.join(name)).unwrap();
        }
        let mut info: Value =
            serde_json::from_str(&std::fs::read_to_string(fixtures.join("model.json")).unwrap())
                .unwrap();
        info["context_chars"] = json!(context_chars);
        std::fs::write(directory.join("model.json"), info.to_string()).unwrap();
    };
    const LARGE: usize = 80;
    const SMALL: usize = 64;
    let bundle = tempfile::tempdir().unwrap();
    let resources = bundle.path().join("EngineResources");
    std::fs::create_dir(&resources).unwrap();
    let directory = bundle.path().join(ffi::RERANK_MODEL_DIRECTORY);
    install(&directory, 64);
    let root = resources.to_str().unwrap();
    assert_eq!(
        ffi::rerank_models_root(root, None, None, None).as_deref(),
        Some(bundle.path())
    );
    let mut scorer = ffi::keyboard_reranker(true, root, None, None, None, None, None)
        .expect("the ONNX model loads");
    assert_eq!(scorer.context_length(), LARGE);
    // 共用的那一份要把整句池也报出来，runtime 才会让引擎多解读法。池多大看这台机器此刻多快，至少 9 条。
    assert!(scorer.sentence_pool() >= msime_engine::ordering::SENTENCE_POOL);
    // 共用的那一份要把 ONNX 后端的混合权重也报出来，不能落回默认的 0.5。
    assert_eq!(scorer.static_evidence_weight(), 0.3);
    // 也要报出它比落定模型强，否则停顿后随包的 model-v1 仍会把它排好的顺序再排一遍。
    assert!(scorer.supersedes_settled_rerank());
    assert_eq!(
        scorer
            .log_probabilities("我们今天", &["去北京", "去背景"])
            .len(),
        2
    );

    // 资源目录里的同名目录不算：那里只放词库锁里的文件。
    let inside = tempfile::tempdir().unwrap();
    install(&inside.path().join(ffi::RERANK_MODEL_DIRECTORY), 64);
    assert!(ffi::keyboard_reranker(
        true,
        inside.path().to_str().unwrap(),
        None,
        None,
        None,
        None,
        None
    )
    .is_none());

    // 进程里的重排器：同一个模型再挂一次是同一份（细节标 `(shared)`），不再各加载一份；「增强」关掉时放开，再打开时重新加载。
    let context =
        |attached: &ffi::AttachedOnnxReranker| attached.scorer.lock().unwrap().context_length();
    let ranks_prediction = |attached: &ffi::AttachedOnnxReranker| {
        attached
            .scorer
            .lock()
            .unwrap()
            .scores_prediction_pool_within_keystroke_budget()
    };
    // 下面直接建的 `OnnxRerankers` 多数用固定的整句池规则（一律 32 条），池不随并行测试时机器的忙闲变。
    let widest = || ffi::OnnxRerankers::with_pool_rule(Box::new(|_| Some(32)));
    let mut rerankers = widest();
    let first = rerankers
        .attach(bundle.path(), 0.0, f64::MAX, None)
        .attached()
        .unwrap();
    // 每个会话都来挂一次（2026-11-02 的 macOS 输入法里三分钟建了 11 个会话，各加载、自测一份大模型，进程占到 558 MB），加载的只有第一次那一份。
    for _ in 0..10 {
        let again = rerankers
            .attach(bundle.path(), 0.0, f64::MAX, None)
            .attached()
            .unwrap();
        assert!(Arc::ptr_eq(&first.scorer, &again.scorer));
        assert!(again.detail.ends_with("(shared)"), "{}", again.detail);
    }
    rerankers.release(bundle.path());
    let reloaded = rerankers
        .attach(bundle.path(), 0.0, f64::MAX, None)
        .attached()
        .unwrap();
    assert!(!Arc::ptr_eq(&first.scorer, &reloaded.scorer));
    assert!(
        !reloaded.detail.ends_with("(shared)"),
        "{}",
        reloaded.detail
    );
    // 别的根目录下没有模型，挂不上，也不放开已经加载的那个。
    assert!(matches!(
        rerankers.attach(&resources, 0.0, f64::MAX, None),
        ffi::OnnxAttach::Unavailable
    ));
    assert!(Arc::ptr_eq(
        &reloaded.scorer,
        &rerankers
            .attach(bundle.path(), 0.0, f64::MAX, None)
            .attached()
            .unwrap()
            .scorer
    ));

    // 下载的词库代次旁边没有模型，按随包资源目录找。
    let generations = tempfile::tempdir().unwrap();
    let generation = generations.path().join("generation");
    std::fs::create_dir(&generation).unwrap();
    let downloaded = ffi::keyboard_reranker(
        true,
        generation.to_str().unwrap(),
        Some(root),
        None,
        None,
        None,
        None,
    )
    .expect("the bundled ONNX model is found through the bundle");
    assert_eq!(downloaded.context_length(), LARGE);

    // 宿主指明模型目录（`rerank_models`）时只在那里找。Windows 的 Server 就是这样：随包资源目录的回退（`helpcode_fallback_resources`）是放辅助码表的 DataDir，装了在线词库更新后资源目录又换成 DataDir 下的代次，两条都推不到 Program Files 里 Server 旁边的模型。
    let data = tempfile::tempdir().unwrap();
    let data_root = data.path().to_str().unwrap();
    let generation_root = generation.to_str().unwrap();
    assert!(ffi::keyboard_reranker(
        true,
        generation_root,
        Some(data_root),
        None,
        None,
        None,
        None
    )
    .is_none());
    let named = ffi::keyboard_reranker(
        true,
        generation_root,
        Some(data_root),
        bundle.path().to_str(),
        None,
        None,
        None,
    )
    .expect("the ONNX model is found in the named directory");
    assert_eq!(named.context_length(), LARGE);

    // Linux 的安装包不带词库：资源目录是用户自己下载的那一份，旁边没有模型；宿主把安装目录里的 `sound-packs` 写成 `sound_packs`，按它的上一级找。Windows 不这样找（那里的音效包目录普通用户可写）。
    let user_data = tempfile::tempdir().unwrap();
    let downloaded_resources = user_data.path().join("resources");
    std::fs::create_dir(&downloaded_resources).unwrap();
    let sound_packs = bundle.path().join("sound-packs");
    let found = ffi::keyboard_reranker(
        true,
        downloaded_resources.to_str().unwrap(),
        None,
        None,
        Some(&sound_packs),
        None,
        None,
    );
    if cfg!(target_os = "windows") {
        assert!(found.is_none());
    } else {
        assert_eq!(
            found
                .expect("the installed ONNX model is found through sound_packs")
                .context_length(),
            LARGE
        );
        assert_eq!(
            ffi::rerank_models_root(
                downloaded_resources.to_str().unwrap(),
                None,
                Some(&sound_packs),
                None
            )
            .as_deref(),
            Some(bundle.path())
        );
    }
    // 资源目录旁边有模型时先用那里的，不看音效包目录。
    assert_eq!(
        ffi::rerank_models_root(
            root,
            None,
            Some(&user_data.path().join("sound-packs")),
            None
        )
        .as_deref(),
        Some(bundle.path())
    );

    // 只装了大的时不换档，换档的预算再小、内存再少也用它；只查它打一页在上限之内，整句池也按这一页定。
    let large_only = widest()
        .attach(bundle.path(), 0.0, f64::MAX, Some(0))
        .attached()
        .unwrap();
    assert_eq!(context(&large_only), LARGE);
    assert_eq!(large_only.category, "keyboard reranker onnx large");
    assert!(
        large_only.detail.contains("(page "),
        "{}",
        large_only.detail
    );
    // 没过换档的自测，所以不排上屏后联想。
    assert!(!ranks_prediction(&large_only));
    // 打一页超过上限就不挂，也不交给随包的句子模型（`OverLimit` 而不是 `Unavailable`）。
    match ffi::OnnxRerankers::default().attach(bundle.path(), 0.0, 0.0, None) {
        ffi::OnnxAttach::OverLimit(detail) => {
            assert!(detail.contains(ffi::RERANK_MODEL_DIRECTORY), "{detail}");
            assert!(detail.contains("limit 0.0 ms"), "{detail}");
        }
        _ => panic!("the only model is over the limit"),
    }

    // 再装一个小模型，两条路都走一遍：自测值在预算之内留用大的，超过预算换小的。`f64::MAX` 的预算只在自测真的量出了一个有限的时间时才留用大的（自测失败返回无穷大）；0 毫秒的预算任何一次真实的图运行都超过。
    let small = bundle.path().join(ffi::SMALL_RERANK_MODEL_DIRECTORY);
    install(&small, 48);
    let mut tiers = widest();
    let fast = tiers
        .attach(bundle.path(), f64::MAX, f64::MAX, None)
        .attached()
        .unwrap();
    assert_eq!(context(&fast), LARGE);
    assert_eq!(fast.category, "keyboard reranker onnx large");
    // 这个小模型打一页远不到一毫秒，按键预算放得下最大的整句池，于是不必再看小模型。
    assert_eq!(
        fast.scorer.lock().unwrap().sentence_pool(),
        msime_engine::ordering::MAX_SENTENCE_POOL
    );
    assert!(fast.detail.contains("sentence pool 32"), "{}", fast.detail);
    // 过了预算的大模型同时也排上屏后联想；退到小模型的机器不排。
    assert!(ranks_prediction(&fast));
    let calibrated = |detail: &str| detail[detail.find("(calibrated").expect(detail)..].to_owned();
    let measured = calibrated(&fast.detail);
    // 自测在进程里只做一次：放开后换一个预算再挂，用的还是那一次量出来的毫秒数，于是换成小的，是另一份重排器。
    tiers.release(bundle.path());
    let slow = tiers
        .attach(bundle.path(), 0.0, f64::MAX, None)
        .attached()
        .unwrap();
    assert_eq!(context(&slow), SMALL);
    assert_eq!(slow.category, "keyboard reranker onnx small");
    assert_eq!(
        calibrated(&slow.detail).split(", budget").next(),
        measured.split(", budget").next()
    );
    assert!(!Arc::ptr_eq(&fast.scorer, &slow.scorer));
    assert!(!ranks_prediction(&slow));
    // 再放开、按原来的预算挂回大的：还是不重测，量过的结论交给新加载的这一份，照样排上屏后联想。
    tiers.release(bundle.path());
    let again = tiers
        .attach(bundle.path(), f64::MAX, f64::MAX, None)
        .attached()
        .unwrap();
    assert_eq!(context(&again), LARGE);
    assert_eq!(
        calibrated(&again.detail).split(", budget").next(),
        measured.split(", budget").next()
    );
    assert!(ranks_prediction(&again));

    // 可用内存不够加载大模型时直接用小的，不先加载大的去自测；档位定下后，内存再多也沿用到放开为止。刚好够时用大的。
    let mut low = widest();
    let short = low
        .attach(
            bundle.path(),
            f64::MAX,
            f64::MAX,
            Some(ffi::LARGE_RERANK_MODEL_MIN_AVAILABLE - 1),
        )
        .attached()
        .unwrap();
    assert_eq!(context(&short), SMALL);
    assert_eq!(short.category, "keyboard reranker onnx small (low memory)");
    assert!(!short.detail.contains("calibrated"), "{}", short.detail);
    assert!(!ranks_prediction(&short));
    let plenty = low
        .attach(bundle.path(), f64::MAX, f64::MAX, Some(u64::MAX))
        .attached()
        .unwrap();
    assert!(Arc::ptr_eq(&short.scorer, &plenty.scorer));
    low.release(bundle.path());
    let enough = low
        .attach(
            bundle.path(),
            f64::MAX,
            f64::MAX,
            Some(ffi::LARGE_RERANK_MODEL_MIN_AVAILABLE),
        )
        .attached()
        .unwrap();
    assert_eq!(context(&enough), LARGE);
    assert!(enough.detail.contains("calibrated"), "{}", enough.detail);

    // 整句池只在选定的模型上定：大模型过了预算就用它，池再小也不换成小模型。
    let only_nine = ffi::OnnxRerankers::with_pool_rule(Box::new(|_| Some(9)))
        .attach(bundle.path(), f64::MAX, f64::MAX, None)
        .attached()
        .unwrap();
    assert_eq!(context(&only_nine), LARGE);
    assert_eq!(only_nine.category, "keyboard reranker onnx large");
    assert_eq!(only_nine.scorer.lock().unwrap().sentence_pool(), 9);
    assert!(
        only_nine.detail.contains("sentence pool 9"),
        "{}",
        only_nine.detail
    );

    // 大模型在预算里只比较得了九条、小模型比较得了整池时挂小模型配整池（桌面上的 8x448 与 6x256）。两个目录里是同一张图，自测毫秒数分不出大小，所以规则按调用的先后给池：每次挂时第一次问的是大模型，之后问的是小模型；每次挂之前把计数清零。小模型的自测按同一个预算记下，同样排上屏后联想。
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = Arc::clone(&asked);
    let mut desktop = ffi::OnnxRerankers::with_pool_rule(Box::new(move |_| {
        let call = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(if call == 0 { 9 } else { 32 })
    }));
    let switched = desktop
        .attach(bundle.path(), f64::MAX, f64::MAX, None)
        .attached()
        .unwrap();
    assert_eq!(context(&switched), SMALL);
    assert_eq!(switched.category, "keyboard reranker onnx small");
    assert_eq!(
        switched.scorer.lock().unwrap().sentence_pool(),
        msime_engine::ordering::MAX_SENTENCE_POOL
    );
    assert!(
        switched.detail.contains("compares 9"),
        "{}",
        switched.detail
    );
    assert!(ranks_prediction(&switched));
    // 量过的结论留着：放开再挂不重测，照样是小模型配整池。
    desktop.release(bundle.path());
    asked.store(0, std::sync::atomic::Ordering::SeqCst);
    let again = desktop
        .attach(bundle.path(), f64::MAX, f64::MAX, None)
        .attached()
        .unwrap();
    assert_eq!(context(&again), SMALL);
    assert!(ranks_prediction(&again));

    // 大模型过不了换档线、小模型打一页又超过上限时什么都不挂（`OverLimit`）。小模型量过的结论留着：再挂时不加载它就知道超了；上限放宽时按量过的那个数挂上它，不重量。
    let mut strict = ffi::OnnxRerankers::default();
    let page = |detail: &str| detail[detail.find("(page").expect(detail)..].to_owned();
    let over = match strict.attach(bundle.path(), 0.0, 0.0, None) {
        ffi::OnnxAttach::OverLimit(detail) => detail,
        _ => panic!("the small model is over a zero limit"),
    };
    assert!(over.contains(ffi::SMALL_RERANK_MODEL_DIRECTORY), "{over}");
    assert!(over.contains("limit 0.0 ms"), "{over}");
    match strict.attach(bundle.path(), 0.0, 0.0, None) {
        ffi::OnnxAttach::OverLimit(again) => assert_eq!(page(&again), page(&over)),
        _ => panic!("the measured verdict is kept"),
    }
    let relaxed = strict
        .attach(bundle.path(), 0.0, f64::MAX, None)
        .attached()
        .unwrap();
    assert_eq!(context(&relaxed), SMALL);
    assert_eq!(
        page(&relaxed.detail).split(", limit").next(),
        page(&over).split(", limit").next()
    );
    // 内存不够、只能用小的时同样要过上限。
    assert!(matches!(
        ffi::OnnxRerankers::default().attach(bundle.path(), f64::MAX, 0.0, Some(0)),
        ffi::OnnxAttach::OverLimit(_)
    ));

    // 宿主用的换档预算是 10 毫秒、上限是 15 毫秒，这个两层、32 维的小模型打一页远在其内，所以挂大的，也排上屏后联想。小模型装上后重新定档位，不沿用只装了大的时加载、没自测的那一份。
    let hosted = ffi::keyboard_reranker(true, root, None, None, None, None, None).unwrap();
    assert_eq!(hosted.context_length(), LARGE);
    assert!(hosted.scores_prediction_pool_within_keystroke_budget());

    // 大的坏了就用小的，不管换档预算，只查上限；小的没过换档的自测，不排上屏后联想。
    std::fs::write(directory.join("lm_step.ort"), b"not a model").unwrap();
    let fallback_small = ffi::keyboard_reranker(true, root, None, None, None, None, None).unwrap();
    assert_eq!(fallback_small.context_length(), SMALL);
    assert!(!fallback_small.scores_prediction_pool_within_keystroke_budget());

    // 两个都坏，资源目录里也没有随包的句子模型，于是什么都不挂。
    std::fs::write(small.join("lm_step.ort"), b"not a model").unwrap();
    assert!(ffi::keyboard_reranker(true, root, None, None, None, None, None).is_none());

    // 两个都坏、但有随包的句子模型时退回它。句子模型按路径缓存，上面那次已经把资源目录里的默认路径记成了「没有」，所以这里经 `configured` 指向另一个文件。
    let sentence = bundle.path().join("fallback-sentence-model.safetensors");
    write_context_free_sentence_model(&sentence);
    let fallback = ffi::keyboard_reranker(true, root, None, None, None, None, sentence.to_str())
        .expect("falls back to the bundled sentence model");
    let bundled = ffi::sentence_model(root, sentence.to_str()).unwrap();
    assert_eq!(fallback.context_length(), bundled.context_length());
    assert_ne!(fallback.context_length(), LARGE);
    assert_ne!(fallback.context_length(), SMALL);
}

/// 整句池按自测速度定：表里的系数是 Mac 上量的，几个真机的自测值落在哪一档要与实测相符。
#[cfg(feature = "onnx-rerank")]
#[test]
fn sentence_pool_follows_the_measured_page() {
    let pools: Vec<Option<usize>> = [0.5, 1.2, 2.0, 4.1, 6.0, 10.0, 30.0]
        .into_iter()
        .map(ffi::sentence_pool_for)
        .collect();
    // 越快的机器池越大，从不变小。
    for pair in pools.windows(2) {
        assert!(pair[0].unwrap_or(0) >= pair[1].unwrap_or(0), "{pools:?}");
    }
    if cfg!(any(target_os = "ios", target_os = "android")) {
        // iPhone 17 的大模型一页 4 ms 左右，比得下 24 条；Pixel 3 XL 的小模型一页 8.9 ms，比得下 9 条（整键实测 9.8 ms），12 条就超了（10.9 ms）。慢到 9 条也放不下时宿主仍给默认的 9 条。
        assert_eq!(ffi::sentence_pool_for(4.1), Some(24));
        assert_eq!(ffi::sentence_pool_for(8.9), Some(9));
        assert_eq!(ffi::sentence_pool_for(10.0), None);
    } else {
        // Mac：小模型一页 1.2 ms 放得下 32 条，大模型一页 4.1 ms 只放得下 9 条。
        assert_eq!(ffi::sentence_pool_for(1.2), Some(32));
        assert_eq!(ffi::sentence_pool_for(4.1), Some(9));
        assert_eq!(ffi::sentence_pool_for(6.0), None);
    }
}

/// Android 发布包不带 ONNX 重排模型，「增强」打开时由设置应用下载 `rerank-model-small` 资源包。随包资源目录旁边没有模型时，逐键重排用状态目录下装好的这个资源包；没装、或装的与锁不一致（过期）时不算，退回随包的句子模型。用 `crates/onnx-rerank` 测试用的小模型冒充资源包里的文件（`installed_file` 只核对清单与锁，不重算摘要），`context_chars` 改成 48，挂上的 `context_length` 是 64。
#[cfg(feature = "onnx-rerank")]
#[test]
fn keyboard_reranker_uses_the_downloaded_small_model_pack() {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../onnx-rerank/tests/fixtures");
    let bundle = tempfile::tempdir().unwrap();
    let resources = bundle.path().join("resources");
    std::fs::create_dir(&resources).unwrap();
    let root = resources.to_str().unwrap();
    let state = tempfile::tempdir().unwrap();
    assert_eq!(
        ResourcePack::RerankModelSmall.id(),
        ffi::SMALL_RERANK_MODEL_DIRECTORY
    );

    // 没装资源包：没有 ONNX 模型的来源，也没有随包的句子模型，什么都不挂。
    assert!(ffi::rerank_models_root(root, None, None, Some(state.path())).is_none());
    assert!(
        ffi::keyboard_reranker(true, root, None, None, None, Some(state.path()), None).is_none()
    );

    let pack = publish_resource_pack(state.path(), ResourcePack::RerankModelSmall, &[]);
    for name in ["lm_step.ort", "vocab.json"] {
        std::fs::copy(fixtures.join(name), pack.join(name)).unwrap();
    }
    let mut info: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures.join("model.json")).unwrap())
            .unwrap();
    info["context_chars"] = json!(48);
    std::fs::write(pack.join("model.json"), info.to_string()).unwrap();
    assert_eq!(
        ffi::rerank_models_root(root, None, None, Some(state.path())),
        Some(resource_packs::root(state.path()))
    );
    assert_eq!(
        ffi::onnx_rerank_source(root, None, None, None, Some(state.path())),
        Some(resource_packs::root(state.path()))
    );
    let attached = ffi::keyboard_reranker(true, root, None, None, None, Some(state.path()), None)
        .expect("the downloaded model loads");
    assert_eq!(attached.context_length(), 64);
    // 没有状态目录的宿主照旧只看安装包。
    assert!(ffi::rerank_models_root(root, None, None, None).is_none());

    // 随包带着模型的平台优先用随包的那份。
    let beside = bundle.path().join(ffi::SMALL_RERANK_MODEL_DIRECTORY);
    std::fs::create_dir(&beside).unwrap();
    assert_eq!(
        ffi::rerank_models_root(root, None, None, Some(state.path())).as_deref(),
        Some(bundle.path())
    );
    std::fs::remove_dir(&beside).unwrap();

    // 清单与锁对不上（锁换了模型，装的是旧的）：不用。
    let mut manifest = ResourcePack::RerankModelSmall.manifest();
    manifest["artifacts"][0]["sha256"] = json!("0".repeat(64));
    std::fs::write(pack.join("msime-model.json"), manifest.to_string()).unwrap();
    assert!(ffi::rerank_models_root(root, None, None, Some(state.path())).is_none());
}

/// 「增强」开着的会话建立时还没有重排模型（Android 上资源包还在下载），设置应用装好 `rerank-model-small` 后，下一次聚焦就把它挂上，不必等会话重建；再聚焦一次、来源没变时不重新加载。
#[cfg(feature = "onnx-rerank")]
#[test]
fn a_rerank_model_pack_installed_after_the_session_opened_is_attached_on_focus() {
    let root = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = root.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let state = path("state");
    let resources = path("resources");
    rusqlite::Connection::open(resources.join("msime-pinyin.db"))
        .unwrap()
        .execute_batch("CREATE TABLE tbl_2_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);")
        .unwrap();
    bundle_wubi_table(&resources);
    let dictionaries = path("dictionaries");
    std::fs::copy(
        resources.join("msime-pinyin.db"),
        dictionaries.join("msime-pinyin.db"),
    )
    .unwrap();
    let mut preferences = chinese_preferences();
    preferences.sentence_association.neural_keyboard = true;
    let options = json!({ "api_version": 1, "resources": resources, "user_data": path("user"), "cache": path("cache"), "dictionaries": dictionaries, "preferences": preferences, "preferences_directory": state }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    let observed = || {
        settle_keyboard_reranker(handle);
        SESSIONS.with(|sessions| {
            let sessions = sessions.borrow();
            let session = sessions.get(&handle).unwrap();
            (
                session.keyboard_reranker_source.clone(),
                session.runtime.has_reranker(),
            )
        })
    };
    // 没有随包的句子模型，也还没有资源包：什么都没挂。
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    assert_eq!(observed(), (None, false));

    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../onnx-rerank/tests/fixtures");
    let pack = publish_resource_pack(&state, ResourcePack::RerankModelSmall, &[]);
    for name in ["lm_step.ort", "model.json", "vocab.json"] {
        std::fs::copy(fixtures.join(name), pack.join(name)).unwrap();
    }
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    assert_eq!(observed(), (Some(resource_packs::root(&state)), true));
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    assert_eq!(observed(), (Some(resource_packs::root(&state)), true));
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

/// 逐键重排器在后台挂（`HostSession::keyboard_reranker_loading`）：等它挂完，再像下一次按键之后那样走一遍 `apply_pending` 把它换上。
fn settle_keyboard_reranker(handle: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let loaded = SESSIONS.with(|sessions| {
            sessions.borrow()[&handle]
                .keyboard_reranker_loading
                .as_ref()
                .is_none_or(|slot| slot.lock().unwrap().is_some())
        });
        if loaded {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the keyboard reranker never finished loading"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    SESSIONS.with(|sessions| {
        sessions
            .borrow_mut()
            .get_mut(&handle)
            .unwrap()
            .apply_pending()
            .unwrap();
    });
}

/// The keyboard model runs inside every keystroke, so it is attached only while `neural_keyboard` (整句联想「增强」) is on: attached on the default 增强, dropped when the user picks 标准, attached again when the switch turns back on. It is attached to the runtime alone; the Engine's own keyboard-model rerank stays off throughout. It takes the real model from `MSIME_EVAL_RESOURCES` or `MSIME_NEURAL_MODEL_DIR` as the Engine's model tests do, and without one a tiny context-free model (`write_context_free_sentence_model`), so CI, which has no model, still checks that session creation and every preference update use `keyboard_reranker_enabled`.
#[test]
fn keyboard_reranker_follows_the_keyboard_model_switch() {
    let synthetic = tempfile::tempdir().unwrap();
    let model = ["MSIME_EVAL_RESOURCES", "MSIME_NEURAL_MODEL_DIR"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|directory| std::path::PathBuf::from(directory).join(ffi::SENTENCE_MODEL_FILE))
        .find(|path| path.is_file())
        .unwrap_or_else(|| {
            let path = synthetic.path().join(ffi::SENTENCE_MODEL_FILE);
            write_context_free_sentence_model(&path);
            path
        });
    let create_with = |root: &std::path::Path, preferences: &Preferences, extra: Value| {
        let path = |name| {
            let path = root.join(name);
            std::fs::create_dir_all(&path).unwrap();
            path
        };
        let mut options = json!({ "api_version": 1, "resources": path("resources"), "user_data": path("user"), "cache": path("cache"), "dictionaries": path("dictionaries"), "sentence_model": model, "preferences": preferences });
        for (key, value) in extra.as_object().unwrap() {
            options[key] = value.clone();
        }
        let options = options.to_string();
        let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
        assert_eq!(created["ok"], true, "{created}");
        created["value"]["session"].as_u64().unwrap()
    };
    let create = |root: &std::path::Path, preferences: &Preferences| {
        create_with(root, preferences, json!({}))
    };
    // 挂没挂 runtime 的逐键重排器；引擎那层键盘模型在任何时候都不打开。
    let attached = |handle: u64| {
        settle_keyboard_reranker(handle);
        SESSIONS.with(|sessions| {
            let session = &sessions.borrow()[&handle];
            assert!(!session.options.sentence_association.neural_keyboard);
            session.runtime.has_reranker()
        })
    };

    let directory = tempfile::tempdir().unwrap();
    let mut preferences = chinese_preferences();
    assert!(preferences.sentence_association.neural_keyboard);
    let handle = create(directory.path(), &preferences);
    // 会话建立时不等模型加载：逐键重排器在后台挂，挂好之前 runtime 上什么都没挂（#10743）。
    SESSIONS.with(|sessions| {
        let session = &sessions.borrow()[&handle];
        assert!(session.keyboard_reranker_loading.is_some());
        assert!(!session.runtime.has_reranker());
    });
    assert!(
        attached(handle),
        "the default 增强 attaches the keyboard model when the session opens"
    );

    preferences.sentence_association.neural_keyboard = false;
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], false);
    assert!(!attached(handle));

    preferences.sentence_association.neural_keyboard = true;
    assert_eq!(update(handle, 2, &preferences)["value"]["deferred"], false);
    assert!(attached(handle));
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);

    // 镜像会话（Windows TIP 在应用进程里的宿主会话，`keyboard_reranker: false`）建立时和开关再打开时都不挂：候选窗的顺序是它跟着的那个会话排的。
    let directory = tempfile::tempdir().unwrap();
    let mirror = create_with(
        directory.path(),
        &preferences,
        json!({ "keyboard_reranker": false }),
    );
    assert!(
        !attached(mirror),
        "a mirror session never loads the keyboard model"
    );
    preferences.sentence_association.neural_keyboard = false;
    assert_eq!(update(mirror, 1, &preferences)["value"]["deferred"], false);
    preferences.sentence_association.neural_keyboard = true;
    assert_eq!(update(mirror, 2, &preferences)["value"]["deferred"], false);
    assert!(!attached(mirror));
    assert_eq!(read(msime_client_destroy(mirror))["ok"], true);

    let directory = tempfile::tempdir().unwrap();
    preferences.sentence_association.neural_keyboard = false;
    let handle = create(directory.path(), &preferences);
    assert!(!attached(handle), "标准 must not run the keyboard model");
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

/// 随包句子模型（没有 ONNX 模型的构建）挂上之前先量一页重负载：在上限之内才挂，超过就不挂；每个模型路径在进程里只量一次，之后按结论定，不再量。
#[test]
fn the_bundled_keyboard_reranker_is_attached_only_within_the_keystroke_limit() {
    let directory = tempfile::tempdir().unwrap();
    let model = directory.path().join("limit-sentence-model.safetensors");
    write_context_free_sentence_model(&model);
    let resources = directory.path().to_str().unwrap();
    let configured = model.to_str();
    // 宿主的上限是 15 ms；这个零层的模型打一页远在其内。
    assert!(
        ffi::bundled_keyboard_reranker(resources, configured, ffi::KEYSTROKE_RERANK_LIMIT_MS)
            .is_some()
    );
    assert!(ffi::bundled_keyboard_reranker(resources, configured, f64::MAX).is_some());
    // 任何一次真实的打分都超过 0 ms 的上限：不挂。
    assert!(ffi::bundled_keyboard_reranker(resources, configured, 0.0).is_none());
    // 没有模型文件时什么都不挂。
    let missing = directory.path().join("missing.safetensors");
    assert!(ffi::bundled_keyboard_reranker(resources, missing.to_str(), f64::MAX).is_none());
}

/// Writes the smallest sentence model `SentenceModel::load` accepts: no layers, a one-wide embedding and only the three special tokens, so every context gets the same logits. It is good for checking that a reranker is mounted, not for anything it would rank.
fn write_context_free_sentence_model(path: &std::path::Path) {
    const CONTEXT: usize = 16;
    let vocabulary = ["<pad>", "<unk>", "<bos>"];
    let config = format!(
        r#"{{"vocab":{},"n_layer":0,"n_head":1,"n_embd":1,"context":{CONTEXT},"dropout":0.0}}"#,
        vocabulary.len()
    );
    let tensors: [(&str, Vec<usize>, Vec<f32>); 4] = [
        (
            "tok.weight",
            vec![vocabulary.len(), 1],
            vec![0.0, -8.0, 0.0],
        ),
        ("pos.weight", vec![CONTEXT, 1], vec![0.0; CONTEXT]),
        ("ln_f.weight", vec![1], vec![0.0]),
        ("ln_f.bias", vec![1], vec![1.0]),
    ];
    let mut header = serde_json::Map::new();
    header.insert(
        "__metadata__".into(),
        json!({
            "format": "chinese-ime-lm",
            "version": "1",
            "precision": "f32",
            "config": config,
            "vocab": serde_json::to_string(&vocabulary).unwrap(),
        }),
    );
    let mut data = Vec::new();
    for (name, shape, values) in tensors {
        let start = data.len();
        data.extend(values.iter().flat_map(|value| value.to_le_bytes()));
        header.insert(
            name.into(),
            json!({"dtype": "F32", "shape": shape, "data_offsets": [start, data.len()]}),
        );
    }
    let header = serde_json::to_vec(&Value::Object(header)).unwrap();
    let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&data);
    std::fs::write(path, bytes).unwrap();
}

fn update(handle: u64, revision: u64, preferences: &Preferences) -> Value {
    let snapshot = json!({ "format_version": 1, "revision": revision, "preferences": preferences })
        .to_string();
    read(unsafe { msime_client_update_preferences(handle, snapshot.as_ptr(), snapshot.len()) })
}

#[test]
fn translation_queries_use_latest_preferences_without_resetting_composition() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        candidate_translations: false,
        ..Preferences::default()
    };
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    let mut view = Value::Null;
    for byte in b"U4e2d" {
        view = read(msime_client_character(
            handle,
            *byte,
            byte.is_ascii_uppercase(),
        ))["value"]["view"]
            .clone();
    }
    assert!(!view["candidates"].as_array().unwrap().is_empty());
    assert_eq!(
        read(msime_client_translation_query(handle))["value"],
        Value::Null
    );
    preferences.candidate_translations = true;
    preferences.translation_target_language =
        msime_client_core::preferences::TranslationTargetLanguage::Fr;
    preferences.translation_secondary_language =
        Some(msime_client_core::preferences::TranslationTargetLanguage::Ja);
    preferences.custom_translation.enabled = true;
    preferences.custom_translation.endpoint = "https://translation.example.invalid".into();
    preferences.tencent_tmt.secret_id = "AKIDsynthetic".into();
    preferences.tencent_tmt.secret_key = "synthetic".into();
    let changed = update(handle, 1, &preferences);
    assert_eq!(changed["value"]["deferred"], true);
    assert_eq!(
        changed["value"]["view"]["editing_text"],
        view["editing_text"]
    );
    assert!(
        changed["value"]["view"]["generation"].as_u64().unwrap()
            > view["generation"].as_u64().unwrap()
    );
    let query = read(msime_client_translation_query(handle));
    assert_eq!(
        query["value"]["generation"],
        changed["value"]["view"]["generation"]
    );
    assert_eq!(query["value"]["target_language"], "fr");
    assert_eq!(query["value"]["target_languages"], json!(["fr", "ja"]));
    assert!(query["value"]["user_data"].is_null());
    assert_eq!(
        query["value"]["custom_translation"]["endpoint"],
        "https://translation.example.invalid"
    );
    assert!(!query["value"]["candidates"].as_array().unwrap().is_empty());
    assert!(query["value"]["tencent_tmt"].is_null());
    preferences.custom_translation.enabled = false;
    update(handle, 2, &preferences);
    let tencent_query = read(msime_client_translation_query(handle));
    assert!(
        tencent_query["value"]["generation"].as_u64().unwrap()
            > query["value"]["generation"].as_u64().unwrap()
    );
    assert_eq!(
        tencent_query["value"]["tencent_tmt"]["region"],
        "ap-guangzhou"
    );
    assert_eq!(
        tencent_query["value"]["tencent_tmt"]["secret_id"],
        "AKIDsynthetic"
    );
    preferences.tencent_tmt.enabled = false;
    update(handle, 3, &preferences);
    assert!(read(msime_client_translation_query(handle))["value"]["tencent_tmt"].is_null());
    preferences.tencent_tmt.enabled = true;
    preferences.tencent_tmt.secret_key.clear();
    update(handle, 4, &preferences);
    assert!(read(msime_client_translation_query(handle))["value"]["tencent_tmt"].is_null());
    preferences.candidate_translations = false;
    update(handle, 5, &preferences);
    assert_eq!(
        read(msime_client_translation_query(handle))["value"],
        Value::Null
    );

    // The offline gloss is a packaged dictionary lookup, so it has to be
    // reachable with every online provider off - which is the usual case,
    // and was why Windows could not offer the setting at all.
    preferences.candidate_english_gloss = true;
    preferences.translation_target_language =
        msime_client_core::preferences::TranslationTargetLanguage::En;
    update(handle, 6, &preferences);
    let gloss = read(msime_client_translation_query(handle));
    assert!(
        gloss["value"]["generation"].as_u64().unwrap()
            > tencent_query["value"]["generation"].as_u64().unwrap()
    );
    assert_eq!(gloss["value"]["english_gloss"], true);
    assert_eq!(gloss["value"]["target_language"], "en");
    // The dictionary paths ride along only because the lookup needs them.
    assert!(gloss["value"]["resources"].is_string());
    assert!(!gloss["value"]["candidates"].as_array().unwrap().is_empty());
    // Still no online provider: the gloss must not imply one.
    assert!(gloss["value"]["tencent_tmt"].is_null());
    assert!(gloss["value"]["custom_translation"].is_null());
    assert!(gloss["value"]["niutrans"].is_null());

    // The packaged gloss dictionary is English only, so another target
    // language is an online request or nothing - never a wrong-language
    // gloss.
    preferences.translation_target_language =
        msime_client_core::preferences::TranslationTargetLanguage::Ja;
    update(handle, 7, &preferences);
    assert_eq!(
        read(msime_client_translation_query(handle))["value"],
        Value::Null
    );

    // An English secondary language still enables the packaged offline
    // dictionary while preserving the user's primary target.
    preferences.translation_secondary_language =
        Some(msime_client_core::preferences::TranslationTargetLanguage::En);
    preferences.candidate_english_gloss = true;
    update(handle, 8, &preferences);
    let secondary_gloss = read(msime_client_translation_query(handle));
    assert_eq!(
        secondary_gloss["value"]["target_languages"],
        json!(["ja", "en"])
    );
    assert_eq!(secondary_gloss["value"]["english_gloss"], true);

    // 离线英文释义关着、只开候选翻译时：用户目录照旧带上，存成功的英文翻译。整句候选不再逐词拆解，所以不带 gloss_breakdown；读音和离线释义都没开，资源目录也不带。
    preferences.candidate_english_gloss = false;
    preferences.translation_target_language =
        msime_client_core::preferences::TranslationTargetLanguage::En;
    preferences.candidate_translations = true;
    preferences.tencent_tmt.secret_key = "synthetic".into();
    update(handle, 9, &preferences);
    let online = read(msime_client_translation_query(handle));
    assert_eq!(online["value"]["english_gloss"], false);
    assert!(online["value"].get("gloss_breakdown").is_none());
    assert!(online["value"]["resources"].is_null());
    assert!(online["value"]["user_data"].is_string());

    // 目标语言里没有英文时资源目录同样不带。
    preferences.translation_target_language =
        msime_client_core::preferences::TranslationTargetLanguage::Ja;
    preferences.translation_secondary_language = None;
    update(handle, 10, &preferences);
    let japanese = read(msime_client_translation_query(handle));
    assert!(japanese["value"].get("gloss_breakdown").is_none());
    assert!(japanese["value"]["resources"].is_null());
    read(msime_client_destroy(handle));
}

fn offline_gloss_fixture(path: &std::path::Path, language: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    rusqlite::Connection::open(path)
        .unwrap()
        .execute_batch(&format!(
            "CREATE TABLE zh_glosses(chinese TEXT PRIMARY KEY, gloss TEXT NOT NULL, source TEXT NOT NULL) WITHOUT ROWID;
             CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
             INSERT INTO meta VALUES('target_language', '{language}');
             INSERT INTO zh_glosses VALUES('你好', 'bonjour, salut', 'hello');
             PRAGMA user_version = 1;"
        ))
        .unwrap();
}

/// A Traditional candidate - a Korean Hanja, or a candidate under Traditional output - is glossed under its Simplified characters when its own spelling finds nothing, and the reply names it as shown.
#[test]
fn candidate_gloss_request_retries_traditional_candidates_under_their_simplified_form() {
    let directory = tempfile::tempdir().unwrap();
    let resources = directory.path().join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    let resources = resources.to_str().unwrap().as_bytes().to_vec();
    let database = directory.path().join("offline-glosses/zh-fr.db");
    offline_gloss_fixture(&database, "fr");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("INSERT INTO zh_glosses VALUES('韩', 'Corée', 'korea'); INSERT INTO zh_glosses VALUES('漢', 'Han (as stored)', 'han');")
        .unwrap();
    let request = serde_json::to_vec(&json!({
        "generation": 3,
        "target_language": "fr",
        "candidates": [
            {"text":"韓","source":0},
            {"text":"漢","source":0},
            {"text":"寒","source":0},
            {"text":"你好","source":0}
        ]
    }))
    .unwrap();
    let reply = read(unsafe {
        msime_client_candidate_gloss_request(
            request.as_ptr(),
            request.len(),
            resources.as_ptr(),
            resources.len(),
        )
    });
    assert_eq!(reply["ok"], true);
    // 韓 is found under 韩; 漢 has its own row, which wins over 汉; 寒 is the same in both scripts and has none.
    assert_eq!(
        reply["value"],
        json!({"generation":3,"translations":[
            {"text":"韓","translation":"Corée"},
            {"text":"漢","translation":"Han (as stored)"},
            {"text":"你好","translation":"bonjour, salut"}
        ]})
    );
}

/// A non-English offline dictionary is announced only when it is installed, and the query without one is the one hosts always received.
#[test]
fn translation_query_lists_installed_offline_gloss_languages() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        candidate_translations: false,
        candidate_english_gloss: true,
        translation_target_language: msime_client_core::preferences::TranslationTargetLanguage::Fr,
        ..Preferences::default()
    };
    preferences.tencent_tmt.enabled = false;
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    for byte in b"U4e2d" {
        read(msime_client_character(
            handle,
            *byte,
            byte.is_ascii_uppercase(),
        ));
    }
    // Nothing installed: French with translation off has no gloss source, as before.
    assert_eq!(
        read(msime_client_translation_query(handle))["value"],
        Value::Null
    );
    preferences.candidate_translations = true;
    update(handle, 1, &preferences);
    let plain = read(msime_client_translation_query(handle));
    assert!(plain["value"].get("offline_gloss_languages").is_none());
    assert!(plain["value"]["resources"].is_null());

    offline_gloss_fixture(&dir.path().join("offline-glosses/zh-fr.db"), "fr");
    offline_gloss_fixture(&dir.path().join("offline-glosses/zh-ko.db"), "ko");
    preferences.translation_secondary_language =
        Some(msime_client_core::preferences::TranslationTargetLanguage::Ja);
    update(handle, 2, &preferences);
    let installed = read(msime_client_translation_query(handle));
    // Korean is installed but not a target; Japanese is a target but not installed.
    assert_eq!(installed["value"]["offline_gloss_languages"], json!(["fr"]));
    assert!(installed["value"]["resources"].is_string());
    assert_eq!(installed["value"]["english_gloss"], false);

    // The offline switch alone reaches it, with online translation off and no provider implied.
    preferences.candidate_translations = false;
    update(handle, 3, &preferences);
    let offline = read(msime_client_translation_query(handle));
    assert_eq!(offline["value"]["offline_gloss_languages"], json!(["fr"]));
    assert_eq!(offline["value"]["translation_account"], false);
    assert!(offline["value"]["tencent_tmt"].is_null());

    // Both switches off: nothing, installed or not.
    preferences.candidate_english_gloss = false;
    update(handle, 4, &preferences);
    assert_eq!(
        read(msime_client_translation_query(handle))["value"],
        Value::Null
    );
    read(msime_client_destroy(handle));
}

#[test]
fn translation_query_carries_the_pronunciation_switch_only_when_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        candidate_english_gloss: true,
        ..Preferences::default()
    };
    preferences.tencent_tmt.enabled = false;
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    for byte in b"U4e2d" {
        read(msime_client_character(
            handle,
            *byte,
            byte.is_ascii_uppercase(),
        ));
    }
    let off = read(msime_client_translation_query(handle));
    assert!(off["value"].is_object());
    assert!(off["value"].get("candidate_pronunciation").is_none());

    preferences.candidate_pronunciation = true;
    update(handle, 1, &preferences);
    let on = read(msime_client_translation_query(handle));
    assert_eq!(on["value"]["candidate_pronunciation"], true);

    // 只开在线翻译时也带资源目录，宿主才能给在线翻出来的英文释义标音标。
    preferences.candidate_english_gloss = false;
    preferences.candidate_translations = true;
    update(handle, 2, &preferences);
    let online = read(msime_client_translation_query(handle));
    assert_eq!(online["value"]["english_gloss"], false);
    assert!(online["value"]["resources"].is_string());

    // Pronunciation annotates a gloss; with every gloss source off there is nothing to annotate.
    preferences.candidate_english_gloss = false;
    preferences.candidate_translations = false;
    update(handle, 3, &preferences);
    assert_eq!(
        read(msime_client_translation_query(handle))["value"],
        Value::Null
    );
    read(msime_client_destroy(handle));
}

#[test]
fn candidate_gloss_request_reads_the_offline_dictionary_for_its_target_language() {
    let directory = tempfile::tempdir().unwrap();
    let resources = directory.path().join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    let user = directory.path().join("user");
    std::fs::create_dir_all(&user).unwrap();
    let resources = resources.to_str().unwrap().as_bytes().to_vec();
    let call = |request: Value| {
        let request = serde_json::to_vec(&request).unwrap();
        read(unsafe {
            msime_client_candidate_gloss_request(
                request.as_ptr(),
                request.len(),
                resources.as_ptr(),
                resources.len(),
            )
        })
    };
    let candidates = json!([
        {"text":"你好","source":0},
        {"text":"Hello","source":4},
        {"text":"再见","source":0}
    ]);
    // Not installed is not an error: the host keeps whatever the online path brings.
    let missing = call(json!({"generation":7,"target_language":"fr","candidates":candidates}));
    assert_eq!(missing["ok"], true);
    assert_eq!(missing["value"], json!({"generation":7,"translations":[]}));

    offline_gloss_fixture(&directory.path().join("offline-glosses/zh-fr.db"), "fr");
    // The user directory holds English only, so a French request never reads it even when given.
    std::fs::write(user.join("custom_translations.txt"), "你好\thand written\n").unwrap();
    let french = call(json!({
        "generation":8,
        "target_language":"fr",
        "user_data":user.to_str().unwrap(),
        "candidates":candidates
    }));
    assert_eq!(french["ok"], true);
    assert_eq!(
        french["value"],
        json!({"generation":8,"translations":[{"text":"你好","translation":"bonjour, salut"}]})
    );

    // A file under the wrong name is refused rather than shown as another language.
    std::fs::copy(
        directory.path().join("offline-glosses/zh-fr.db"),
        directory.path().join("offline-glosses/zh-ja.db"),
    )
    .unwrap();
    let renamed = call(json!({"generation":9,"target_language":"ja","candidates":candidates}));
    assert_eq!(renamed["ok"], false);
    assert_eq!(renamed["error"], "candidate gloss dictionary unavailable");

    for language in ["xx", "EN", "../fr", ""] {
        let invalid =
            call(json!({"generation":10,"target_language":language,"candidates":candidates}));
        assert_eq!(
            invalid["error"], "invalid candidate gloss request",
            "{language}"
        );
    }
    // English, spelled out or implied, is still the packaged dictionary, which this directory lacks.
    for request in [
        json!({"generation":11,"target_language":"en","candidates":candidates}),
        json!({"generation":11,"candidates":candidates}),
    ] {
        assert_eq!(
            call(request)["error"],
            "candidate gloss dictionary unavailable"
        );
    }
}

/// Android 不再随包带离线释义：资源目录旁没有时，用状态目录下已下载的 `offline-glosses` 资源包。翻译查询列出这些语言并带上 `state_root`，宿主把它交回释义请求；随包的那份仍然优先，那时查询和以前一样不带 `state_root`。
/// Linux keeps the Tencent secret in the provider's own file, so the query's credential fields cannot say which service the user picked. The explicit choice has to survive to the socket even when that service is unusable, or the provider falls back to Tencent.
#[cfg(unix)]
#[test]
fn translation_query_names_the_selected_service_through_the_provider_socket() {
    use std::io::{BufRead, Write};
    let dir = tempfile::tempdir().unwrap();
    // A stored document that never chose the account, whatever the platform's fresh-install default.
    let mut preferences = Preferences {
        candidate_translations: true,
        translation_account: false,
        ..Preferences::default()
    };
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    for byte in b"U4e2d" {
        read(msime_client_character(
            handle,
            *byte,
            byte.is_ascii_uppercase(),
        ));
    }
    let sockets = tempfile::tempdir().unwrap();
    // The provider connection refuses a socket directory other users can reach.
    std::fs::set_permissions(
        sockets.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let socket = sockets.path().join("translation.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    // Send the query as the Linux hosts do and return the document the provider received, or None when nothing connected.
    let forward = |query: &Value| -> Option<Value> {
        let mut transport = query.clone();
        transport["candidates"] = Value::Array(
            query["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|candidate| candidate["text"].clone())
                .collect(),
        );
        let request = transport.to_string();
        let path = socket.to_str().unwrap();
        std::thread::scope(|scope| {
            let call = scope.spawn(|| {
                read(unsafe {
                    msime_client_translation_provider_request(
                        request.as_ptr(),
                        request.len(),
                        path.as_ptr(),
                        path.len(),
                    )
                })
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            // A call that connected cannot return before it is answered, so a finished call with nothing pending never connected.
            let received = loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let mut line = String::new();
                        std::io::BufReader::new(stream.try_clone().unwrap())
                            .read_line(&mut line)
                            .unwrap();
                        stream.write_all(b"{\"translations\":[]}\n").unwrap();
                        break Some(serde_json::from_str::<Value>(&line).unwrap());
                    }
                    Err(_) if call.is_finished() => break None,
                    Err(_) => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "provider request hung"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
            };
            assert_eq!(call.join().unwrap()["value"], json!({"translations": []}));
            received
        })
    };

    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["provider"], "tencent");
    // Without the stored choice the MSIME account endpoint is not used.
    assert_eq!(query["translation_account"], false);
    assert_eq!(forward(&query).unwrap()["query"]["provider"], "tencent");

    preferences.tencent_tmt.enabled = false;
    update(handle, 1, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["provider"], "none");
    assert!(
        forward(&query).is_none(),
        "a switched-off query reached the provider"
    );

    preferences.niutrans.enabled = true;
    update(handle, 2, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["provider"], "niutrans");
    assert!(query["niutrans"].is_null() && query["tencent_tmt"].is_null());
    let received = forward(&query).unwrap();
    assert_eq!(received["query"]["provider"], "niutrans");
    assert!(received["query"].get("niutrans").is_none());

    preferences.niutrans.enabled = false;
    preferences.custom_translation.enabled = true;
    update(handle, 3, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["provider"], "custom");
    assert!(query["custom_translation"].is_null());
    assert_eq!(forward(&query).unwrap()["query"]["provider"], "custom");
    assert_eq!(query["translation_account"], false);

    // Choosing the account switches every other service off, and the Linux provider socket receives
    // the explicit account flag so it can create or reuse the anonymous translation identity.
    preferences.custom_translation.enabled = false;
    preferences.translation_account = true;
    update(handle, 4, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["translation_account"], true);
    assert_eq!(query["provider"], "account");
    let received = forward(&query).expect("an account query should reach the provider");
    assert_eq!(received["query"]["provider"], "account");
    assert_eq!(received["query"]["translation_account"], true);

    // Tencent's default `enabled: true` without usable secrets is not a user choice and does not displace the account; usable secrets do.
    preferences.tencent_tmt.enabled = true;
    update(handle, 5, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["translation_account"], true);
    preferences.tencent_tmt.secret_id = "id".into();
    preferences.tencent_tmt.secret_key = "key".into();
    update(handle, 6, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["translation_account"], false);
    preferences.tencent_tmt.enabled = false;

    // The user's own service wins over the account.
    preferences.niutrans.enabled = true;
    update(handle, 7, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["translation_account"], false);

    // The offline English gloss keeps the query alive with candidate translations off, and must not carry the account.
    preferences.niutrans.enabled = false;
    preferences.candidate_translations = false;
    preferences.candidate_english_gloss = true;
    preferences.translation_target_language =
        msime_client_core::preferences::TranslationTargetLanguage::En;
    update(handle, 8, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["english_gloss"], true);
    assert_eq!(query["translation_account"], false);
    read(msime_client_destroy(handle));
}

/// 「AI 辅助」翻译是用户显式选择的服务（`translation_ai`）：凭据来自「AI 与云联想」页的
/// ai_assistant 配置，联想开关不替它授权；配置不可用时 provider 仍是 ai、凭据一栏留空，
/// 让宿主和设置页能照实提示，而不是静默回退成离线或别的服务。
#[test]
fn translation_query_carries_the_selected_ai_assistant_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        candidate_translations: true,
        ..Preferences::default()
    };
    preferences.tencent_tmt.enabled = false;
    preferences.ai_assistant.provider = "openai".into();
    preferences.ai_assistant.endpoint = "https://ai.example.invalid/v1/chat/completions".into();
    preferences.ai_assistant.model = "fixture-model".into();
    preferences.ai_assistant.tokens.insert(
        "https://ai.example.invalid:443".into(),
        "synthetic-token".into(),
    );
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    for byte in b"U4e2d" {
        read(msime_client_character(
            handle,
            *byte,
            byte.is_ascii_uppercase(),
        ));
    }

    // AI 辅助开着但没被选为翻译服务：查询仍然是不联网的 none，不带上 AI 凭据。
    preferences.ai_assistant.enabled = true;
    update(handle, 1, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["provider"], "none");
    assert!(query["ai_assistant"].is_null());

    // 选中 AI 辅助：联想开关关着也能翻译，查询带上 AI 凭据并置 enabled 供宿主发送。
    preferences.translation_ai = true;
    preferences.ai_assistant.enabled = false;
    update(handle, 2, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["provider"], "ai");
    assert_eq!(
        query["ai_assistant"]["endpoint"],
        "https://ai.example.invalid/v1/chat/completions"
    );
    assert_eq!(query["ai_assistant"]["model"], "fixture-model");
    assert_eq!(query["ai_assistant"]["enabled"], true);
    assert_eq!(
        query["ai_assistant"]["tokens"]["https://ai.example.invalid:443"],
        "synthetic-token"
    );

    // 选了 AI 但配置不可用：provider 仍是 ai、凭据一栏留空，而不是静默不翻译。
    preferences.ai_assistant.tokens.clear();
    preferences.ai_assistant.model.clear();
    update(handle, 3, &preferences);
    let query = read(msime_client_translation_query(handle))["value"].clone();
    assert_eq!(query["provider"], "ai");
    assert!(query["ai_assistant"].is_null());
    read(msime_client_destroy(handle));
}

#[test]
fn translation_preference_changes_remove_gloss_and_reject_inflight_answers() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        candidate_translations: true,
        ..Preferences::default()
    };
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    let mut view = Value::Null;
    for byte in b"U4e2d" {
        view = read(msime_client_character(
            handle,
            *byte,
            byte.is_ascii_uppercase(),
        ))["value"]["view"]
            .clone();
    }
    let query = read(msime_client_translation_query(handle))["value"].clone();
    let generation = query["generation"].as_u64().unwrap();
    let candidate = view["candidates"][0]["text"].as_str().unwrap();
    let payload = serde_json::to_vec(&json!([{
        "text": candidate,
        "translation": "合成释义"
    }]))
    .unwrap();
    let apply = |generation| {
        read(unsafe {
            msime_client_apply_translations(handle, generation, payload.as_ptr(), payload.len())
        })
    };
    assert_eq!(apply(generation)["value"]["applied"], true);

    let disabled = Preferences {
        candidate_translations: false,
        ..preferences.clone()
    };
    let disabled_view = update(handle, 1, &disabled);
    assert_eq!(disabled_view["ok"], true);
    assert_ne!(
        disabled_view["value"]["view"]["candidates"][0]["translation"],
        "合成释义"
    );
    assert!(read(msime_client_translation_query(handle))["value"].is_null());
    assert_eq!(apply(generation)["value"]["applied"], false);

    assert_eq!(update(handle, 2, &preferences)["ok"], true);
    assert_eq!(apply(generation)["value"]["applied"], false);
    let current_generation = read(msime_client_translation_query(handle))["value"]["generation"]
        .as_u64()
        .unwrap();
    assert!(current_generation > generation);
    assert_eq!(apply(current_generation)["value"]["applied"], true);
    read(msime_client_destroy(handle));
}

#[test]
fn translation_target_change_rejects_old_language_gloss() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        candidate_translations: true,
        ..Preferences::default()
    };
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    let mut view = Value::Null;
    for byte in b"U4e2d" {
        view = read(msime_client_character(
            handle,
            *byte,
            byte.is_ascii_uppercase(),
        ))["value"]["view"]
            .clone();
    }
    let old_generation = read(msime_client_translation_query(handle))["value"]["generation"]
        .as_u64()
        .unwrap();
    let candidate = view["candidates"][0]["text"].as_str().unwrap();
    let payload = serde_json::to_vec(&json!([{
        "text": candidate,
        "translation": "合成旧语言释义"
    }]))
    .unwrap();

    preferences.translation_target_language =
        msime_client_core::preferences::TranslationTargetLanguage::Ja;
    let updated = update(handle, 1, &preferences);
    assert_eq!(updated["ok"], true);
    assert_eq!(
        updated["value"]["view"]["editing_text"],
        view["editing_text"]
    );
    let late = read(unsafe {
        msime_client_apply_translations(handle, old_generation, payload.as_ptr(), payload.len())
    });
    assert_eq!(late["value"]["applied"], false);
    read(msime_client_destroy(handle));
}

#[test]
fn translation_results_reject_control_characters_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    let mut view = Value::Null;
    for byte in b"U4e2d" {
        view = read(msime_client_character(
            handle,
            *byte,
            byte.is_ascii_uppercase(),
        ))["value"]["view"]
            .clone();
    }
    let generation = view["generation"].as_u64().unwrap();
    let candidate = view["candidates"][0]["text"].as_str().unwrap().to_owned();
    let apply = |values: Value| {
        let encoded = serde_json::to_vec(&values).unwrap();
        read(unsafe {
            msime_client_apply_translations(handle, generation, encoded.as_ptr(), encoded.len())
        })
    };

    let applied = apply(json!([{"text":candidate,"translation":"合成释义"}]));
    assert_eq!(applied["ok"], true);
    assert_eq!(applied["value"]["applied"], true);
    assert_eq!(
        applied["value"]["view"]["candidates"][0]["translation"],
        "合成释义"
    );
    let before = applied["value"]["view"].clone();

    for codepoint in (0..=0x1f).chain(0x7f..=0x9f) {
        let control = char::from_u32(codepoint).unwrap();
        for invalid in [
            json!([{"text":format!("{candidate}{control}"),"translation":"safe"}]),
            json!([{"text":candidate,"translation":format!("before{control}after")}]),
        ] {
            let rejected = apply(invalid);
            assert_eq!(rejected["ok"], false);
            assert_eq!(rejected["error"], "translation entries exceed limits");
        }
    }
    assert_eq!(read(msime_client_view(handle))["value"], before);
    read(msime_client_destroy(handle));
}

#[test]
fn translation_queries_follow_active_japanese_mode() {
    for temporary in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        if temporary {
            // The host disables the temporary Japanese shortcut when its
            // model resource is absent.  This test supplies a bounded
            // placeholder so it exercises the shortcut's generated kana
            // path without depending on a packaged model.
            std::fs::create_dir_all(dir.path().join("resources")).unwrap();
            std::fs::write(
                dir.path().join("resources/msime-japanese.dat"),
                b"synthetic",
            )
            .unwrap();
        }
        let preferences = Preferences {
            scheme: if temporary {
                InputScheme::Quanpin
            } else {
                InputScheme::Japanese
            },
            candidate_translations: true,
            ..chinese_preferences()
        };
        let handle = test_host_preferences(dir.path(), preferences.clone());
        read(msime_client_focus(handle, true));
        if temporary {
            read(msime_client_character(handle, b'R', true));
        }
        let view = read(msime_client_character(handle, b'a', false))["value"]["view"].clone();
        assert!(!view["candidates"].as_array().unwrap().is_empty());
        assert_eq!(view["local_mode"] == "temporary_japanese", temporary);
        assert_eq!(
            read(msime_client_translation_query(handle))["value"],
            Value::Null
        );
        read(msime_client_command(handle, 3));
        update(
            handle,
            1,
            &Preferences {
                scheme: InputScheme::Quanpin,
                ..preferences
            },
        );
        for byte in b"U4e2d" {
            read(msime_client_character(
                handle,
                *byte,
                byte.is_ascii_uppercase(),
            ));
        }
        let query = read(msime_client_translation_query(handle));
        assert!(query["value"]["candidates"]
            .as_array()
            .is_some_and(|items| !items.is_empty()));
        read(msime_client_destroy(handle));
    }
}

#[test]
fn shuangpin_preedit_mode_is_applied_after_composition() {
    let dir = tempfile::tempdir().unwrap();
    let raw = Preferences {
        scheme: InputScheme::Shuangpin,
        ..chinese_preferences()
    };
    let handle = test_host_preferences(dir.path(), raw.clone());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'h', false));
    let before = read(msime_client_character(handle, b'k', false))["value"]["view"].clone();
    let expanded = Preferences {
        shuangpin_preedit_uses_raw: false,
        single_character_only: false,
        ..raw.clone()
    };
    let queued = update(handle, 1, &expanded);
    assert_eq!(queued["value"]["deferred"], true);
    assert_eq!(queued["value"]["view"], before);
    read(msime_client_command(handle, 3));
    assert_eq!(update(handle, 1, &expanded)["value"]["deferred"], false);
    read(msime_client_character(handle, b'h', false));
    let after = read(msime_client_character(handle, b'k', false))["value"]["view"].clone();
    assert_eq!(after["editing_text"], before["editing_text"]);
    assert_ne!(after["preedit"], before["preedit"]);
    read(msime_client_command(handle, 3));
    update(handle, 2, &raw);
    read(msime_client_character(handle, b'h', false));
    let restored = read(msime_client_character(handle, b'k', false));
    assert_eq!(restored["value"]["view"]["preedit"], before["preedit"]);
    msime_client_destroy(handle);
}

#[test]
fn smart_abc_profile_commits_and_defers_switching_until_composition_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let smart_abc = Preferences {
        scheme: InputScheme::Shuangpin,
        shuangpin_profile: ShuangpinProfile::SmartAbc,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(dir.path(), smart_abc.clone());
    read(msime_client_focus(handle, true));
    assert_eq!(
        read(msime_client_view(handle))["value"]["shuangpin_profile"],
        "smart_abc"
    );
    SESSIONS.with(|sessions| assert_eq!(sessions.borrow()[&handle].options.shuangpin_profile, 5));
    for character in b"nihk" {
        assert_eq!(
            read(msime_client_character(handle, *character, false))["ok"],
            true
        );
    }
    let xiaohe = Preferences {
        shuangpin_profile: ShuangpinProfile::Xiaohe,
        ..smart_abc.clone()
    };
    assert_eq!(update(handle, 1, &xiaohe)["value"]["deferred"], true);
    assert_eq!(
        read(msime_client_command(handle, 1))["value"]["commit"],
        "本地"
    );
    assert_eq!(update(handle, 1, &xiaohe)["value"]["deferred"], false);
    assert_eq!(
        read(msime_client_view(handle))["value"]["shuangpin_profile"],
        "xiaohe"
    );
    assert_eq!(update(handle, 2, &smart_abc)["value"]["deferred"], false);
    for character in b"agfu" {
        read(msime_client_character(handle, *character, false));
    }
    assert_eq!(
        read(msime_client_command(handle, 1))["value"]["commit"],
        "政府"
    );
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

#[test]
fn shuangpin_profile_creation_and_deferred_replacement_use_real_engine() {
    let dir = tempfile::tempdir().unwrap();
    let microsoft = Preferences {
        scheme: InputScheme::Shuangpin,
        shuangpin_profile: ShuangpinProfile::Microsoft,
        ..chinese_preferences()
    };
    let handle = test_host_preferences(dir.path(), microsoft.clone());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'b', false));
    let first = read(msime_client_character(handle, b';', false));
    assert_eq!(first["value"]["view"]["editing_text"], "b;");
    let xiaohe = Preferences {
        shuangpin_profile: ShuangpinProfile::Xiaohe,
        ..microsoft.clone()
    };
    let before = read(msime_client_view(handle))["value"].clone();
    let queued = update(handle, 1, &xiaohe);
    assert_eq!(before["microsoft_shuangpin"], true);
    assert_eq!(before["shuangpin_profile"], "microsoft");
    assert_eq!(queued["value"]["deferred"], true);
    assert_eq!(queued["value"]["view"], before);
    // The old composition completes under Microsoft before replacing Engine.
    assert_eq!(
        read(msime_client_command(handle, 2))["value"]["commit"],
        "b;"
    );
    assert_eq!(update(handle, 1, &xiaohe)["value"]["deferred"], false);
    assert_eq!(
        read(msime_client_view(handle))["value"]["shuangpin_profile"],
        "xiaohe"
    );
    assert_eq!(
        read(msime_client_view(handle))["value"]["microsoft_shuangpin"],
        false
    );
    read(msime_client_character(handle, b'b', false));
    let replaced = read(msime_client_character(handle, b';', false));
    assert_ne!(replaced["value"]["view"]["editing_text"], "b;");
    read(msime_client_command(handle, 3));
    assert_eq!(update(handle, 2, &microsoft)["value"]["deferred"], false);
    read(msime_client_character(handle, b'b', false));
    assert_eq!(
        read(msime_client_character(handle, b';', false))["value"]["view"]["editing_text"],
        "b;"
    );
    read(msime_client_destroy(handle));
}
/// A lock to Chinese carries the punctuation switch with it, as the Windows host turns the switch on whenever punctuation is locked; unlocking hands the switch back.
#[test]
fn chinese_punctuation_lock_overrides_a_switched_off_punctuation_mode() {
    let comma =
        |handle| read(msime_client_punctuation_with_context(handle, b',', 0))["value"].clone();
    let dir = tempfile::tempdir().unwrap();
    let locked = test_host_preferences(
        dir.path(),
        Preferences {
            chinese_punctuation: false,
            punctuation_lock: msime_client_core::preferences::PunctuationLock::Chinese,
            ..chinese_preferences()
        },
    );
    read(msime_client_focus(locked, true));
    assert_eq!(comma(locked)["commit"], "，");
    read(msime_client_set_chinese_punctuation(locked, false));
    assert_eq!(comma(locked)["commit"], "，");
    read(msime_client_set_punctuation_lock(locked, 0));
    assert_ne!(comma(locked)["commit"], "，");
    read(msime_client_set_punctuation_lock(locked, 1));
    assert_eq!(comma(locked)["commit"], "，");
    read(msime_client_destroy(locked));

    let dir = tempfile::tempdir().unwrap();
    let follow = test_host_preferences(dir.path(), chinese_preferences());
    read(msime_client_focus(follow, true));
    read(msime_client_set_chinese_punctuation(follow, false));
    assert_ne!(comma(follow)["commit"], "，");
    read(msime_client_destroy(follow));
}

/// Windows keeps punctuation Chinese in English mode while it is locked to Chinese (`ResolvePunctuationOpen`), so a host that hands English-mode punctuation over under the lock must get the Chinese mark back.
#[test]
fn chinese_punctuation_lock_holds_in_english_mode() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            punctuation_lock: msime_client_core::preferences::PunctuationLock::Chinese,
            ..chinese_preferences()
        },
    );
    read(msime_client_focus(handle, true));
    read(msime_client_set_english_mode(handle, true));
    assert_eq!(
        read(msime_client_punctuation_with_context(handle, b',', 0))["value"]["commit"],
        "，"
    );
    read(msime_client_destroy(handle));
}

/// 「大写锁定时使用英文标点」（#6370）：宿主报告大写锁定后，三个标点入口在没有组字时都把键留给宿主按 ASCII 输出；组字中和固定中文标点时照旧，开关关着或大写锁定关掉时也照旧。
#[test]
fn caps_lock_sends_idle_punctuation_to_ascii_when_the_switch_is_on() {
    let left_to_host = |value: Value| {
        assert_eq!(value["ok"], true);
        assert_eq!(value["value"]["handled"], false);
        assert!(value["value"]["commit"].is_null());
    };
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            caps_lock_ascii_punctuation: true,
            ..chinese_preferences()
        },
    );
    read(msime_client_focus(handle, true));
    assert_eq!(
        read(msime_client_character(handle, b',', false))["value"]["commit"],
        "，"
    );

    assert_eq!(
        read(msime_client_set_caps_lock(handle, true))["value"],
        true
    );
    left_to_host(read(msime_client_character(handle, b',', false)));
    left_to_host(read(msime_client_character(handle, b'?', true)));
    left_to_host(read(msime_client_punctuation(handle, b';')));
    left_to_host(read(msime_client_punctuation_with_context(
        handle,
        b'.',
        u32::from('中'),
    )));

    // 组字中 Engine 照旧决定：候选连同中文标点一起上屏。
    read(msime_client_character(handle, b'n', false));
    read(msime_client_character(handle, b'i', false));
    let composed = read(msime_client_character(handle, b',', false));
    assert!(composed["value"]["commit"]
        .as_str()
        .is_some_and(|value| value.ends_with('，')));

    // 固定中文标点优先于大写锁定。
    read(msime_client_set_punctuation_lock(handle, 1));
    assert_eq!(
        read(msime_client_character(handle, b',', false))["value"]["commit"],
        "，"
    );
    read(msime_client_set_punctuation_lock(handle, 0));

    read(msime_client_set_caps_lock(handle, false));
    assert_eq!(
        read(msime_client_character(handle, b',', false))["value"]["commit"],
        "，"
    );
    read(msime_client_destroy(handle));

    // 开关关着（默认）时，大写锁定不改变标点。
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_set_caps_lock(handle, true));
    assert_eq!(
        read(msime_client_character(handle, b',', false))["value"]["commit"],
        "，"
    );
    assert_eq!(
        read(msime_client_punctuation_with_context(handle, b',', 0))["value"]["commit"],
        "，"
    );
    read(msime_client_destroy(handle));
}

#[test]
fn explicit_punctuation_finishes_unicode_and_rejects_invalid_bytes() {
    for enabled in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let handle = test_host(dir.path());
        assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
        read(msime_client_set_chinese_punctuation(handle, enabled));
        read(msime_client_character(handle, b'U', true));
        for byte in b"4e2d" {
            read(msime_client_character(handle, *byte, false));
        }
        let before = read(msime_client_view(handle));
        for invalid in [b'a', b' ', 0, 128, 255] {
            assert_eq!(read(msime_client_punctuation(handle, invalid))["ok"], false);
            assert_eq!(read(msime_client_view(handle)), before);
        }
        assert_eq!(
            std::thread::spawn(move || read(msime_client_punctuation(handle, b','))["ok"].clone())
                .join()
                .unwrap(),
            false
        );
        assert_eq!(read(msime_client_view(handle)), before);
        let result = read(msime_client_punctuation(handle, b','));
        assert_eq!(result["ok"], true);
        assert_eq!(result["value"]["handled"], true);
        assert_eq!(
            result["value"]["commit"],
            if enabled { "中，" } else { "中," }
        );
        assert_eq!(result["value"]["view"]["editing_text"], "");
        read(msime_client_destroy(handle));
        assert_eq!(read(msime_client_punctuation(handle, b','))["ok"], false);
    }
}

// 智能标点默认开，走的是宿主真正用的入口：什么都不改的偏好下，数字后的 , . : 交给宿主按 ASCII 上屏（`0.10.1` 不会变成 `0。10。1`），中文后照旧是中文标点；ASCII 句点刚上屏后两秒内再按一次句点，换回「。」，句末是数字（「共计 100。」）靠的就是这条退路。
#[test]
fn default_preferences_keep_ascii_after_a_digit_and_repeat_turns_it_chinese() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(dir.path(), chinese_preferences());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);

    for punctuation in *b",.:" {
        let after_digit = read(msime_client_punctuation_with_context(
            handle,
            punctuation,
            u32::from('1'),
        ));
        assert_eq!(after_digit["ok"], true);
        assert_eq!(
            after_digit["value"]["handled"], false,
            "{} after a digit should be left to the host as ASCII",
            punctuation as char
        );
    }
    for (punctuation, chinese) in [(b',', "，"), (b'.', "。"), (b':', "：")] {
        let after_chinese = read(msime_client_punctuation_with_context(
            handle,
            punctuation,
            u32::from('计'),
        ));
        assert_eq!(after_chinese["value"]["commit"], chinese);
    }

    // 「100」后宿主上屏了 ASCII 句点，再按一次句点：换成中文句号。
    let arm = json!({
        "ascii": 46, "commit": ".", "timestamp_ms": 10_000,
        "editor_generation": 1, "auto_closed_pair": false
    })
    .to_string();
    // SAFETY: the buffer outlives the call.
    let armed =
        read(unsafe { msime_client_smart_punctuation_arm(handle, arm.as_ptr(), arm.len()) });
    assert_eq!(armed["value"]["repeat"]["committed"], ".");
    let decide = json!({
        "character": 46, "preceding": ".", "timestamp_ms": 10_800,
        "editor_generation": 1, "repeat": armed["value"]["repeat"], "space": null
    })
    .to_string();
    // SAFETY: the buffer outlives the call.
    let decided = read(unsafe {
        msime_client_smart_punctuation_decide(handle, decide.as_ptr(), decide.len())
    });
    assert_eq!(decided["value"]["replace_with"], "。");
    read(msime_client_destroy(handle));
}

// 默认改变之前的偏好文档经宿主的读取入口读出来：macOS、Windows 上存着的旧默认 `false` 一次性改成开，别的平台原样；空格转换不动。
#[test]
fn a_legacy_preferences_document_loads_with_smart_punctuation_settled() {
    let directory = tempfile::tempdir().unwrap();
    let mut legacy =
        serde_json::to_value(msime_client_core::preferences::PreferencesSnapshot::default())
            .unwrap();
    let preferences = legacy["preferences"].as_object_mut().unwrap();
    for key in [
        "smart_punctuation",
        "smart_punctuation_repeat",
        "smart_punctuation_direct_digit",
        "smart_punctuation_direct_letter",
        "smart_punctuation_space_convert",
    ] {
        preferences.insert(key.into(), false.into());
    }
    preferences.remove("smart_punctuation_migrated").unwrap();
    std::fs::write(
        directory.path().join("preferences.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    let path = directory.path().to_str().unwrap();
    let loaded = read(unsafe { msime_client_load_preferences(path.as_ptr(), path.len()) });
    assert_eq!(loaded["ok"], true, "{loaded}");
    let flipped = cfg!(any(windows, target_os = "macos"));
    let preferences = &loaded["value"]["preferences"];
    for key in [
        "smart_punctuation",
        "smart_punctuation_repeat",
        "smart_punctuation_direct_digit",
        "smart_punctuation_direct_letter",
    ] {
        assert_eq!(preferences[key], flipped, "{key}");
    }
    assert_eq!(preferences["smart_punctuation_space_convert"], false);
    assert_eq!(preferences["smart_punctuation_migrated"], true);
}

#[test]
fn contextual_punctuation_respects_editor_context_preferences_and_composition() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            // 写明而不靠默认：这条测的是开关打开时的路线，默认值另有 `default_preferences_keep_ascii_after_a_digit_and_repeat_turns_it_chinese` 守着。
            smart_punctuation: true,
            smart_punctuation_direct_digit: true,
            smart_punctuation_direct_letter: true,
            ..chinese_preferences()
        },
    );
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);

    for preceding in [u32::from('0'), u32::from('a'), u32::from('Z')] {
        for punctuation in *b",.:" {
            let result = read(msime_client_punctuation_with_context(
                handle,
                punctuation,
                preceding,
            ));
            assert_eq!(result["ok"], true);
            assert_eq!(result["value"]["handled"], false);
        }
    }
    for preceding in [0, u32::from('中'), u32::from(' ')] {
        let result = read(msime_client_punctuation_with_context(
            handle, b',', preceding,
        ));
        assert_eq!(result["value"]["commit"], "，");
    }
    assert_eq!(
        read(msime_client_punctuation_with_context(
            handle,
            b'?',
            u32::from('a')
        ))["value"]["commit"],
        "？"
    );

    // Turning the two halves off is how a user asks for none of this, and then a comma after a digit
    // or a letter stays Chinese. They default on where the parent does - the parent's own
    // description promises exactly this conversion - so the document says so rather than relying on
    // a default that now points the other way.
    let plain = test_host_preferences(
        dir.path(),
        Preferences {
            smart_punctuation: true,
            smart_punctuation_direct_digit: false,
            smart_punctuation_direct_letter: false,
            ..chinese_preferences()
        },
    );
    assert_eq!(read(msime_client_focus(plain, true))["ok"], true);
    for preceding in [u32::from('0'), u32::from('a')] {
        assert_eq!(
            read(msime_client_punctuation_with_context(
                plain, b',', preceding
            ))["value"]["commit"],
            "，"
        );
    }

    read(msime_client_set_punctuation_lock(handle, 1));
    assert_eq!(
        read(msime_client_punctuation_with_context(
            handle,
            b',',
            u32::from('a')
        ))["value"]["commit"],
        "，"
    );
    read(msime_client_set_punctuation_lock(handle, 2));
    assert_eq!(
        read(msime_client_punctuation_with_context(
            handle,
            b',',
            u32::from('中')
        ))["value"]["handled"],
        false
    );
    read(msime_client_set_punctuation_lock(handle, 0));

    read(msime_client_character(handle, b'n', false));
    read(msime_client_character(handle, b'i', false));
    let composed = read(msime_client_punctuation_with_context(
        handle,
        b',',
        u32::from('a'),
    ));
    assert!(composed["value"]["commit"]
        .as_str()
        .is_some_and(|value| value.ends_with('，')));

    let preferences = Preferences {
        smart_punctuation: false,
        ..Preferences::default()
    };
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], false);
    assert_eq!(
        read(msime_client_punctuation_with_context(
            handle,
            b'.',
            u32::from('7')
        ))["value"]["commit"],
        "。"
    );
    assert_eq!(
        read(msime_client_punctuation_with_context(
            handle,
            b'a',
            u32::from('7')
        ))["ok"],
        false
    );
    assert_eq!(
        read(msime_client_punctuation_with_context(handle, b',', 0xd800))["ok"],
        false
    );
    read(msime_client_destroy(handle));
}

#[test]
fn paired_book_title_auto_close_balance_is_narrow_and_owned() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    assert_eq!(
        read(msime_client_punctuation(handle, b'<'))["value"]["commit"],
        "《"
    );
    let before = read(msime_client_view(handle));
    for invalid in [b'(', b'>', b'a', b' ', 0, 128, 255] {
        assert_eq!(
            read(msime_client_balance_paired_punctuation_after_auto_close(
                handle, invalid
            ))["ok"],
            false
        );
        assert_eq!(read(msime_client_view(handle)), before);
    }
    let balanced = read(msime_client_balance_paired_punctuation_after_auto_close(
        handle, b'<',
    ));
    assert_eq!(balanced["ok"], true);
    assert_eq!(balanced["value"], before["value"]);
    assert_eq!(
        read(msime_client_punctuation(handle, b'<'))["value"]["commit"],
        "《"
    );
    read(msime_client_balance_paired_punctuation_after_auto_close(
        handle, b'<',
    ));
    read(msime_client_destroy(handle));
    assert_eq!(
        read(msime_client_balance_paired_punctuation_after_auto_close(
            handle, b'<',
        ))["ok"],
        false
    );
}

#[test]
fn unpaired_punctuation_keeps_quote_alternation_and_book_title_nesting() {
    // With paired completion off (or in an excluded host) nothing supplies the closing half, so the Engine's own alternation and nesting are the only way to type it - the reference's GetPunctuation does both regardless of the setting.
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    assert_eq!(
        read(msime_client_set_paired_punctuation(handle, false))["ok"],
        true
    );
    let marks = |keys: &[u8]| -> Vec<String> {
        keys.iter()
            .map(|&key| {
                read(msime_client_punctuation(handle, key))["value"]["commit"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    };
    assert_eq!(marks(b"\"\"\"\""), ["“", "”", "“", "”"]);
    assert_eq!(marks(b"''"), ["‘", "’"]);
    assert_eq!(marks(b"<<>>"), ["《", "〈", "〉", "》"]);
    // An unmatched closing mark leaves the depth at zero, so the next pair opens with 《 again.
    assert_eq!(marks(b"><>"), ["》", "《", "》"]);

    // The state belongs to the session, not to the setting: turning pairing on mid-quote does not reset it, as the reference's toggle is never reset by the switch either. With pairing on, the same state carries over: the next quote closes the one left open and a book-title mark nests inside the open 《.
    assert_eq!(marks(b"\"<"), ["“", "《"]);
    assert_eq!(
        read(msime_client_set_paired_punctuation(handle, true))["ok"],
        true
    );
    assert_eq!(marks(b"\"<>>"), ["”", "〈", "〉", "》"]);
    read(msime_client_destroy(handle));
}

#[test]
fn candidate_edge_uses_engine_han_text_and_preserves_unsupported_composition() {
    for (code, first, last) in [("4e2d", "中", "中"), ("20000", "𠀀", "𠀀"), ("41", "", "")] {
        for (edge, expected) in [(0, first), (1, last)] {
            let dir = tempfile::tempdir().unwrap();
            let handle = test_host(dir.path());
            read(msime_client_focus(handle, true));
            read(msime_client_character(handle, b'U', true));
            for byte in code.bytes() {
                read(msime_client_character(handle, byte, false));
            }
            let before = read(msime_client_view(handle))["value"].clone();
            assert!(
                before["candidates"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty()),
                "Missing Unicode fixture candidate for {code}: {before}"
            );
            let id = &before["candidates"][0]["id"];
            let generation = id["generation"].as_u64().unwrap();
            let index = id["index"].as_u64().unwrap() as usize;
            for invalid in [2, 255] {
                assert_eq!(
                    read(msime_client_select_edge(handle, generation, index, invalid))["ok"],
                    false
                );
                assert_eq!(read(msime_client_view(handle))["value"], before);
            }
            assert_eq!(
                read(msime_client_select_edge(
                    handle,
                    generation - 1,
                    index,
                    edge
                ))["ok"],
                false
            );
            assert_eq!(
                read(msime_client_select_edge(
                    handle,
                    generation,
                    usize::MAX,
                    edge
                ))["ok"],
                false
            );
            assert_eq!(
                std::thread::spawn(move || read(msime_client_select_edge(
                    handle, generation, index, edge
                ))["ok"]
                    .clone())
                .join()
                .unwrap(),
                false
            );
            assert_eq!(read(msime_client_view(handle))["value"], before);
            let result = read(msime_client_select_edge(handle, generation, index, edge));
            assert_eq!(result["ok"], true);
            assert_eq!(result["value"]["handled"], !expected.is_empty());
            if expected.is_empty() {
                assert!(result["value"]["commit"].is_null());
                assert_eq!(
                    result["value"]["view"]["editing_text"],
                    before["editing_text"]
                );
                assert_eq!(
                    result["value"]["view"]["candidates"][0]["text"],
                    before["candidates"][0]["text"]
                );
            } else {
                assert_eq!(result["value"]["commit"], expected);
                assert_eq!(result["value"]["view"]["editing_text"], "");
                assert!(result["value"]["view"]["candidates"]
                    .as_array()
                    .unwrap()
                    .is_empty());
            }
            assert_eq!(
                read(msime_client_select_edge(handle, generation, index, edge))["ok"],
                false
            );
            read(msime_client_destroy(handle));
            assert_eq!(
                read(msime_client_select_edge(handle, generation, index, edge))["ok"],
                false
            );
        }
    }
}

#[test]
fn live_punctuation_preserves_composition_and_survives_preferences() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'U', true));
    for byte in b"4e2d" {
        read(msime_client_character(handle, *byte, false));
    }
    let before = read(msime_client_view(handle))["value"].clone();
    for _ in 0..2 {
        let toggled = read(msime_client_set_chinese_punctuation(handle, false));
        assert_eq!(toggled["value"], before);
    }
    let preferences = Preferences {
        candidate_page_size: 2,
        ..Preferences::default()
    };
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
    let committed = read(msime_client_command(handle, 9));
    assert_eq!(committed["value"]["commit"], "中");
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], false);
    let ascii = read(msime_client_character(handle, b',', false));
    assert_eq!(ascii["value"]["handled"], false);
    assert!(ascii["value"]["commit"].is_null());
    assert_eq!(
        read(msime_client_set_chinese_punctuation(handle, true))["ok"],
        true
    );
    assert_eq!(
        read(msime_client_character(handle, b',', false))["value"]["commit"],
        "，"
    );
    assert_eq!(
        std::thread::spawn(
            move || read(msime_client_set_chinese_punctuation(handle, false))["ok"].clone()
        )
        .join()
        .unwrap(),
        false
    );
    read(msime_client_destroy(handle));
    assert_eq!(
        read(msime_client_set_chinese_punctuation(handle, true))["ok"],
        false
    );
}

#[test]
fn dedicated_english_mode_switches_through_host_api() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    let enabled = read(msime_client_set_english_mode(handle, true));
    assert_eq!(enabled["ok"], true);
    assert_eq!(enabled["value"]["focused"], true);
    assert_eq!(enabled["value"]["dedicated_english"], true);
    let typed = read(msime_client_character(handle, b'a', false));
    assert_eq!(typed["value"]["view"]["dedicated_english"], true);
    assert_eq!(typed["value"]["view"]["local_mode"], "none");
    let disabled = read(msime_client_set_english_mode(handle, false));
    assert_eq!(disabled["ok"], true);
    assert_eq!(disabled["value"]["dedicated_english"], false);
    read(msime_client_destroy(handle));
}

#[test]
fn enabling_dedicated_english_cancels_active_composition() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'n', false));
    let composing = read(msime_client_character(handle, b'i', false));
    assert!(!composing["value"]["view"]["editing_text"]
        .as_str()
        .unwrap()
        .is_empty());
    let enabled = read(msime_client_set_english_mode(handle, true));
    assert_eq!(enabled["value"]["dedicated_english"], true);
    assert_eq!(enabled["value"]["editing_text"], "");
    assert!(enabled["value"]["candidates"]
        .as_array()
        .unwrap()
        .is_empty());
    read(msime_client_destroy(handle));
}

// 默认输入状态 = 英文 is the host's passthrough state: the host keeps the
// letters and no session sees them. It must not put the session itself
// into dedicated English. A session that starts there answers the first
// key with English word candidates, and the host's own CN/EN toggle does
// not clear it - so the toggle flips between passthrough English and
// English candidates, and Chinese is unreachable.
#[test]
fn english_default_ime_mode_leaves_dedicated_english_off() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host_preferences(
        dir.path(),
        Preferences {
            default_ime_mode: msime_client_core::preferences::DefaultImeMode::English,
            ..Preferences::default()
        },
    );
    read(msime_client_focus(handle, true));
    assert_eq!(
        read(msime_client_view(handle))["value"]["dedicated_english"],
        false
    );
    let typed = read(msime_client_character(handle, b'n', false));
    assert_eq!(typed["value"]["view"]["dedicated_english"], false);
    // Still reachable - it just has to be asked for, by the menu row or
    // the hotkey that owns it.
    let enabled = read(msime_client_set_english_mode(handle, true));
    assert_eq!(enabled["value"]["dedicated_english"], true);
    read(msime_client_destroy(handle));
}

#[test]
fn preferences_wait_for_commit_keep_handle_and_reject_old_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'U', true));
    for byte in b"4e2d" {
        read(msime_client_character(handle, *byte, false));
    }
    let before = read(msime_client_view(handle))["value"].clone();
    let prefs = Preferences {
        chinese_punctuation: false,
        candidate_page_size: 2,
        ..Preferences::default()
    };
    let queued = update(handle, 1, &prefs);
    assert_eq!(queued["value"]["deferred"], true);
    assert_eq!(queued["value"]["view"], before);
    SESSIONS.with(|sessions| {
        assert!(sessions.borrow().get(&handle).unwrap().preferences_pending);
    });
    let committed = read(msime_client_command(handle, 1));
    assert_eq!(committed["value"]["commit"], "中");
    assert_eq!(committed["value"]["view"]["session"], handle);
    assert_eq!(committed["value"]["view"]["focused"], true);
    assert_eq!(update(handle, 1, &prefs)["value"]["deferred"], false);
    SESSIONS.with(|sessions| {
        assert!(!sessions.borrow().get(&handle).unwrap().preferences_pending);
    });
    assert_eq!(
        read(msime_client_character(handle, b',', false))["value"]["handled"],
        false
    );
    assert_eq!(update(handle, 0, &prefs)["ok"], false);
    assert_eq!(update(handle, 1, &Preferences::default())["ok"], false);
    let generation = before["generation"].as_u64().unwrap();
    assert_eq!(
        read(msime_client_select(handle, generation, 0))["ok"],
        false
    );
    read(msime_client_destroy(handle));
}
#[test]
fn newest_pending_preferences_win_on_blur_and_invalid_values_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'U', true));
    let off = Preferences {
        chinese_punctuation: false,
        ..Preferences::default()
    };
    assert_eq!(update(handle, 1, &off)["value"]["deferred"], true);
    let invalid = Preferences {
        candidate_page_size: 0,
        ..off.clone()
    };
    assert_eq!(update(handle, 20, &invalid)["ok"], false);
    assert_eq!(update(handle, 2, &Preferences::default())["ok"], true);
    read(msime_client_focus(handle, false));
    read(msime_client_focus(handle, true));
    assert_eq!(
        read(msime_client_character(handle, b',', false))["value"]["commit"],
        "，"
    );
    let wrong = std::thread::spawn(move || update(handle, 3, &Preferences::default()))
        .join()
        .unwrap();
    assert_eq!(wrong["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_update_preferences(handle, std::ptr::null(), 0) })["ok"],
        false
    );
    read(msime_client_destroy(handle));
}
#[test]
fn failed_rebuild_preserves_completed_input_and_retries_later() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'U', true));
    for byte in b"4e2d" {
        read(msime_client_character(handle, *byte, false));
    }
    let prefs = Preferences {
        chinese_punctuation: false,
        ..Preferences::default()
    };
    update(handle, 1, &prefs);
    // Inject invalid replacement options without touching the live Engine or disk.
    let original = SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let host = sessions.get_mut(&handle).unwrap();
        std::mem::replace(&mut host.options.resources, "relative".into())
    });
    let committed = read(msime_client_command(handle, 1));
    assert_eq!(committed["value"]["commit"], "中");
    assert!(committed["value"]["diagnostic"]
        .as_str()
        .unwrap()
        .contains("Preferences update deferred"));
    SESSIONS.with(|sessions| {
        sessions
            .borrow_mut()
            .get_mut(&handle)
            .unwrap()
            .options
            .resources = original
    });
    assert_eq!(update(handle, 1, &prefs)["value"]["deferred"], false);
    assert_eq!(
        read(msime_client_character(handle, b',', false))["value"]["handled"],
        false
    );
    read(msime_client_destroy(handle));
}
/// The contract the input hosts' dictionary-maintenance release relies on: a live session is what keeps maintenance out, destroying it is all it takes to let maintenance in, and a session asked for while maintenance runs is refused rather than queued.
#[test]
fn a_session_and_dictionary_maintenance_exclude_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let (user, dictionaries) = (path("user"), path("dictionaries"));
    let options = json!({ "api_version": 1, "resources": path("resources"), "user_data": user, "cache": path("cache"), "dictionaries": dictionaries, "preferences": { "scheme": "quanpin", "default_ime_mode": "chinese", "candidate_page_size": 5, "learning": false, "chinese_punctuation": true } }).to_string();
    let create = || read(unsafe { msime_client_create(options.as_ptr(), options.len()) });

    let maintenance = DictionaryAccess::try_maintenance(&user, &dictionaries)
        .unwrap()
        .unwrap();
    let refused = create();
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"], "dictionary maintenance busy");
    drop(maintenance);

    let created = create();
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    assert!(DictionaryAccess::try_maintenance(&user, &dictionaries)
        .unwrap()
        .is_none());
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
    assert!(DictionaryAccess::try_maintenance(&user, &dictionaries)
        .unwrap()
        .is_some());
}
pub(super) fn read(pointer: *mut c_char) -> Value {
    // SAFETY: all callers pass a fresh response allocation.
    let string = unsafe { CString::from_raw(pointer) };
    serde_json::from_slice(string.as_bytes()).unwrap()
}
#[test]
fn native_boundary_drives_real_engine_and_rejects_wrong_thread() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let options = json!({ "api_version": 1, "resources": path("resources"), "user_data": path("user"), "cache": path("cache"), "dictionaries": path("dictionaries"), "preferences": { "scheme": "quanpin", "default_ime_mode": "chinese", "candidate_page_size": 5, "learning": false, "chinese_punctuation": true } }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    let handle = created["value"]["session"].as_u64().unwrap();
    let wrong_thread = std::thread::spawn(move || read(msime_client_view(handle)))
        .join()
        .unwrap();
    assert_eq!(wrong_thread["ok"], false);
    assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
    read(msime_client_character(handle, b'U', true));
    for byte in b"4e2d" {
        assert_eq!(
            read(msime_client_character(handle, *byte, false))["ok"],
            true
        );
    }
    let result = read(msime_client_command(handle, 1));
    assert_eq!(result["value"]["commit"], "中");
    let punctuation = read(msime_client_character(handle, b',', false));
    assert_eq!(punctuation["ok"], true);
    assert_eq!(punctuation["value"]["handled"], true);
    assert_eq!(punctuation["value"]["commit"], "，");
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
    assert_eq!(read(msime_client_view(handle))["ok"], false);
    assert_eq!(read(msime_client_destroy(handle))["ok"], false);
}
#[test]
fn cloud_response_boundary_guards_identity_permission_and_buffers() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Quanpin,
        cloud_candidates: true,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    assert!(read(msime_client_online_query(handle))["value"].is_null());
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let query = read(msime_client_online_query(handle))["value"].to_string();
    let body = r#"["SUCCESS", [["nihao", ["你好"]]]]"#.as_bytes();
    let apply = |target, query: &str, body: &[u8]| {
        read(unsafe {
            msime_client_apply_cloud_response(
                target,
                query.as_ptr(),
                query.len(),
                body.as_ptr(),
                body.len(),
            )
        })
    };
    let before = read(msime_client_view(handle))["value"].clone();
    assert!(!before["candidates"].as_array().unwrap().is_empty());
    for candidate in ["", "bad\nvalue"] {
        let result = read(unsafe {
            msime_client_apply_online_candidate(
                handle,
                query.as_ptr(),
                query.len(),
                candidate.as_ptr(),
                candidate.len(),
                0,
            )
        });
        assert_eq!(result["value"]["applied"], false);
        assert_eq!(result["value"]["view"], before);
    }
    let malformed = apply(handle, &query, b"not json");
    assert_eq!(malformed["value"]["applied"], false);
    assert_eq!(malformed["value"]["view"], before);
    assert_eq!(apply(handle, &query, body)["value"]["applied"], true);
    let after_cloud = read(msime_client_view(handle))["value"].clone();
    assert_ne!(after_cloud["generation"], before["generation"]);
    assert_eq!(after_cloud["editing_text"], before["editing_text"]);
    let other_dir = tempfile::tempdir().unwrap();
    let other = test_host_preferences(other_dir.path(), preferences.clone());
    read(msime_client_focus(other, true));
    for byte in b"nihao" {
        read(msime_client_character(other, *byte, false));
    }
    assert_eq!(apply(other, &query, body)["value"]["applied"], false);
    read(msime_client_character(handle, b'a', false));
    assert_eq!(apply(handle, &query, body)["value"]["applied"], false);
    let current = read(msime_client_online_query(handle))["value"].to_string();
    let disabled = Preferences {
        cloud_candidates: false,
        ..preferences
    };
    assert_eq!(update(handle, 1, &disabled)["value"]["deferred"], true);
    assert_eq!(
        read(msime_client_online_query(handle))["value"]["cloud_candidates"],
        false
    );
    assert_eq!(apply(handle, &current, body)["value"]["applied"], false);
    for (q, qlen, b, blen) in [
        (std::ptr::null(), 0, body.as_ptr(), body.len()),
        (query.as_ptr(), query.len(), std::ptr::null(), 0),
        (
            query.as_ptr(),
            msime_input_runtime::MAX_ONLINE_QUERY_BYTES + 1,
            body.as_ptr(),
            body.len(),
        ),
        (query.as_ptr(), query.len(), body.as_ptr(), 262145),
    ] {
        assert_eq!(
            read(unsafe { msime_client_apply_cloud_response(handle, q, qlen, b, blen) })["ok"],
            false
        );
    }
    assert_eq!(
        apply(handle, "invalid", body)["error"],
        "invalid online query document"
    );
    read(msime_client_destroy(other));
    read(msime_client_destroy(handle));
}

#[test]
fn cloud_candidate_requires_an_existing_local_candidate_page() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Quanpin,
        cloud_candidates: true,
        ..chinese_preferences()
    };
    let handle = test_host_preferences(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let query = read(msime_client_online_query(handle))["value"].to_string();
    let before = read(msime_client_view(handle))["value"].clone();
    let candidates = before["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["source"], 15);
    assert_eq!(candidates[0]["text"], "☁");
    let candidate = "你好";
    let result = read(unsafe {
        msime_client_apply_online_candidate(
            handle,
            query.as_ptr(),
            query.len(),
            candidate.as_ptr(),
            candidate.len(),
            0,
        )
    });
    assert_eq!(result["value"]["applied"], true);
    assert_eq!(result["value"]["view"]["candidates"][0]["text"], "你好");
    assert_eq!(result["value"]["view"]["candidates"][0]["source"], 2);
    read(msime_client_destroy(handle));
}

#[test]
fn single_online_candidate_rejects_unknown_source() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Quanpin,
        cloud_candidates: true,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let query = read(msime_client_online_query(handle))["value"].to_string();
    let candidate = "合成候选";
    let result = read(unsafe {
        msime_client_apply_online_candidate(
            handle,
            query.as_ptr(),
            query.len(),
            candidate.as_ptr(),
            candidate.len(),
            2,
        )
    });
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error"], "invalid online candidate source");
    read(msime_client_destroy(handle));
}

#[test]
fn direct_cloud_callbacks_follow_a_pending_disable() {
    for batch in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let preferences = Preferences {
            scheme: InputScheme::Quanpin,
            cloud_candidates: true,
            ..chinese_preferences()
        };
        let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
        read(msime_client_focus(handle, true));
        for byte in b"nihao" {
            read(msime_client_character(handle, *byte, false));
        }
        let old_query = read(msime_client_online_query(handle))["value"].clone();
        assert_eq!(old_query["cloud_candidates"], true);
        let before = read(msime_client_view(handle))["value"].clone();
        assert!(!before["candidates"].as_array().unwrap().is_empty());

        let disabled = Preferences {
            cloud_candidates: false,
            ..preferences
        };
        assert_eq!(update(handle, 1, &disabled)["value"]["deferred"], true);
        let after_disable = read(msime_client_view(handle))["value"].clone();

        let query = old_query.to_string();
        let result = if batch {
            let candidates = serde_json::to_vec(&json!(["云候选"])).unwrap();
            read(unsafe {
                msime_client_apply_online_candidates(
                    handle,
                    query.as_ptr(),
                    query.len(),
                    candidates.as_ptr(),
                    candidates.len(),
                    0,
                )
            })
        } else {
            let candidate = "云候选";
            read(unsafe {
                msime_client_apply_online_candidate(
                    handle,
                    query.as_ptr(),
                    query.len(),
                    candidate.as_ptr(),
                    candidate.len(),
                    0,
                )
            })
        };
        assert_eq!(result["value"]["applied"], false, "batch={batch}: {result}");
        assert_eq!(
            result["value"]["view"], after_disable,
            "batch={batch}: {result}"
        );
        read(msime_client_destroy(handle));
    }
}

#[test]
fn cloud_disable_removes_visible_row_and_invalidates_old_query_after_reenable() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Quanpin,
        cloud_candidates: true,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let old_query = read(msime_client_online_query(handle))["value"].to_string();
    let candidates = serde_json::to_vec(&json!(["云候选"])).unwrap();
    let apply = |query: &str| {
        read(unsafe {
            msime_client_apply_online_candidates(
                handle,
                query.as_ptr(),
                query.len(),
                candidates.as_ptr(),
                candidates.len(),
                0,
            )
        })
    };
    assert_eq!(apply(&old_query)["value"]["applied"], true);
    let visible = read(msime_client_view(handle))["value"].clone();
    assert!(visible["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate| candidate["text"] == "云候选"));

    let disabled = Preferences {
        cloud_candidates: false,
        ..preferences.clone()
    };
    assert_eq!(update(handle, 1, &disabled)["value"]["deferred"], true);
    let after_disable = read(msime_client_view(handle))["value"].clone();
    assert!(!after_disable["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate| candidate["text"] == "云候选"));

    assert_eq!(update(handle, 2, &preferences)["ok"], true);
    assert_eq!(apply(&old_query)["value"]["applied"], false);
    let current_query = read(msime_client_online_query(handle))["value"].to_string();
    assert_eq!(apply(&current_query)["value"]["applied"], true);
    read(msime_client_destroy(handle));
}

fn assert_cloud_seat_for_next_composition(handle: u64, enabled: bool) {
    for byte in b"nihao" {
        assert_eq!(
            read(msime_client_character(handle, *byte, false))["ok"],
            true
        );
    }
    let view = read(msime_client_view(handle))["value"].clone();
    let candidates = view["candidates"].as_array().unwrap();
    assert!(candidates
        .iter()
        .any(|candidate| candidate["text"] == "本地"));
    assert_eq!(
        candidates.iter().any(|candidate| candidate["source"] == 15),
        enabled,
        "{view}"
    );
    assert_eq!(
        read(msime_client_online_query(handle))["value"]["cloud_candidates"],
        enabled
    );
}

#[test]
fn cloud_seat_survives_idle_preference_rebuild() {
    for enabled in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut preferences = Preferences {
            scheme: InputScheme::Quanpin,
            cloud_candidates: !enabled,
            ..chinese_preferences()
        };
        let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
        assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
        preferences.cloud_candidates = enabled;
        assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], false);
        assert_cloud_seat_for_next_composition(handle, enabled);
        read(msime_client_destroy(handle));
    }
}

#[test]
fn cloud_seat_survives_deferred_preference_rebuild() {
    for enabled in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut preferences = Preferences {
            scheme: InputScheme::Quanpin,
            cloud_candidates: !enabled,
            ..chinese_preferences()
        };
        let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
        assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
        assert_cloud_seat_for_next_composition(handle, !enabled);
        preferences.cloud_candidates = enabled;
        assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
        let committed = read(msime_client_command(handle, 1));
        assert_eq!(committed["ok"], true);
        assert_eq!(committed["value"]["commit"], "本地");
        SESSIONS.with(|sessions| {
            assert!(!sessions.borrow().get(&handle).unwrap().preferences_pending);
        });
        assert_cloud_seat_for_next_composition(handle, enabled);
        read(msime_client_destroy(handle));
    }
}

#[test]
fn cloud_seat_survives_resource_only_engine_rebuild() {
    for enabled in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let preferences = Preferences {
            scheme: InputScheme::Quanpin,
            cloud_candidates: enabled,
            ..chinese_preferences()
        };
        let handle = test_host_with_pinyin_fixture(dir.path(), preferences);
        assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
        SESSIONS.with(|sessions| {
            let mut sessions = sessions.borrow_mut();
            let session = sessions.get_mut(&handle).unwrap();
            assert!(session.requested.is_none());
            session.resources_pending = true;
        });
        assert_eq!(read(msime_client_focus(handle, false))["ok"], true);
        assert_eq!(read(msime_client_focus(handle, true))["ok"], true);
        SESSIONS.with(|sessions| {
            assert!(!sessions.borrow().get(&handle).unwrap().resources_pending);
        });
        assert_cloud_seat_for_next_composition(handle, enabled);
        read(msime_client_destroy(handle));
    }
}

#[test]
fn direct_cloud_callbacks_follow_a_pending_enable() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Preferences {
        scheme: InputScheme::Quanpin,
        cloud_candidates: false,
        ..chinese_preferences()
    };
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    assert_eq!(
        read(msime_client_online_query(handle))["value"]["cloud_candidates"],
        false
    );

    // A menu switch during the composition defers the preference change, but the cloud row it asks for is taken at once.
    let enabled = Preferences {
        cloud_candidates: true,
        ..preferences
    };
    assert_eq!(update(handle, 1, &enabled)["value"]["deferred"], true);
    let query = read(msime_client_online_query(handle))["value"].clone();
    assert_eq!(query["cloud_candidates"], true);
    let query = query.to_string();
    let candidates = serde_json::to_vec(&json!(["云候选"])).unwrap();
    let result = read(unsafe {
        msime_client_apply_online_candidates(
            handle,
            query.as_ptr(),
            query.len(),
            candidates.as_ptr(),
            candidates.len(),
            0,
        )
    });
    assert_eq!(result["value"]["applied"], true, "{result}");
    assert!(result["value"]["view"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate| candidate["text"] == "云候选"));
    read(msime_client_destroy(handle));
}

#[test]
fn initial_session_preserves_first_page_english_count_when_cloud_arrives() {
    let root = tempfile::tempdir().unwrap();
    let dictionaries = root.path().join("dictionaries");
    std::fs::create_dir_all(&dictionaries).unwrap();
    rusqlite::Connection::open(dictionaries.join("msime-english.db"))
        .unwrap().execute_batch(
            "CREATE TABLE english_words(word TEXT,display TEXT,weight INTEGER);\
             INSERT INTO english_words VALUES('nihao','nihao',100),('nihaos','nihaos',90),('nihaoly','nihaoly',80);",
        ).unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        candidate_page_size: 6,
        cloud_candidates: true,
        ..chinese_preferences()
    };
    preferences.mixed_input.english = true;
    preferences.mixed_input.first_page_count = 3;
    let handle = test_host_with_pinyin_fixture(root.path(), preferences);
    read(msime_client_focus(handle, true));
    type_letters(handle, b"nihao");
    let query = read(msime_client_online_query(handle))["value"].to_string();
    let candidates = serde_json::to_vec(&json!(["合成云候选"])).unwrap();
    let applied = read(unsafe {
        msime_client_apply_online_candidates(
            handle,
            query.as_ptr(),
            query.len(),
            candidates.as_ptr(),
            candidates.len(),
            0,
        )
    });
    assert_eq!(applied["value"]["applied"], true, "{applied}");
    let actual: Vec<_> = applied["value"]["view"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        &actual[..5],
        ["本地", "合成云候选", "nihao", "nihaos", "nihaoly"]
    );
    read(msime_client_destroy(handle));
}

#[test]
fn clearing_online_source_removes_its_rows_without_empty_batch_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        cloud_candidates: true,
        ..chinese_preferences()
    };
    preferences.ai_assistant.enabled = true;
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let query = read(msime_client_online_query(handle))["value"].to_string();
    let apply = |source: u8, text: &str| {
        let candidates = serde_json::to_vec(&json!([text])).unwrap();
        read(unsafe {
            msime_client_apply_online_candidates(
                handle,
                query.as_ptr(),
                query.len(),
                candidates.as_ptr(),
                candidates.len(),
                source,
            )
        })
    };
    let cloud = apply(0, "云候选");
    assert_eq!(cloud["value"]["applied"], true, "{cloud}");
    assert_eq!(apply(1, "AI候选")["value"]["applied"], true);
    let cleared = read(msime_client_clear_online_candidates(handle, 0));
    assert_eq!(cleared["value"]["applied"], true);
    let candidates = cleared["value"]["view"]["candidates"].as_array().unwrap();
    assert!(!candidates.iter().any(|item| item["text"] == "云候选"));
    assert!(candidates.iter().any(|item| item["text"] == "AI候选"));
    read(msime_client_destroy(handle));
}

/// #6888：开着 AI 时不再放占位行（来源 12）；宿主在 AI 失败后照旧调用的 resolve 只校验参数、回答 `applied:false`，视图和代次都不变；AI 回答到达时第一条坐第一页最后一格，前面的候选不挪位。
#[test]
fn an_ai_answer_takes_the_last_slot_of_the_first_page_and_resolve_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        candidate_page_size: 5,
        ..chinese_preferences()
    };
    preferences.ai_assistant.enabled = true;
    preferences.ai_assistant.provider = "deepseek".into();
    preferences.ai_assistant.endpoint = "https://api.deepseek.com/chat/completions".into();
    preferences.ai_assistant.model = "synthetic-model".into();
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    let token = "synthetic-session-key";
    assert_eq!(
        read(unsafe { msime_client_set_ai_credential(handle, token.as_ptr(), token.len()) })
            ["value"],
        true
    );
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let texts = |view: &Value| -> Vec<String> {
        view["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|candidate| candidate["text"].as_str().unwrap().to_owned())
            .collect()
    };
    let before = read(msime_client_view(handle))["value"].clone();
    assert!(!before["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate| candidate["source"] == 12));

    let query = read(msime_client_online_query(handle))["value"].to_string();
    let resolve = |source: u8| {
        read(unsafe {
            msime_client_resolve_online_candidates(handle, query.as_ptr(), query.len(), source)
        })
    };
    for source in [0, 1] {
        let resolved = resolve(source);
        assert_eq!(resolved["value"]["applied"], false, "{resolved}");
        assert_eq!(resolved["value"]["view"], before, "source {source}");
    }
    assert_eq!(resolve(2)["ok"], false);
    let garbage = b"not a query";
    assert_eq!(
        read(unsafe {
            msime_client_resolve_online_candidates(handle, garbage.as_ptr(), garbage.len(), 1)
        })["ok"],
        false
    );

    let candidates = serde_json::to_vec(&json!(["合成一", "合成二"])).unwrap();
    let applied = read(unsafe {
        msime_client_apply_online_candidates(
            handle,
            query.as_ptr(),
            query.len(),
            candidates.as_ptr(),
            candidates.len(),
            1,
        )
    });
    assert_eq!(applied["value"]["applied"], true, "{applied}");
    let after = &applied["value"]["view"];
    let shown = texts(&before);
    let seat = shown.len().min(4);
    assert_eq!(texts(after)[..seat], shown[..seat]);
    assert_eq!(after["candidates"][seat]["text"], "合成一");
    assert_eq!(after["candidates"][seat]["source"], 3);
    read(msime_client_destroy(handle));
}

#[test]
fn an_ai_credential_handed_over_in_memory_signs_requests_without_being_stored() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        ..chinese_preferences()
    };
    preferences.ai_assistant.enabled = true;
    preferences.ai_assistant.provider = "deepseek".into();
    preferences.ai_assistant.model = "synthetic-model".into();
    preferences.ai_assistant.endpoint = "https://api.deepseek.com/chat/completions".into();
    let handle = test_host_preferences(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let query = read(msime_client_online_query(handle))["value"].to_string();
    let request = |handle: u64| {
        read(unsafe { msime_client_ai_request_for_query(handle, query.as_ptr(), query.len()) })
    };
    let set = |token: &str| {
        read(unsafe { msime_client_set_ai_credential(handle, token.as_ptr(), token.len()) })
    };
    assert_ne!(request(handle)["ok"], true, "no token anywhere");
    assert_eq!(set("synthetic-keychain")["ok"], true);
    let descriptor = request(handle);
    assert_eq!(descriptor["ok"], true);
    assert_eq!(
        descriptor["value"]["headers"]["Authorization"],
        "Bearer synthetic-keychain"
    );
    assert!(!read(msime_client_view(handle))
        .to_string()
        .contains("synthetic-keychain"));
    assert!(!read(msime_client_online_query(handle))
        .to_string()
        .contains("synthetic-keychain"));
    assert_eq!(set("bad\ntoken")["ok"], false);
    assert_eq!(set("")["ok"], true);
    assert_ne!(request(handle)["ok"], true, "cleared");
    read(msime_client_destroy(handle));
}

#[test]
fn rotating_ai_credential_clears_old_candidates_and_rejects_old_query() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        ..chinese_preferences()
    };
    preferences.ai_assistant.enabled = true;
    preferences.ai_assistant.provider = "deepseek".into();
    preferences.ai_assistant.model = "synthetic-model".into();
    preferences.ai_assistant.endpoint = "https://api.deepseek.com/chat/completions".into();
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    let set_credential = |token: &str| {
        read(unsafe { msime_client_set_ai_credential(handle, token.as_ptr(), token.len()) })
    };
    assert_eq!(set_credential("synthetic-first")["ok"], true);
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let apply = |query: &Value, text: &str| {
        let query = query.to_string();
        let candidates = serde_json::to_vec(&json!([text])).unwrap();
        read(unsafe {
            msime_client_apply_online_candidates(
                handle,
                query.as_ptr(),
                query.len(),
                candidates.as_ptr(),
                candidates.len(),
                1,
            )
        })
    };
    let first_query = read(msime_client_online_query(handle))["value"].clone();
    assert_eq!(apply(&first_query, "合成旧候选")["value"]["applied"], true);
    let old_query = read(msime_client_online_query(handle))["value"].clone();
    let old_view = read(msime_client_view(handle))["value"].clone();
    assert!(old_view["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["text"] == "合成旧候选"));

    assert_eq!(set_credential("synthetic-second")["ok"], true);
    let new_view = read(msime_client_view(handle))["value"].clone();
    assert!(new_view["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["text"] != "合成旧候选"));
    let new_query = read(msime_client_online_query(handle))["value"].clone();
    assert_ne!(new_query["generation"], old_query["generation"]);
    assert_eq!(apply(&old_query, "迟到旧候选")["value"]["applied"], false);
    assert_eq!(apply(&new_query, "合成新候选")["value"]["applied"], true);
    let stable_view = read(msime_client_view(handle))["value"].clone();
    assert_eq!(set_credential("synthetic-second")["ok"], true);
    assert_eq!(read(msime_client_view(handle))["value"], stable_view);
    read(msime_client_destroy(handle));
}

#[test]
fn updating_ai_preferences_clears_old_candidates_when_only_token_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        ..chinese_preferences()
    };
    preferences.ai_assistant.enabled = true;
    preferences.ai_assistant.provider = "deepseek".into();
    preferences.ai_assistant.model = "synthetic-model".into();
    preferences.ai_assistant.endpoint = "https://api.deepseek.com/chat/completions".into();
    preferences.ai_assistant.token = "synthetic-first".into();
    let handle = test_host_with_pinyin_fixture(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let apply = |query: &Value, text: &str| {
        let query = query.to_string();
        let candidates = serde_json::to_vec(&json!([text])).unwrap();
        read(unsafe {
            msime_client_apply_online_candidates(
                handle,
                query.as_ptr(),
                query.len(),
                candidates.as_ptr(),
                candidates.len(),
                1,
            )
        })
    };
    let first_query = read(msime_client_online_query(handle))["value"].clone();
    assert_eq!(apply(&first_query, "合成旧候选")["value"]["applied"], true);
    let old_query = read(msime_client_online_query(handle))["value"].clone();

    preferences.ai_assistant.token = "synthetic-second".into();
    assert_eq!(update(handle, 1, &preferences)["value"]["deferred"], true);
    let new_view = read(msime_client_view(handle))["value"].clone();
    assert!(new_view["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["text"] != "合成旧候选"));
    let new_query = read(msime_client_online_query(handle))["value"].clone();
    assert_ne!(new_query["generation"], old_query["generation"]);
    assert_eq!(apply(&old_query, "迟到旧候选")["value"]["applied"], false);
    assert_eq!(apply(&new_query, "合成新候选")["value"]["applied"], true);
    read(msime_client_destroy(handle));
}

#[test]
fn an_in_memory_ai_credential_overrides_a_stored_origin_token() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        ..chinese_preferences()
    };
    preferences.ai_assistant.enabled = true;
    preferences.ai_assistant.provider = "deepseek".into();
    preferences.ai_assistant.model = "synthetic-model".into();
    preferences.ai_assistant.endpoint = "https://api.deepseek.com/chat/completions".into();
    preferences.ai_assistant.tokens.insert(
        "https://api.deepseek.com:443".into(),
        "synthetic-stored".into(),
    );
    let handle = test_host_preferences(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let query = read(msime_client_online_query(handle))["value"].to_string();
    let token = b"synthetic-keychain";
    assert_eq!(
        read(unsafe { msime_client_set_ai_credential(handle, token.as_ptr(), token.len()) })["ok"],
        true
    );
    let descriptor =
        read(unsafe { msime_client_ai_request_for_query(handle, query.as_ptr(), query.len()) });
    assert_eq!(descriptor["ok"], true);
    assert_eq!(
        descriptor["value"]["headers"]["Authorization"],
        "Bearer synthetic-keychain"
    );
    assert_eq!(
        read(unsafe { msime_client_set_ai_credential(handle, std::ptr::null(), 0) })["ok"],
        true
    );
    let restored =
        read(unsafe { msime_client_ai_request_for_query(handle, query.as_ptr(), query.len()) });
    assert_eq!(
        restored["value"]["headers"]["Authorization"],
        "Bearer synthetic-stored"
    );
    read(msime_client_destroy(handle));
}

#[test]
fn an_in_memory_ai_credential_stays_bound_to_its_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        ..chinese_preferences()
    };
    preferences.ai_assistant.enabled = true;
    preferences.ai_assistant.provider = "deepseek".into();
    preferences.ai_assistant.model = "synthetic-model".into();
    preferences.ai_assistant.endpoint = "https://api.deepseek.com/chat/completions".into();
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let token = b"synthetic-deepseek-keychain";
    assert_eq!(
        read(unsafe { msime_client_set_ai_credential(handle, token.as_ptr(), token.len()) })["ok"],
        true
    );

    preferences.ai_assistant.provider = "openai".into();
    preferences.ai_assistant.endpoint = "https://api.openai.com/v1/chat/completions".into();
    assert_eq!(update(handle, 1, &preferences)["ok"], true);
    let query = read(msime_client_online_query(handle))["value"].to_string();
    let descriptor =
        read(unsafe { msime_client_ai_request_for_query(handle, query.as_ptr(), query.len()) });
    assert_ne!(
        descriptor["ok"], true,
        "old keychain token reached a new endpoint"
    );
    read(msime_client_destroy(handle));
}

#[test]
fn the_ai_request_sends_the_platforms_candidate_limit() {
    // 存下的数量是 1。固定数量的平台（macOS、Windows、Linux）不读它：在线查询和请求描述用同一个固定值，`chat_completion_http_request` 才不会因为两边不一致拒绝请求。
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        ..chinese_preferences()
    };
    preferences.ai_assistant.enabled = true;
    preferences.ai_assistant.model = "synthetic-model".into();
    preferences.ai_assistant.token = "synthetic-private".into();
    preferences.ai_assistant.endpoint = "https://api.deepseek.com/chat/completions".into();
    preferences.ai_assistant.candidate_limit = 1;
    let expected = host_ai_candidate_limit(1);
    let handle = test_host_preferences(dir.path(), preferences);
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let query = read(msime_client_online_query(handle))["value"].clone();
    assert_eq!(query["ai_assistant"]["candidate_limit"], expected);
    let query = query.to_string();
    let descriptor =
        read(unsafe { msime_client_ai_request_for_query(handle, query.as_ptr(), query.len()) });
    assert_eq!(descriptor["ok"], true);
    let user: Value = serde_json::from_str(
        descriptor["value"]["body"]["messages"][1]["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(user["candidate_limit"], expected);
    read(msime_client_destroy(handle));
}

#[test]
fn ai_queries_and_delivery_follow_pending_preferences() {
    let dir = tempfile::tempdir().unwrap();
    let mut preferences = Preferences {
        scheme: InputScheme::Quanpin,
        ..chinese_preferences()
    };
    preferences.ai_assistant.enabled = true;
    preferences.ai_assistant.model = "synthetic-original".into();
    preferences.ai_assistant.token = "synthetic-private".into();
    // An enabled assistant with no endpoint has nowhere to send anything;
    // a real one is always configured with the provider's URL.
    preferences.ai_assistant.endpoint = "https://api.deepseek.com/chat/completions".into();
    let handle = test_host_preferences(dir.path(), preferences.clone());
    read(msime_client_focus(handle, true));
    for byte in b"nihaoshijie" {
        read(msime_client_character(handle, *byte, false));
    }
    let apply = |query: &Value, batch: bool| {
        let query = query.to_string();
        if batch {
            let candidates = serde_json::to_vec(&json!(["合成候选"])).unwrap();
            read(unsafe {
                msime_client_apply_online_candidates(
                    handle,
                    query.as_ptr(),
                    query.len(),
                    candidates.as_ptr(),
                    candidates.len(),
                    1,
                )
            })
        } else {
            let candidate = "合成候选".as_bytes();
            read(unsafe {
                msime_client_apply_online_candidate(
                    handle,
                    query.as_ptr(),
                    query.len(),
                    candidate.as_ptr(),
                    candidate.len(),
                    1,
                )
            })
        }
    };
    let original = read(msime_client_online_query(handle))["value"].clone();
    assert_eq!(original["ai_eligible"], true);
    assert!(!original.to_string().contains("synthetic-private"));
    let original_bytes = original.to_string();
    let descriptor = read(unsafe {
        msime_client_ai_request_for_query(handle, original_bytes.as_ptr(), original_bytes.len())
    });
    assert_eq!(descriptor["ok"], true);
    assert_eq!(descriptor["value"]["method"], "POST");
    assert_eq!(
        descriptor["value"]["headers"]["Content-Type"],
        "application/json"
    );
    for revision in 1..=5 {
        let old = read(msime_client_online_query(handle))["value"].clone();
        match revision {
            1 => preferences.ai_assistant.enabled = false,
            2 => {
                preferences.ai_assistant.enabled = true;
                preferences.ai_assistant.model = "synthetic-new".into();
            }
            3 => {
                preferences.ai_assistant.endpoint =
                    "https://synthetic.invalid/v1/chat/completions".into()
            }
            4 => preferences.ai_assistant.prompt_custom_1 = "synthetic prompt".into(),
            _ => preferences.ai_assistant.candidate_limit = 1,
        }
        assert_eq!(
            update(handle, revision, &preferences)["value"]["deferred"],
            true
        );
        let current = read(msime_client_online_query(handle))["value"].clone();
        assert!(current["generation"].as_u64().unwrap() > old["generation"].as_u64().unwrap());
        assert_eq!(current["query_text"], original["query_text"]);
        assert_eq!(
            current["ai_assistant"].is_null(),
            !preferences.ai_assistant.enabled
        );
        if preferences.ai_assistant.enabled {
            assert_eq!(
                current["ai_assistant"]["model"],
                preferences.ai_assistant.model
            );
            assert_eq!(
                current["ai_assistant"]["endpoint"],
                preferences.ai_assistant.endpoint
            );
            assert_eq!(
                current["ai_assistant"]["prompt_custom_1"],
                preferences.ai_assistant.prompt_custom_1
            );
            // 固定候选数量的平台（macOS、Windows、Linux）不读存下的值，查询里始终是固定值。
            assert_eq!(
                current["ai_assistant"]["candidate_limit"],
                host_ai_candidate_limit(preferences.ai_assistant.candidate_limit)
            );
        }
        for batch in [false, true] {
            assert_eq!(apply(&old, batch)["value"]["applied"], false);
        }
    }
    let current = read(msime_client_online_query(handle))["value"].clone();
    let before_ai = read(msime_client_view(handle))["value"].clone();
    assert_eq!(apply(&current, true)["value"]["applied"], true);
    let after_ai = read(msime_client_view(handle))["value"].clone();
    assert_ne!(after_ai["generation"], before_ai["generation"]);
    read(msime_client_destroy(handle));
}
#[test]
fn custom_translation_plan_accepts_keyboard_model_sentence_candidates() {
    let plan = |target: &str, text: &str| {
        let request = serde_json::to_vec(&json!({
            "target_language": target,
            "candidates": [{
                "text": text,
                "source": msime_engine::CandidateSource::NeuralKeyboard as u8,
            }],
        }))
        .unwrap();
        read(unsafe { msime_client_custom_translation_plan(request.as_ptr(), request.len()) })
    };
    for target in ["en", "fr", "ja", "es", "ru", "de", "ko"] {
        let planned = plan(target, "今天下雨");
        assert_eq!(planned["ok"], true);
        assert_eq!(
            planned["value"],
            json!([{
                "text": "今天下雨",
                "key": "今天下雨",
                "source_language": "zh",
                "target_language": target,
            }]),
            "{target}"
        );
    }
    assert_eq!(
        plan("fr", "Synthetic Word")["value"],
        json!([{
            "text": "Synthetic Word",
            "key": "synthetic word",
            "source_language": "en",
            "target_language": "zh",
        }])
    );
}

#[test]
fn custom_translation_plan_rejects_pending_special_and_unknown_sources() {
    for source in [6, 7, 10, 12, 13, 14, 15, 255] {
        let request = serde_json::to_vec(&json!({
            "target_language": "en",
            "candidates": [{"text": "合成", "source": source}],
        }))
        .unwrap();
        let planned =
            read(unsafe { msime_client_custom_translation_plan(request.as_ptr(), request.len()) });
        assert_eq!(planned["ok"], true, "source={source}");
        assert_eq!(planned["value"], json!([]), "source={source}");
    }
}

#[test]
fn custom_translation_plan_preserves_direction_and_filters_visible_sources() {
    let plan = |request: Value| {
        let bytes = serde_json::to_vec(&request).unwrap();
        read(unsafe { msime_client_custom_translation_plan(bytes.as_ptr(), bytes.len()) })
    };
    let candidates = json!([
        {"text":"Hello","source":4},
        {"text":"测试","source":0},
        {"text":"Hello","source":4},
        {"text":"smile","source":6},
        {"text":"smile","source":7},
        {"text":"123","source":0},
        {"text":"test😀","source":0},
        {"text":"x".repeat(257),"source":0},
        {"text":"unknown","source":10}
    ]);
    for target in ["en", "fr", "ja", "es", "ru", "de", "ko"] {
        assert_eq!(
            plan(json!({"target_language":target,"candidates":candidates}))["value"],
            json!([
                {"text":"Hello","key":"hello","source_language":"en","target_language":"zh"},
                {"text":"测试","key":"测试","source_language":"zh","target_language":target}
            ])
        );
    }
    for request in [
        json!({"target_language":"unknown","candidates":[]}),
        json!({"target_language":"en","candidates":vec![json!({"text":"hello","source":0}); 11]}),
        json!({"target_language":"en","candidates":[{"text":"hello","source":true}]}),
    ] {
        assert_eq!(plan(request)["ok"], false);
    }
    assert_eq!(
        read(unsafe { msime_client_custom_translation_plan(std::ptr::null(), 0) })["ok"],
        false
    );
}
#[test]
fn mixed_script_chinese_candidates_are_planned_and_saved_like_the_windows_source() {
    // Windows `IsCloudTranslatableChinese` only needs one Han codepoint and no emoji, so words such as 卡拉OK and T恤 are translated zh->target and their English gloss is learned.
    let bytes = serde_json::to_vec(&json!({
        "target_language": "en",
        "candidates": [
            {"text":"卡拉OK","source":0},
            {"text":"T恤","source":0},
            {"text":"T恤😀","source":0},
        ],
    }))
    .unwrap();
    assert_eq!(
        read(unsafe { msime_client_custom_translation_plan(bytes.as_ptr(), bytes.len()) })["value"],
        json!([
            {"text":"卡拉OK","key":"卡拉OK","source_language":"zh","target_language":"en"},
            {"text":"T恤","key":"T恤","source_language":"zh","target_language":"en"}
        ])
    );
    let user = tempfile::tempdir().unwrap();
    let user_path = user.path().to_str().unwrap();
    let request = serde_json::to_vec(&json!({
        "target_language": "en",
        "translations": [{"text":"卡拉OK","translation":"karaoke"}],
    }))
    .unwrap();
    let saved = read(unsafe {
        msime_client_translation_gloss_save(
            request.as_ptr(),
            request.len(),
            user_path.as_ptr(),
            user_path.len(),
        )
    });
    assert_eq!(saved["value"]["saved"], 1);
    let database = rusqlite::Connection::open(user.path().join("translation-glosses.db")).unwrap();
    assert_eq!(
        database
            .query_row(
                "SELECT english_gloss FROM zh_en_glosses WHERE chinese='卡拉OK'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "karaoke"
    );
}
#[test]
fn learned_translation_buffers_are_bounded() {
    assert_eq!(
        read(unsafe { msime_client_ai_http_request(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_ai_http_request(b"x".as_ptr(), 65537) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_parse_ai_response(b"x".as_ptr(), 1048577, 1) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_parse_ai_response(b"x".as_ptr(), 1, 11) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_ai_translation_http_request(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_parse_ai_translation_response(b"x".as_ptr(), 1048577) })["ok"],
        false
    );
    let request = serde_json::to_vec(&json!({
        "config": {
            "enabled": true,
            "provider": "openai",
            "endpoint": "https://synthetic.invalid/v1/chat/completions",
            "model": "synthetic-model",
            "tokens": {"https://synthetic.invalid:443": "synthetic-token"}
        },
        "input": {"text": "合成翻译句子", "source_language": "zh", "target_language": "en"}
    }))
    .unwrap();
    let descriptor =
        read(unsafe { msime_client_ai_translation_http_request(request.as_ptr(), request.len()) });
    assert_eq!(descriptor["ok"], true);
    assert_eq!(descriptor["value"]["body"]["model"], "synthetic-model");
    let body = serde_json::to_vec(&json!({
        "choices": [{"message": {"content": "{\"translation\":\"synthetic gloss\"}"}}]
    }))
    .unwrap();
    assert_eq!(
        read(unsafe { msime_client_parse_ai_translation_response(body.as_ptr(), body.len()) })
            ["value"],
        "synthetic gloss"
    );
    let empty = br#"{"choices":[{"message":{"content":"{\"translation\":\"\"}"}}]}"#;
    assert!(!unsafe { msime_client_ai_translation_reply_failed(empty.as_ptr(), empty.len()) });
    let failed = br#"{"error":{"message":"synthetic failure"}}"#;
    assert!(unsafe { msime_client_ai_translation_reply_failed(failed.as_ptr(), failed.len()) });
    for (pointer, length) in [(std::ptr::null(), 0), (b"x".as_ptr(), 65537)] {
        assert_eq!(
            read(unsafe { msime_client_learned_translation_request(pointer, length) })["ok"],
            false
        );
    }
}
#[test]
fn formats_host_translation_glosses_like_provider_replies() {
    let format = |text: &[u8]| {
        read(unsafe { msime_client_format_translation_gloss(text.as_ptr(), text.len()) })
    };
    assert_eq!(format(b"test\nfoo ")["value"], "test foo");
    assert_eq!(format(b" \n")["value"], Value::Null);
    assert_eq!(format(b"a\x07b")["value"], Value::Null);
    assert_eq!(format(b"\xff")["ok"], false);
    assert_eq!(
        read(unsafe { msime_client_format_translation_gloss(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_format_translation_gloss(b"x".as_ptr(), 65537) })["ok"],
        false
    );
}
#[test]
fn tencent_translation_buffers_are_bounded() {
    assert_eq!(
        read(unsafe { msime_client_tencent_translation_http_request(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_tencent_translation_http_request(b"x".as_ptr(), 65537) })["ok"],
        false
    );
    for (length, expected) in [(1048577, 1), (1, 0), (1, 11)] {
        assert_eq!(
            read(unsafe {
                msime_client_parse_tencent_translation_response(b"x".as_ptr(), length, expected)
            })["ok"],
            false
        );
    }
}
#[test]
fn custom_translation_http_bridge_is_bounded_and_pure() {
    let build = |request: Value| {
        let bytes = serde_json::to_vec(&request).unwrap();
        read(unsafe { msime_client_custom_translation_http_request(bytes.as_ptr(), bytes.len()) })
    };
    let request = json!({"config":{"enabled":true,"endpoint":"https://translation.invalid/api","api_key":"synthetic"},
        "text":"hello","source_language":"en","target_language":"zh"});
    let value = build(request.clone());
    assert_eq!(value["ok"], true);
    assert_eq!(value["value"]["method"], "POST");
    assert_eq!(
        value["value"]["headers"]["Authorization"],
        "Bearer synthetic"
    );
    assert_eq!(
        value["value"]["body"],
        json!({"text":"hello","source_lang":"EN","target_lang":"ZH"})
    );
    assert_eq!(value["value"]["timeout_ms"], 2500);
    assert_eq!(value["value"]["max_response_bytes"], 1048576);
    let mut padded = request.clone();
    padded["config"]["endpoint"] = json!("  https://translation.invalid/api  ");
    padded["config"]["api_key"] = json!("  synthetic  ");
    let padded_value = build(padded);
    assert_eq!(
        padded_value["value"]["url"],
        "https://translation.invalid/api"
    );
    assert_eq!(
        padded_value["value"]["headers"]["Authorization"],
        "Bearer synthetic"
    );
    let mut disabled = request.clone();
    disabled["config"]["enabled"] = json!(false);
    assert!(build(disabled)["value"].is_null());
    let mut keyless = request.clone();
    keyless["config"]["api_key"] = json!("");
    assert!(build(keyless)["value"]["headers"]
        .get("Authorization")
        .is_none());
    for (field, value) in [
        ("text", "x".repeat(257)),
        ("source_language", "en\r\n".into()),
    ] {
        let mut invalid = request.clone();
        invalid[field] = json!(value);
        assert_eq!(
            build(invalid)["error"],
            "invalid custom translation parameters"
        );
    }
    for codepoint in (0..=0x1f).chain(0x7f..=0x9f) {
        let control = char::from_u32(codepoint).unwrap();
        let mut invalid = request.clone();
        invalid["text"] = json!(format!("before{control}after"));
        assert_eq!(
            build(invalid)["error"],
            "invalid custom translation parameters"
        );
    }
    for (field, value) in [
        ("endpoint", "file:///synthetic"),
        ("api_key", "synthetic\r\nheader"),
    ] {
        let mut invalid = request.clone();
        invalid["config"][field] = json!(value);
        assert_eq!(
            build(invalid)["error"],
            "invalid custom translation parameters"
        );
    }
    let parse = |body: &[u8]| {
        read(unsafe { msime_client_parse_custom_translation_response(body.as_ptr(), body.len()) })
    };
    assert_eq!(parse(br#"{"data":"translated"}"#)["value"], "translated");
    assert_eq!(
        parse(br#"{"data":"  hello\nworld\t "}"#)["value"],
        "hello world"
    );
    assert!(parse(br#"{"data":"bad\u0000gloss"}"#)["value"].is_null());
    for body in [
        b"invalid".as_slice(),
        br#"{"code":500,"data":"ignored"}"#,
        b"\xff",
    ] {
        assert!(parse(body)["value"].is_null());
    }
    assert!(parse(json!({"data":"x".repeat(4097)}).to_string().as_bytes())["value"].is_null());
    assert_eq!(
        read(unsafe { msime_client_custom_translation_http_request(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_parse_custom_translation_response(b"x".as_ptr(), 1048577) })
            ["ok"],
        false
    );
}

/// 官方 DeepL 地址在描述器里换成 DeepL 的鉴权和正文；Windows、macOS、iOS 和鸿蒙的键盘都直接发这个描述器。
#[test]
fn custom_translation_http_bridge_speaks_deepl_to_official_endpoints() {
    let build = |endpoint: &str, key: &str| {
        let request = json!({"config":{"enabled":true,"endpoint":endpoint,"api_key":key},
            "text":"测试","source_language":"zh","target_language":"en"});
        let bytes = serde_json::to_vec(&request).unwrap();
        read(unsafe { msime_client_custom_translation_http_request(bytes.as_ptr(), bytes.len()) })
    };
    let value = build(" https://api.deepl.com/v2/translate ", " synthetic-key:fx ");
    assert_eq!(value["ok"], true);
    assert_eq!(
        value["value"]["url"],
        "https://api-free.deepl.com/v2/translate"
    );
    assert_eq!(
        value["value"]["headers"],
        json!({"Content-Type":"application/json","Authorization":"DeepL-Auth-Key synthetic-key:fx"})
    );
    assert_eq!(
        value["value"]["body"],
        json!({"text":["测试"],"source_lang":"ZH","target_lang":"EN-US"})
    );
    assert_eq!(value["value"]["timeout_ms"], 2500);
    assert_eq!(value["value"]["max_response_bytes"], 1048576);
    assert_eq!(
        build("https://api-free.deepl.com/v2/translate", "")["error"],
        "invalid custom translation parameters"
    );
    // 最长的密钥加上 `DeepL-Auth-Key ` 前缀是 4111 字节，macOS 的描述器校验按这个上限放行。
    let longest = build("https://api.deepl.com/v2/translate", &"k".repeat(4096));
    assert_eq!(
        longest["value"]["headers"]["Authorization"]
            .as_str()
            .unwrap()
            .len(),
        4111
    );
    let parse = |body: &[u8]| {
        read(unsafe { msime_client_parse_custom_translation_response(body.as_ptr(), body.len()) })
    };
    let reply = br#"{"translations":[{"detected_source_language":"ZH","text":"Test"}]}"#;
    assert_eq!(parse(reply)["value"], "Test");
    assert!(!unsafe { msime_client_custom_translation_reply_failed(reply.as_ptr(), reply.len()) });
    for body in [
        br#"{"message":"Forbidden"}"#.as_slice(),
        br#"{"message":"Quota Exceeded"}"#,
    ] {
        assert!(parse(body)["value"].is_null());
        assert!(unsafe { msime_client_custom_translation_reply_failed(body.as_ptr(), body.len()) });
    }
}

#[test]
fn translation_object_fallback_reaches_the_custom_response_bridge() {
    for (body, expected) in [
        (
            r#"{"data":{"text":"","translation":"synthetic"}}"#,
            "synthetic",
        ),
        (
            r#"{"code":200,"result":[{"text":"","data":"  synthetic\ntranslation  "}]}"#,
            "synthetic translation",
        ),
        (
            r#"{"translations":[{"text":"","translation":"synthetic translation"}]}"#,
            "synthetic translation",
        ),
    ] {
        assert!(!unsafe {
            msime_client_custom_translation_reply_failed(body.as_ptr(), body.len())
        });
        let parsed = read(unsafe {
            msime_client_parse_custom_translation_response(body.as_ptr(), body.len())
        });
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["value"], expected, "{body}");
    }
}

#[test]
fn custom_translation_reply_success_code_requires_a_supported_result() {
    for (body, failed) in [
        (r#"{"code":200}"#, true),
        (r#"{"code":"200","data":null}"#, true),
        (r#"{"code":200,"translation":42}"#, true),
        (r#"{"code":200,"data":""}"#, false),
        (r#"{"code":"200","translations":[]}"#, false),
        (r#"{"code":200,"translation":{"text":""}}"#, false),
        (r#"{"code":429,"data":""}"#, true),
        (r#"{"code":200,"error":"rate limited","data":""}"#, true),
    ] {
        assert_eq!(
            unsafe { msime_client_custom_translation_reply_failed(body.as_ptr(), body.len()) },
            failed,
            "{body}"
        );
        let parsed = read(unsafe {
            msime_client_parse_custom_translation_response(body.as_ptr(), body.len())
        });
        assert_eq!(parsed["ok"], true);
        assert!(parsed["value"].is_null(), "{body}");
    }
}

#[test]
fn niutrans_reply_failure_rejects_missing_or_invalid_translation_fields() {
    for body in [
        "{}",
        r#"{"message":"temporarily unavailable"}"#,
        r#"{"tgtText":null}"#,
        r#"{"tgtText":42}"#,
        r#"{"tgtText":false}"#,
        r#"{"tgtText":[]}"#,
        r#"{"tgtText":{"text":"synthetic"}}"#,
    ] {
        assert!(
            unsafe { msime_client_niutrans_translation_reply_failed(body.as_ptr(), body.len()) },
            "{body}"
        );
        let parsed = read(unsafe {
            msime_client_parse_niutrans_translation_response(body.as_ptr(), body.len())
        });
        assert_eq!(parsed["ok"], true);
        assert!(parsed["value"].is_null(), "{body}");
    }
}

#[test]
fn niutrans_reply_failure_preserves_answers_and_error_precedence() {
    for (body, expected) in [
        (r#"{"tgtText":""}"#, Value::Null),
        (r#"{"tgtText":" \n\t"}"#, Value::Null),
        (
            r#"{"tgtText":" synthetic\ngloss ","from":"zh","to":"en"}"#,
            json!("synthetic gloss"),
        ),
    ] {
        assert!(!unsafe {
            msime_client_niutrans_translation_reply_failed(body.as_ptr(), body.len())
        });
        let parsed = read(unsafe {
            msime_client_parse_niutrans_translation_response(body.as_ptr(), body.len())
        });
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["value"], expected);
    }
    for body in [
        r#"{"errorCode":"13001","tgtText":"synthetic"}"#,
        r#"{"errorMsg":"rate limited","tgtText":""}"#,
    ] {
        assert!(unsafe {
            msime_client_niutrans_translation_reply_failed(body.as_ptr(), body.len())
        });
        let parsed = read(unsafe {
            msime_client_parse_niutrans_translation_response(body.as_ptr(), body.len())
        });
        assert!(parsed["value"].is_null());
    }
}

#[test]
fn niutrans_reply_failure_rejects_unbounded_lengths() {
    assert!(unsafe { msime_client_niutrans_translation_reply_failed(b"x".as_ptr(), usize::MAX) });
}

#[test]
fn invalid_buffers_and_commands_return_owned_errors() {
    assert_eq!(
        read(unsafe { msime_client_prepare_host(std::ptr::null(), 0) })["ok"],
        false
    );
    let invalid = br#"{"resources":"relative","state_root":"relative"}"#;
    assert_eq!(
        read(unsafe { msime_client_prepare_host(invalid.as_ptr(), invalid.len()) })["ok"],
        false
    );
    assert_eq!(
        read(unsafe { msime_client_create(std::ptr::null(), 0) })["ok"],
        false
    );
    assert_eq!(read(msime_client_command(0, 999))["ok"], false);
    unsafe { msime_client_string_free(std::ptr::null_mut()) };
}

#[test]
fn incomplete_local_mode_input_shows_the_raw_text_as_a_fallback_space_commits() {
    // Windows' PrepareCandidateList shows the raw composition as a Fallback row whenever a special mode has nothing else, and Space commits it; a bare Y or R prefix included.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("resources")).unwrap();
    std::fs::write(dir.path().join("resources/msime-english.db"), b"fixture").unwrap();
    std::fs::write(
        dir.path().join("resources/msime-japanese.dat"),
        b"synthetic",
    )
    .unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    let only_fallback = |transition: &Value, text: &str| {
        let view = &transition["value"]["view"];
        let candidates = view["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 1, "{view}");
        assert_eq!(candidates[0]["text"], text, "{view}");
        assert_eq!(candidates[0]["source"], 9, "{view}");
    };
    let typed = |text: &[u8]| {
        let mut last = Value::Null;
        for byte in text {
            last = read(msime_client_character(
                handle,
                *byte,
                byte.is_ascii_uppercase(),
            ));
        }
        last
    };

    only_fallback(&typed(b"Y"), "Y");
    let committed = read(msime_client_command(handle, 1));
    assert_eq!(committed["value"]["commit"], "Y");
    assert!(committed["value"]["view"]["candidates"]
        .as_array()
        .unwrap()
        .is_empty());

    only_fallback(&typed(b"Kzzz"), "Kzzz");
    read(msime_client_command(handle, 3));

    only_fallback(&typed(b"U+"), "U+");
    read(msime_client_command(handle, 3));

    only_fallback(&typed(b"Txin"), "Txin");
    assert_eq!(
        read(msime_client_command(handle, 1))["value"]["commit"],
        "Txin"
    );

    only_fallback(&typed(b"R"), "R");
    assert_eq!(
        read(msime_client_command(handle, 1))["value"]["commit"],
        "R"
    );
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

#[test]
fn candidate_page_edge_commands_reach_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    for byte in b"nihao" {
        read(msime_client_character(handle, *byte, false));
    }
    let first = read(msime_client_command(handle, 104));
    assert_eq!(first["value"]["handled"], false);
    assert!(first["value"]["view"]["candidates"]
        .as_array()
        .unwrap()
        .is_empty());
    let last = read(msime_client_command(handle, 105));
    assert_eq!(last["value"]["handled"], false);
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

#[test]
fn complete_candidate_abi_keeps_view_paged_and_selects_a_later_entry() {
    let dir = tempfile::tempdir().unwrap();
    let handle = test_host(dir.path());
    read(msime_client_focus(handle, true));
    read(msime_client_character(handle, b'T', true));
    read(msime_client_character(handle, b'r', false));
    let transition = read(msime_client_character(handle, b'q', false));
    let view = &transition["value"]["view"];
    let generation = view["generation"].as_u64().unwrap();
    let visible_count = view["candidates"].as_array().unwrap().len();
    assert!(visible_count > 0);

    let complete = read(msime_client_all_candidates(handle));
    assert_eq!(complete["ok"], true);
    assert_eq!(complete["value"]["session"], handle);
    assert_eq!(complete["value"]["generation"], generation);
    assert_eq!(complete["value"]["preedit"], "Trq");
    let complete_count = complete["value"]["candidates"].as_array().unwrap().len();
    assert!(complete_count > visible_count);
    let later = complete["value"]["candidates"][visible_count]["id"]["index"]
        .as_u64()
        .unwrap() as usize;

    assert_eq!(
        read(msime_client_select(handle, generation, later))["ok"],
        false
    );
    assert_eq!(
        read(msime_client_select_any_candidate(
            handle,
            generation + 1,
            later
        ))["ok"],
        false
    );
    assert_eq!(
        read(msime_client_select_any_candidate(
            handle,
            generation,
            complete_count
        ))["ok"],
        false
    );
    let selected = read(msime_client_select_any_candidate(handle, generation, later));
    assert_eq!(selected["ok"], true);
    assert_eq!(selected["value"]["handled"], true);
    assert!(selected["value"]["commit"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert_eq!(read(msime_client_destroy(handle))["ok"], true);
}

/// 表情目录请求列出插件符号组：不需要 msime-others.db，没传插件目录时为空，相对路径被拒绝。
#[test]
fn emoji_catalog_lists_plugin_symbol_groups() {
    let directory = tempfile::tempdir().unwrap();
    let pack = directory.path().join("plugins/symbol_set/arrows");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(
        pack.join("plugin.toml"),
        "schema_version = 1\nkind = \"symbol_set\"\nid = \"arrows\"\nname = \"箭头大全\"\nversion = \"1\"\nlicense = \"CC0-1.0\"\n[[groups]]\ntab = \"symbols\"\ntitle = \"箭头\"\nkeywords = \"jiantou\"\nitems = [\"→\", \"←\"]\n[[groups]]\ntab = \"kaomoji\"\ntitle = \"开心\"\nitems = [\"(^_^)\"]\n",
    )
    .unwrap();
    let resources = directory.path().join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    let resources = resources.to_str().unwrap().to_owned();
    let call = |query: Value| {
        let query = query.to_string();
        read(unsafe {
            super::ffi::msime_client_emoji_catalog_request(
                query.as_ptr(),
                query.len(),
                resources.as_ptr(),
                resources.len(),
            )
        })
    };
    let listed = call(json!({
        "list_plugin_symbol_groups": true,
        "plugins": directory.path().join("plugins"),
    }));
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(
        listed["value"]["plugin_symbol_groups"],
        json!([
            {"pack": "arrows", "pack_name": "箭头大全", "tab": "symbols", "title": "箭头", "keywords": "jiantou", "items": ["→", "←"]},
            {"pack": "arrows", "pack_name": "箭头大全", "tab": "kaomoji", "title": "开心", "keywords": "", "items": ["(^_^)"]},
        ])
    );
    assert_eq!(
        call(json!({"list_plugin_symbol_groups": true}))["value"]["plugin_symbol_groups"],
        json!([])
    );
    assert_eq!(
        call(json!({"list_plugin_symbol_groups": true, "plugins": "plugins"}))["ok"],
        false
    );
}

#[test]
fn emoji_catalog_pagination_preserves_legacy_defaults() {
    let legacy: crate::ffi::EmojiCatalogQuery = serde_json::from_str("{}").unwrap();
    assert!(!legacy.cursor);
    assert_eq!(legacy.offset, 0);
    assert_eq!(legacy.panel.limit, 48);
    let page: crate::ffi::EmojiCatalogQuery = serde_json::from_str(
        r#"{"search":"synthetic","category":"symbols","offset":510,"limit":255}"#,
    )
    .unwrap();
    assert_eq!(page.offset, 510);
    assert_eq!(page.panel.search, "synthetic");
    assert_eq!(page.panel.category, "symbols");
    assert_eq!(page.panel.limit, 255);
    assert!(serde_json::from_str::<crate::ffi::EmojiCatalogQuery>(r#"{"offset":-1}"#).is_err());
}

#[test]
fn emoji_catalog_cursor_advances_over_invalid_rows_and_preserves_duplicates() {
    let directory = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(directory.path().join("msime-others.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE emoji(emoji TEXT,category TEXT,keywords TEXT,pinyin TEXT,sort_order INTEGER);
         CREATE TABLE kaomoji_catalog(kaomoji TEXT,keywords TEXT,sort_order INTEGER);
         CREATE TABLE symbol_catalog(symbol TEXT,category TEXT,parent_category TEXT,keywords TEXT,sort_order INTEGER);",
    ).unwrap();
    for (index, text) in [
        None,
        Some(""),
        Some("synthetic-same"),
        Some("synthetic-same"),
        Some("synthetic-tail"),
    ]
    .into_iter()
    .enumerate()
    {
        db.execute(
            "INSERT INTO emoji VALUES (?1,'fixture','match','',?2)",
            rusqlite::params![text, index as i64],
        )
        .unwrap();
        db.execute(
            "INSERT INTO kaomoji_catalog VALUES (?1,'match',?2)",
            rusqlite::params![text, index as i64],
        )
        .unwrap();
        db.execute(
            "INSERT INTO symbol_catalog VALUES (?1,'fixture','fixture','match',?2)",
            rusqlite::params![text, index as i64],
        )
        .unwrap();
    }
    let resources = directory.path().to_str().unwrap().as_bytes();
    let request = |category: &str, offset: usize, limit: u8, cursor: bool| {
        let query = serde_json::to_vec(&json!({
            "category": category, "offset": offset, "limit": limit, "cursor": cursor,
        }))
        .unwrap();
        read(unsafe {
            msime_client_emoji_catalog_request(
                query.as_ptr(),
                query.len(),
                resources.as_ptr(),
                resources.len(),
            )
        })
    };
    for category in ["", "kaomoji", "symbols"] {
        let empty = request(category, 0, 2, true);
        assert_eq!(empty["ok"], true);
        assert_eq!(
            empty["value"],
            json!({"items":[], "next_offset":2, "complete":false})
        );
        let duplicates = request(category, 2, 2, true);
        assert_eq!(duplicates["value"]["items"].as_array().unwrap().len(), 2);
        assert_eq!(duplicates["value"]["next_offset"], 4);
        assert_eq!(duplicates["value"]["complete"], false);
        let tail = request(category, 4, 2, true);
        assert_eq!(tail["value"]["items"][0]["text"], "synthetic-tail");
        assert_eq!(tail["value"]["next_offset"], 5);
        assert_eq!(tail["value"]["complete"], true);
        let exact = request(category, 4, 1, true);
        assert_eq!(exact["value"]["complete"], false);
        assert_eq!(
            request(category, 5, 1, true)["value"],
            json!({"items":[], "next_offset":5, "complete":true})
        );
        let legacy = request(category, 2, 2, false);
        assert_eq!(legacy["value"]["items"].as_array().unwrap().len(), 1);
        assert!(legacy["value"].get("complete").is_none());
        assert_eq!(request(category, 0, 0, true)["ok"], false);
    }
}

#[test]
fn emoji_catalog_cursor_skips_invalid_groups_without_stalling() {
    let directory = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(directory.path().join("msime-others.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE emoji(emoji TEXT,category TEXT,keywords TEXT,pinyin TEXT,sort_order INTEGER);
         INSERT INTO emoji VALUES ('synthetic-invalid',NULL,'','',0);
         INSERT INTO emoji VALUES ('synthetic-invalid','','','',1);
         INSERT INTO emoji VALUES ('synthetic-valid','fixture','','',2);",
    )
    .unwrap();
    let resources = directory.path().to_str().unwrap().as_bytes();
    let request = |offset: usize| {
        let query = serde_json::to_vec(&json!({"cursor":true,"offset":offset,"limit":2})).unwrap();
        read(unsafe {
            msime_client_emoji_catalog_request(
                query.as_ptr(),
                query.len(),
                resources.as_ptr(),
                resources.len(),
            )
        })
    };
    assert_eq!(
        request(0)["value"],
        json!({"items":[],"next_offset":2,"complete":false})
    );
    let tail = request(2);
    assert_eq!(tail["value"]["items"][0]["text"], "synthetic-valid");
    assert_eq!(tail["value"]["complete"], true);
}

#[test]
fn emoji_catalog_ffi_reads_beyond_first_page() {
    let directory = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(directory.path().join("msime-others.db")).unwrap();
    db.execute_batch(
        "CREATE TABLE emoji(emoji TEXT,category TEXT,keywords TEXT,pinyin TEXT,sort_order INTEGER);
         CREATE TABLE kaomoji_catalog(kaomoji TEXT,keywords TEXT,sort_order INTEGER);
         CREATE TABLE symbol_catalog(symbol TEXT,category TEXT,parent_category TEXT,keywords TEXT,sort_order INTEGER);",
    ).unwrap();
    for index in 0..520 {
        let text = format!("synthetic-{index}");
        db.execute(
            "INSERT INTO emoji VALUES (?1,'fixture','match','',?2)",
            rusqlite::params![text, index],
        )
        .unwrap();
        db.execute(
            "INSERT INTO kaomoji_catalog VALUES (?1,'match',?2)",
            rusqlite::params![text, index],
        )
        .unwrap();
        db.execute(
            "INSERT INTO symbol_catalog VALUES (?1,'fixture','fixture','match',?2)",
            rusqlite::params![text, index],
        )
        .unwrap();
    }
    let resources = directory.path().to_str().unwrap().as_bytes();
    let request = |category: &str, offset: u64, limit: u8| {
        let query = serde_json::to_vec(
            &json!({"category":category,"search":"match","offset":offset,"limit":limit}),
        )
        .unwrap();
        read(unsafe {
            msime_client_emoji_catalog_request(
                query.as_ptr(),
                query.len(),
                resources.as_ptr(),
                resources.len(),
            )
        })
    };
    for category in ["", "kaomoji", "symbols"] {
        for (offset, count) in [(0, 255), (255, 255), (510, 10), (765, 0)] {
            let page = request(category, offset, 255);
            assert_eq!(page["ok"], true);
            assert_eq!(page["value"]["items"].as_array().unwrap().len(), count);
            if count > 0 {
                assert_eq!(
                    page["value"]["items"][0]["text"],
                    format!("synthetic-{offset}")
                );
            }
        }
    }
    db.execute(
        "UPDATE emoji SET emoji='synthetic-0' WHERE sort_order=1",
        [],
    )
    .unwrap();
    assert_eq!(
        request("", 0, 2)["value"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        request("", 2, 2)["value"]["items"][0]["text"],
        "synthetic-2"
    );
    assert_eq!(request("", 0, 0)["ok"], false);
    assert_eq!(request("", u64::MAX, 255)["ok"], false);
    assert_eq!(request("", u64::from(u32::MAX), 255)["ok"], true);
}


/// `empty_host_options` with an empty main dictionary, which the Engine needs before it stores a pinyin word (`ni'…` lands in `tbl_2_n`) or a quick phrase.
pub(crate) fn host_options_with_main_dictionary(root: &Path) -> Value {
    for name in ["resources", "user", "cache", "dictionaries"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    for name in ["resources", "dictionaries"] {
        rusqlite::Connection::open(root.join(name).join("msime-pinyin.db"))
            .unwrap()
            .execute_batch(
                "CREATE TABLE tbl_2_n(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                 CREATE TABLE tbl_3_a(key TEXT,jp TEXT,value TEXT,weight INTEGER);
                 CREATE TABLE wubi86(key TEXT,value TEXT,weight INTEGER);
                 CREATE TABLE quick_parases(key TEXT,value TEXT,weight INTEGER);
                 CREATE INDEX idx_quick_parases_key_weight ON quick_parases(key,weight DESC);",
            )
            .unwrap();
    }
    empty_host_options(root)
}

/// A session whose preferences directory is `<root>/state`, so its plugins root is `<root>/state/plugins`, with the repository's built-in sound packs.
fn plugin_host(root: &std::path::Path, preferences: Preferences) -> u64 {
    let path = |name| {
        let path = root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let sound_packs =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/sound-packs");
    let options = json!({ "api_version": 1, "resources": path("resources"), "user_data": path("user"), "cache": path("cache"), "dictionaries": path("dictionaries"), "preferences": preferences, "preferences_directory": path("state"), "sound_packs": sound_packs }).to_string();
    let created = read(unsafe { msime_client_create(options.as_ptr(), options.len()) });
    assert_eq!(created["ok"], true, "{created}");
    created["value"]["session"].as_u64().unwrap()
}

/// 一个只有一条词条、连接矩阵为 1×1 的 MSJPDT1 日文模型，布局同 `msime_engine` 里 `japanese::decoder::test_model::bytes`（那个只在 engine 自己的测试里可见）。
pub(crate) fn single_lemma_japanese_model(reading: &str, surface: &str, cost: i32) -> Vec<u8> {
    let strings = [reading.as_bytes(), surface.as_bytes()].concat();
    let mut token = Vec::new();
    token.extend_from_slice(&0u32.to_le_bytes());
    token.extend_from_slice(&(reading.len() as u16).to_le_bytes());
    token.extend_from_slice(&(reading.len() as u32).to_le_bytes());
    token.extend_from_slice(&(surface.len() as u16).to_le_bytes());
    token.extend_from_slice(&0u16.to_le_bytes());
    token.extend_from_slice(&0u16.to_le_bytes());
    token.extend_from_slice(&cost.to_le_bytes());
    let token_offset = 56u64;
    let connection_offset = token_offset + token.len() as u64;
    let string_offset = connection_offset + 2;
    let mut file = b"MSJPDT1\0".to_vec();
    for value in [1u32, 1, 1, 0] {
        file.extend_from_slice(&value.to_le_bytes());
    }
    for value in [
        token_offset,
        connection_offset,
        string_offset,
        strings.len() as u64,
    ] {
        file.extend_from_slice(&value.to_le_bytes());
    }
    file.extend_from_slice(&token);
    file.extend_from_slice(&0i16.to_le_bytes());
    file.extend_from_slice(&strings);
    file
}

/// 合成的桌面词库锁：`msime-pinyin.db` 与 `msime-english.db` 是可以复制进代次的 SQLite 小库，另有日文词典与两份 Mozc 许可文本这一组 macOS 按需下载的文件。文件写在 `resources` 里，清单按实际内容计算长度与 SHA-256。
pub(crate) fn synthetic_desktop_lock(resources: &Path) -> ResourceSet {
    std::fs::create_dir_all(resources).unwrap();
    for name in ["msime-pinyin.db", "msime-english.db"] {
        rusqlite::Connection::open(resources.join(name))
            .unwrap()
            .execute_batch("CREATE TABLE fixture(value TEXT);")
            .unwrap();
    }
    std::fs::write(resources.join("msime-japanese.dat"), b"japanese").unwrap();
    std::fs::write(
        resources.join("msime-mozc_dictionary_oss_README.txt"),
        b"readme",
    )
    .unwrap();
    std::fs::write(resources.join("msime-mozc_LICENSE.txt"), b"license").unwrap();
    let artifacts = [
        "msime-pinyin.db",
        "msime-english.db",
        "msime-japanese.dat",
        "msime-mozc_dictionary_oss_README.txt",
        "msime-mozc_LICENSE.txt",
    ]
    .into_iter()
    .map(|name| {
        let bytes = std::fs::read(resources.join(name)).unwrap();
        msime_client_core::resources::Artifact {
            name: name.into(),
            url: format!("https://example.invalid/{name}"),
            sha256: hex::encode(Sha256::digest(&bytes)),
            size: bytes.len() as u64,
        }
    })
    .collect();
    ResourceSet {
        source_commit: "a".repeat(40),
        artifacts,
    }
}

fn empty_host_options(root: &Path) -> Value {
    json!({
        "api_version": 1,
        "resources": root.join("resources"),
        "user_data": root.join("user"),
        "cache": root.join("cache"),
        "dictionaries": root.join("dictionaries"),
        "preferences": Preferences::default(),
        "preferences_directory": root,
    })
}
