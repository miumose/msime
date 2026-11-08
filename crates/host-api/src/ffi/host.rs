//! Host configuration, preferences, the skin catalog and clipboard history.
//!
//! Part of the C ABI; see the parent module for what these shims guarantee.

use crate::*;
use msime_client_core::is_bounded_text;

/// Candidate and commit text normally stays far below this bound. Keep the direct conversion ABI
/// bounded as well so a malformed native length cannot make it scan an unbounded buffer or allocate
/// an arbitrarily large converted string.
const MAX_TRADITIONAL_CONVERSION_BYTES: usize = 1 << 20;


#[no_mangle]
pub extern "C" fn msime_client_abi_version() -> u32 {
    3
}

/// Convert Simplified Chinese text to Traditional Chinese with the shared OpenCC `s2t` tables.
///
/// Returns the converted text itself rather than the standard JSON response: hosts call this for every candidate on a page and every commit, and wrapping each string in a document only to parse it back out would put a JSON round trip on the typing path. Returns null for a null pointer, invalid UTF-8 or an interior NUL, so the caller keeps its own text.
/// # Safety
/// `text` points to `length` readable bytes. The returned string must be released with `msime_client_string_free`.
#[no_mangle]
pub unsafe extern "C" fn msime_client_simplified_to_traditional(
    text: *const u8,
    length: usize,
) -> *mut c_char {
    if text.is_null() || length > MAX_TRADITIONAL_CONVERSION_BYTES {
        return std::ptr::null_mut();
    }
    // SAFETY: guaranteed by the documented caller contract.
    let bytes = unsafe { std::slice::from_raw_parts(text, length) };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return std::ptr::null_mut();
    };
    let converted = msime_client_core::chinese_conversion::simplified_to_traditional(text);
    CString::new(converted).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// 与 `msime_client_simplified_to_traditional` 相同，但按 `variant` 选 OpenCC 配置：0 `s2t`、1 `s2tw`、2 `s2twp`，即偏好 `traditional_chinese_variant` 的三个取值。未知编号和其它非法输入一样返回 null，调用方保留原文。
/// # Safety
/// `text` points to `length` readable bytes. The returned string must be released with `msime_client_string_free`.
#[no_mangle]
pub unsafe extern "C" fn msime_client_convert_to_traditional(
    text: *const u8,
    length: usize,
    variant: u32,
) -> *mut c_char {
    let Some(variant) =
        msime_client_core::chinese_conversion::TraditionalVariant::from_code(variant)
    else {
        return std::ptr::null_mut();
    };
    if text.is_null() || length > MAX_TRADITIONAL_CONVERSION_BYTES {
        return std::ptr::null_mut();
    }
    // SAFETY: guaranteed by the documented caller contract.
    let bytes = unsafe { std::slice::from_raw_parts(text, length) };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return std::ptr::null_mut();
    };
    let converted = msime_client_core::chinese_conversion::convert_to_traditional(text, variant);
    CString::new(converted).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Resolve display font families using the same adapter as the shared preview.
/// # Safety
/// `value` points to `length` readable bytes containing a JSON string array.
/// The returned response must be released with `msime_client_string_free`.
#[no_mangle]
pub unsafe extern "C" fn msime_client_resolve_font_families(
    value: *const u8,
    length: usize,
) -> *mut c_char {
    response(|| {
        if value.is_null() || length > 32 * 1024 {
            return Err("font_family".into());
        }
        // SAFETY: guaranteed by the caller contract; size checked above.
        let bytes = unsafe { std::slice::from_raw_parts(value, length) };
        let names: Vec<String> = serde_json::from_slice(bytes).map_err(|_| "font_family")?;
        let resolved = system_fonts::resolve_css_families(names).map_err(str::to_owned)?;
        Ok(json!(resolved))
    })
}

/// The installed font family names, from the same catalog the shared app's 刷新字体列表 reads
/// (`list_font_families` in the Tauri shell): sorted, without repeats, each a bounded display name.
/// Fails with `unsupported` on a host without a catalog and `font_catalog` / `font_catalog_limit`
/// when enumeration fails or exceeds its bound. Enumerates the system's fonts, so call it off a
/// UI thread.
#[no_mangle]
pub extern "C" fn msime_client_list_font_families() -> *mut c_char {
    response(|| {
        system_fonts::list()
            .map(|names| json!(names))
            .map_err(str::to_owned)
    })
}

/// Resolve a shared surface route so a native host launches the shared shell by
/// name instead of hardcoding window labels. Returns the canonical route plus
/// the panel label, query and geometry, or an error for an unknown route.
/// # Safety
/// `value` points to `length` readable UTF-8 bytes. Null is rejected.
#[no_mangle]
pub unsafe extern "C" fn msime_client_parse_surface_route(
    value: *const u8,
    length: usize,
) -> *mut c_char {
    response(|| {
        if value.is_null() || length > 1024 {
            return Err("invalid surface route buffer".into());
        }
        // SAFETY: guaranteed by the documented caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(value, length) };
        let text = std::str::from_utf8(bytes).map_err(|_| "invalid surface route encoding")?;
        let route = SurfaceRoute::parse(text).map_err(|e| e.to_string())?;
        let mut document = json!({ "route": route.as_arg(), "surface": route });
        if let Some(panel) = route.panel() {
            document["panel"] = json!({
                "label": panel.label,
                "query": panel.query,
                "title": panel.title,
                "width": panel.width,
                "height": panel.height,
            });
        }
        Ok(document)
    })
}

/// Describe what the named host can do, so the shared UI renders from injected
/// capabilities instead of sniffing the user agent.
/// # Safety
/// `platform` points to `length` readable UTF-8 bytes. Null is rejected.
#[no_mangle]
pub unsafe extern "C" fn msime_client_host_capabilities(
    platform: *const u8,
    length: usize,
) -> *mut c_char {
    response(|| {
        if platform.is_null() || length > 64 {
            return Err("invalid host platform buffer".into());
        }
        // SAFETY: guaranteed by the documented caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(platform, length) };
        let text = std::str::from_utf8(bytes).map_err(|_| "invalid host platform encoding")?;
        let platform = HostPlatform::parse(text).map_err(|e| e.to_string())?;
        serde_json::to_value(HostCapabilities::for_platform(platform)).map_err(|e| e.to_string())
    })
}

/// Verify packaged resources and prepare an isolated new host. No live sessions
/// may use the state root while this runs. Returns a HostOptions object.
/// # Safety
/// `options` points to `length` readable UTF-8 JSON bytes. Null is rejected.
#[no_mangle]
pub unsafe extern "C" fn msime_client_prepare_host(
    options: *const u8,
    length: usize,
) -> *mut c_char {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Bootstrap {
        resources: String,
        state_root: String,
        /// 版本 id，缺省是 full；见 `prepare_host_configuration_for_edition`。
        #[serde(default)]
        edition: Option<String>,
        /// 已发布的 runtime-options.json；给了时把这次准备的代次写进去，见 [`publish_prepared_dictionaries`]。
        #[serde(default)]
        runtime_options: Option<String>,
        /// 宿主对 `state_root` 没有写权限（iOS 键盘没有完全访问，#10945）：只读地准备，见 `read_only_state`。不能与 `runtime_options` 同时给。
        #[serde(default)]
        read_only_state: bool,
        /// `read_only_state` 时必填：宿主自己能写、不在 `state_root` 里的目录（扩展自己的容器），放资源校验标记、缓存和只读布局的用户目录。
        #[serde(default)]
        private_root: Option<String>,
    }
    response(|| {
        if options.is_null() || length > 16384 {
            return Err("invalid bootstrap buffer".into());
        }
        // SAFETY: guaranteed by the caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(options, length) };
        let options: Bootstrap =
            serde_json::from_slice(bytes).map_err(|_| "invalid bootstrap document")?;
        let resources = std::path::Path::new(&options.resources);
        let state = std::path::Path::new(&options.state_root);
        let published = options.runtime_options.as_deref().map(std::path::Path::new);
        if !resources.is_absolute()
            || !state.is_absolute()
            || published.is_some_and(|path| !path.is_absolute())
        {
            return Err("bootstrap paths must be absolute".into());
        }
        let edition = match options.edition.as_deref() {
            None => msime_client_core::edition::Edition::full(),
            Some(id) => msime_client_core::edition::Edition::by_id(id).ok_or("unknown edition")?,
        };
        if options.read_only_state {
            let private = options
                .private_root
                .as_deref()
                .map(std::path::Path::new)
                .filter(|path| path.is_absolute())
                .ok_or("a read-only host needs an absolute private root")?;
            if published.is_some() {
                return Err("a read-only host cannot publish runtime options".into());
            }
            let document =
                crate::read_only_state::prepare_read_only_host(resources, state, private, edition)
                    .map_err(|e| e.to_string())?;
            return serde_json::from_str(&document).map_err(|e| e.to_string());
        }
        if options.private_root.is_some() {
            return Err("a private root is only for a read-only host".into());
        }
        let document = prepare_host_configuration_for_edition(resources, state, edition)
            .map_err(|e| e.to_string())?;
        let document: Value = serde_json::from_str(&document).map_err(|e| e.to_string())?;
        // 发布失败不挡住宿主启动：会话照样用这份文档，设置窗口和 TIP 留在旧代次上，下次启动再试。
        if let Some(path) = published {
            if let Err(error) = publish_prepared_dictionaries(path, &document) {
                crate::diagnostics::report(
                    "runtime options generation not published",
                    &error.to_string(),
                );
            }
        }
        Ok(document)
    })
}

/// Re-prepare a published runtime options file whose working dictionaries belong to an older resource generation, as after a package upgrade, and record the language dictionaries installed beside the resources (see [`refresh_host_options_with_language_dictionaries`]). Returns whether the file was rewritten. Only the input method host calls this, before creating any session from that file.
/// # Safety
/// `path` points to `length` readable UTF-8 bytes naming an absolute file. Null is rejected.
#[no_mangle]
pub unsafe extern "C" fn msime_client_refresh_host(path: *const u8, length: usize) -> *mut c_char {
    response(|| {
        if path.is_null() || length > 4096 {
            return Err("invalid options path buffer".into());
        }
        // SAFETY: guaranteed by the caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(path, length) };
        let path = std::path::Path::new(
            std::str::from_utf8(bytes).map_err(|_| "invalid options path encoding")?,
        );
        if !path.is_absolute() {
            return Err("options path must be absolute".into());
        }
        refresh_host_options_with_language_dictionaries(path)
            .map(Value::Bool)
            .map_err(|e| e.to_string())
    })
}


/// The shared preference defaults, as the document a host would have to produce.
///
/// A host that patches one key into a nested preference object needs the rest of
/// that object's fields, because the nested structures require all of them - only
/// the object as a whole is optional. Writing those defaults into a platform host
/// would put a second copy of this contract in another language, so they are
/// published here instead.
#[no_mangle]
pub extern "C" fn msime_client_default_preferences() -> *mut c_char {
    response(|| {
        serde_json::to_value(msime_client_core::preferences::Preferences::default())
            .map_err(|e| e.to_string())
    })
}

/// 触控板设置控件的默认值和范围，宿主只负责展示；结果用 `msime_client_string_free` 释放。
#[no_mangle]
pub extern "C" fn msime_client_text_trackpad_options() -> *mut c_char {
    use msime_client_core::preferences::TextTrackpadPreferences as Trackpad;
    response(|| {
        Ok(json!({
            "defaults": Trackpad::default(),
            "horizontal_dp": {"minimum": Trackpad::HORIZONTAL_MIN_DP, "maximum": Trackpad::HORIZONTAL_MAX_DP},
            "vertical_dp": {"minimum": Trackpad::VERTICAL_MIN_DP, "maximum": Trackpad::VERTICAL_MAX_DP},
        }))
    })
}

/// 本库编译到的宿主平台。主题目录按它列出主题、会话按它截断每页候选数：这些接口不带平台参数，而每个原生宿主链接的都是为自己的平台编出的库。
pub(crate) fn compiled_platform() -> HostPlatform {
    if cfg!(target_os = "ios") {
        HostPlatform::Ios
    } else if cfg!(target_os = "android") {
        HostPlatform::Android
    } else if cfg!(target_env = "ohos") {
        HostPlatform::Harmony
    } else if cfg!(target_os = "macos") {
        HostPlatform::Macos
    } else if cfg!(windows) {
        HostPlatform::Windows
    } else {
        HostPlatform::Linux
    }
}


/// 「重置所有设置」：在偏好锁里读出当前文档，换成本存储所属版本的默认偏好（服务凭据和 `fuzzy_pinyin.seeded` 保留，见 `Preferences::restored_to_defaults_for`），再按 `expected_revision` 比较并交换写回，返回新的快照。修订号不符时以 `preferences changed; reload before saving` 失败，什么也不写。词库、统计和剪贴板历史不受影响；默认关闭剪贴板历史时与保存偏好一样清空已存的历史。
/// # Safety
/// `directory` must point to `length` readable bytes. Null is rejected.
/// The returned response must be released with `msime_client_string_free`.
#[no_mangle]
pub unsafe extern "C" fn msime_client_restore_default_preferences(
    directory: *const u8,
    length: usize,
    expected_revision: u64,
) -> *mut c_char {
    response(|| {
        if directory.is_null() || length > 16384 {
            return Err("invalid preferences directory buffer".into());
        }
        // SAFETY: guaranteed by the documented caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(directory, length) };
        let directory = super::parse_absolute_path(
            bytes,
            "invalid preferences directory encoding",
            "preferences directory must be absolute",
        )?;
        let store = PreferencesStore::new(directory);
        let restored = store
            .restore_defaults(expected_revision)
            .map_err(|e| e.to_string())?;
        if !restored.preferences.clipboard_history {
            store
                .clear_disabled_clipboard_history()
                .map_err(|e| e.to_string())?;
        }
        serde_json::to_value(restored).map_err(|e| e.to_string())
    })
}

/// 「导出设置」：读出偏好目录里的当前偏好，返回设置文件的全文（字符串，见 `msime_client_core::settings_document`）。语音、AI 辅助和翻译服务的配置与密钥、诊断日志、使用统计和剪贴板历史开关不写进文件。宿主负责让用户选位置并写盘。
/// # Safety
/// `directory` 必须指向 `length` 个可读字节，空指针被拒绝。
/// 返回的响应必须交给 `msime_client_string_free` 释放。
#[no_mangle]
pub unsafe extern "C" fn msime_client_export_settings(
    directory: *const u8,
    length: usize,
) -> *mut c_char {
    response(|| {
        if directory.is_null() || length > 16384 {
            return Err("invalid preferences directory buffer".into());
        }
        // SAFETY: 由上面写明的调用方契约保证。
        let bytes = unsafe { std::slice::from_raw_parts(directory, length) };
        let directory = super::parse_absolute_path(
            bytes,
            "invalid preferences directory encoding",
            "preferences directory must be absolute",
        )?;
        let snapshot = PreferencesStore::new(directory)
            .load()
            .map_err(|e| e.to_string())?;
        msime_client_core::settings_document::export_document(&snapshot.preferences)
            .map(Value::String)
            .map_err(|e| e.to_string())
    })
}

/// 「导入设置」：把用户选的设置文件换算成偏好（本机的服务配置与密钥、诊断日志、使用统计和剪贴板历史开关保留，按本目录所属的版本收窄输入方案），再按 `expected_revision` 比较并交换写回，返回新的快照。剪贴板历史开关不随文件变，所以导入不会像保存偏好那样清空已存的剪贴板历史，保存成功就是整个导入成功。失败时的错误是 `settings_document_invalid`（不是设置文件）、`settings_document_macos`（macOS 原生设置窗口导出的文件）、`settings_document_unsupported`（更新版本的文件或读不懂的设置）、`settings_conflict`（设置已在别处更新）之一，或者存储错误的说明；失败时什么也不写。
/// # Safety
/// `directory` 必须指向 `directory_length` 个可读字节，`document` 必须指向 `document_length` 个可读字节，空指针被拒绝。
/// 返回的响应必须交给 `msime_client_string_free` 释放。
#[no_mangle]
pub unsafe extern "C" fn msime_client_import_settings(
    directory: *const u8,
    directory_length: usize,
    expected_revision: u64,
    document: *const u8,
    document_length: usize,
) -> *mut c_char {
    use msime_client_core::settings_document;
    response(|| {
        if directory.is_null() || document.is_null() || directory_length > 16384 {
            return Err("invalid preferences directory buffer".into());
        }
        if document_length > settings_document::MAX_DOCUMENT_BYTES {
            return Err(
                settings_document::SettingsDocumentError::NotSettingsDocument
                    .code()
                    .into(),
            );
        }
        // SAFETY: 由上面写明的调用方契约保证。
        let directory_bytes = unsafe { std::slice::from_raw_parts(directory, directory_length) };
        let directory = super::parse_absolute_path(
            directory_bytes,
            "invalid preferences directory encoding",
            "preferences directory must be absolute",
        )?;
        // SAFETY: 由上面写明的调用方契约保证。
        let document = unsafe { std::slice::from_raw_parts(document, document_length) };
        let store = PreferencesStore::new(directory);
        let current = store.load().map_err(|e| e.to_string())?;
        let imported =
            settings_document::import_document(document, &current.preferences, store.edition())
                .map_err(|error| error.code().to_owned())?;
        let saved = store
            .save(expected_revision, imported)
            .map_err(|error| match error {
                msime_client_core::preferences::PreferencesError::Conflict => {
                    "settings_conflict".to_owned()
                }
                error => error.to_string(),
            })?;
        serde_json::to_value(saved).map_err(|e| e.to_string())
    })
}

/// Per-key double-pinyin hint text for one profile, read out of the Engine's own profile tables.
///
/// A touch keyboard labels its letter keys with the units they carry, and a face that keeps its own copy of that keymap drifts from the scheme the session actually runs. The hints depend only on the profile, not on session state, so this takes no handle. The reserved name `cantonese` gives the Cantonese shuangpin face, read out of the same table its encoder uses. An unknown name yields an empty object rather than the default profile's hints: labelling the keys with a scheme the session is not running is worse than labelling nothing.
/// # Safety
/// `profile` points to `length` readable UTF-8 bytes. Null is rejected.
/// The returned response must be released with `msime_client_string_free`.
#[no_mangle]
pub unsafe extern "C" fn msime_client_shuangpin_key_hints(
    profile: *const u8,
    length: usize,
) -> *mut c_char {
    response(|| {
        if profile.is_null() || length > 64 {
            return Err("invalid shuangpin profile buffer".into());
        }
        // SAFETY: guaranteed by the documented caller contract; size checked above.
        let bytes = unsafe { std::slice::from_raw_parts(profile, length) };
        let name = std::str::from_utf8(bytes).map_err(|_| "invalid shuangpin profile encoding")?;
        let entries = msime_engine::host::shuangpin_key_hints(name);
        let mut hints = serde_json::Map::with_capacity(entries.len());
        hints.extend(
            entries
                .into_iter()
                .map(|entry| (entry.key, serde_json::Value::String(entry.hint))),
        );
        Ok(serde_json::Value::Object(hints))
    })
}

/// The double-pinyin codes of the whole zero-initial syllables for one profile, read out of the Engine's own profile tables, as a JSON object such as `{"a":"aa","ang":"ah",...}`.
///
/// A keymap panel shows these next to the key face, and like the key hints they depend only on the profile, so this takes no handle. An unknown name yields an empty object.
/// # Safety
/// `profile` points to `length` readable UTF-8 bytes. Null is rejected.
/// The returned response must be released with `msime_client_string_free`.
#[no_mangle]
pub unsafe extern "C" fn msime_client_shuangpin_zero_initials(
    profile: *const u8,
    length: usize,
) -> *mut c_char {
    response(|| {
        if profile.is_null() || length > 64 {
            return Err("invalid shuangpin profile buffer".into());
        }
        // SAFETY: guaranteed by the documented caller contract; size checked above.
        let bytes = unsafe { std::slice::from_raw_parts(profile, length) };
        let name = std::str::from_utf8(bytes).map_err(|_| "invalid shuangpin profile encoding")?;
        let entries = msime_engine::host::shuangpin_zero_initials(name);
        let mut codes = serde_json::Map::with_capacity(entries.len());
        codes.extend(entries.into_iter().map(|(syllable, code)| {
            (
                syllable.to_string(),
                serde_json::Value::String(code.to_string()),
            )
        }));
        Ok(serde_json::Value::Object(codes))
    })
}

/// 进程里 `msime_client_load_preferences` 最近读到的一份偏好，连同它所在的目录：Android 键盘显示期间每秒在工作线程上读一次，偏好文件没变时不再解析和校验，见 [`PreferencesStore::load_reusing`]。
static LOADED_PREFERENCES: Mutex<
    Option<(String, msime_client_core::preferences::LoadedPreferences)>,
> = Mutex::new(None);

/// Load the shared store on a worker thread; no session handle is accessed.
/// # Safety
/// `directory` points to `length` readable UTF-8 bytes. Null is rejected.
#[no_mangle]
pub unsafe extern "C" fn msime_client_load_preferences(
    directory: *const u8,
    length: usize,
) -> *mut c_char {
    response(|| {
        if directory.is_null() || length > 16384 {
            return Err("invalid preferences directory buffer".into());
        }
        // SAFETY: guaranteed by the documented caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(directory, length) };
        let directory = super::parse_absolute_path(
            bytes,
            "invalid preferences directory encoding",
            "preferences directory must be absolute",
        )?;
        // 读文件时不持这把锁：同时来的另一次读拿不到上一份，照常整读一遍。
        let previous = LOADED_PREFERENCES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .filter(|(cached, _)| cached == directory)
            .map(|(_, loaded)| loaded);
        let loaded = PreferencesStore::new(directory)
            .load_reusing(previous)
            .map_err(|e| e.to_string())?;
        let value = serde_json::to_value(&loaded.snapshot).map_err(|e| e.to_string())?;
        *LOADED_PREFERENCES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((directory.to_owned(), loaded));
        Ok(value)
    })
}

/// Android 旧布局迁移：在偏好锁内核对原始字段与修订号。
/// # Safety
/// `request` 指向最多 64 KiB 的可读 JSON；返回值通过 `msime_client_string_free` 释放。
#[no_mangle]
pub unsafe extern "C" fn msime_client_migrate_android_key_layout(
    request: *const u8,
    length: usize,
) -> *mut c_char {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        directory: String,
        revision: u64,
        long_space: bool,
        number_row: bool,
        /// 本机旧键里用户自定义的九键左侧符号表；缺省或空表表示不迁。
        #[serde(default)]
        nine_key_symbols: Vec<String>,
        /// 本机旧键里用户自定义的九键数字键盘左侧符号表；缺省或空表表示不迁。
        #[serde(default)]
        nine_key_digit_symbols: Vec<String>,
        /// 本机旧设置 `platform.android.nine_key_swipe` 的原值；本机没有这个键时不带。
        #[serde(default)]
        nine_key_swipe: Option<String>,
    }
    response(|| {
        if request.is_null() || length > 65536 {
            return Err("invalid layout migration request".into());
        }
        // SAFETY: 调用方保证缓冲区可读，上方已经检查长度。
        let request: Request =
            serde_json::from_slice(unsafe { std::slice::from_raw_parts(request, length) })
                .map_err(|_| "invalid layout migration request")?;
        let directory = super::parse_absolute_path(
            request.directory.as_bytes(),
            "invalid preferences directory encoding",
            "preferences directory must be absolute",
        )?;
        let saved = PreferencesStore::new(directory)
            .migrate_android_key_layout(
                request.revision,
                request.long_space,
                request.number_row,
                &request.nine_key_symbols,
                &request.nine_key_digit_symbols,
                // 不认识的旧值（不会出现，旧设置只存过 off、up、down）当作没有旧值，不迁。
                request
                    .nine_key_swipe
                    .as_deref()
                    .and_then(msime_client_core::preferences::TouchNineKeySwipeDigits::parse)
                    .unwrap_or_default(),
            )
            .map_err(|error| error.to_string())?;
        serde_json::to_value(saved).map_err(|error| error.to_string())
    })
}

/// 可重绑切换热键（`keybindings.toggle_english_candidates`、`toggle_full_width`、
/// `toggle_punctuation`、`toggle_voice_input`）按共享 `KeyChord` 契约解码：每个键给出 `null`（不绑定）或
/// `{"key": 主键规范名, "ctrl", "alt", "shift", "meta"}`，宿主按规范名认键，
/// 写法解析只在 Rust 一侧。keybindings 缺键或值为 null 时按出厂值。
/// # Safety
/// `request` 指向 `length` 字节的 JSON `{"keybindings": object}`，最多 64 KiB；
/// keybindings 不是表时返回错误。结果用 `msime_client_string_free` 释放。
#[no_mangle]
pub unsafe extern "C" fn msime_client_toggle_hotkey_chords(
    request: *const u8,
    length: usize,
) -> *mut c_char {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        #[serde(default)]
        keybindings: Value,
    }
    response(|| {
        if request.is_null() || length > 64 * 1024 {
            return Err("invalid toggle hotkey request buffer".into());
        }
        // SAFETY: 调用方保证缓冲区可读，上方已检查长度上限。
        let bytes = unsafe { std::slice::from_raw_parts(request, length) };
        let request: Request = serde_json::from_slice(bytes)
            .map_err(|_| "invalid toggle hotkey request".to_owned())?;
        let keys = if request.keybindings.is_null() {
            msime_client_core::preferences::KeybindingPreferences::default()
        } else {
            serde_json::from_value(request.keybindings)
                .map_err(|_| "invalid keybindings".to_owned())?
        };
        Ok(json!({
            "english_candidates": toggle_hotkey_chord(keys.english_candidates_chord()),
            "full_width": toggle_hotkey_chord(keys.full_width_chord()),
            "punctuation": toggle_hotkey_chord(keys.punctuation_chord()),
            "voice_input": toggle_hotkey_chord(keys.toggle_voice_input.parse().ok()),
        }))
    })
}

/// 原生设置录键的试写检查：复用共享偏好的语法、规范化和冲突表，不写入配置。
/// # Safety
/// `request` 指向不超过 64 KiB 的有效 JSON；返回字符串交由 `msime_client_string_free` 释放。
#[no_mangle]
pub unsafe extern "C" fn msime_client_check_toggle_hotkey(
    request: *const u8,
    length: usize,
) -> *mut c_char {
    use msime_client_core::preferences::{
        KeybindingPreferences, ToggleHotkeyRejection, ToggleHotkeySlot,
    };
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        field: ToggleHotkeySlot,
        chord: String,
        #[serde(default)]
        keybindings: KeybindingPreferences,
        voice_ctrl_f9: bool,
    }
    response(|| {
        if request.is_null() || length > 64 * 1024 {
            return Err("invalid toggle hotkey request buffer".into());
        }
        // SAFETY: 调用方保证缓冲区可读，上方已检查长度上限。
        let bytes = unsafe { std::slice::from_raw_parts(request, length) };
        let request: Request = serde_json::from_slice(bytes)
            .map_err(|_| "invalid toggle hotkey request".to_owned())?;
        Ok(
            match request.keybindings.check_toggle_hotkey(
                request.field,
                &request.chord,
                request.voice_ctrl_f9,
            ) {
                Ok(chord) => json!({"ok": true, "chord": chord}),
                Err(ToggleHotkeyRejection::Invalid(_)) => json!({"ok": false, "kind": "invalid"}),
                Err(ToggleHotkeyRejection::Conflict { owner, .. }) => json!({
                    "ok": false, "kind": "conflict", "owner": owner.id(),
                }),
            },
        )
    })
}

fn toggle_hotkey_chord(chord: Option<msime_client_core::key_chord::KeyChord>) -> Value {
    chord.map_or(Value::Null, |chord| {
        json!({
            "key": chord.key.to_string(),
            "ctrl": chord.modifiers.ctrl,
            "alt": chord.modifiers.alt,
            "shift": chord.modifiers.shift,
            "meta": chord.modifiers.meta,
        })
    })
}



/// 列出自定义辅助码表：状态根和资源目录两处的 `helpcodes/custom`，规则见 [`crate::list_custom_helpcode_schemas`]。原生设置界面和共享设置页用同一份元数据，码表本身仍由 Engine 解析。目录不在时是空列表，不算错。
/// # Safety
/// `resources` points to `length` readable UTF-8 bytes naming an absolute resource directory; null is rejected. `state_root` is null (with `state_root_length` 0) or points to `state_root_length` readable UTF-8 bytes naming the absolute state root (HostOptions `preferences_directory`). The returned JSON must be released with `msime_client_string_free`.
#[no_mangle]
pub unsafe extern "C" fn msime_client_helpcode_schemas(
    resources: *const u8,
    length: usize,
    state_root: *const u8,
    state_root_length: usize,
) -> *mut c_char {
    response(|| {
        if resources.is_null() || length > 16_384 || state_root_length > 16_384 {
            return Err("invalid helpcode resource buffer".into());
        }
        // SAFETY: guaranteed by the documented caller contract; size checked above.
        let bytes = unsafe { std::slice::from_raw_parts(resources, length) };
        let resources =
            std::str::from_utf8(bytes).map_err(|_| "invalid helpcode resource encoding")?;
        let state_root = if state_root.is_null() {
            None
        } else {
            // SAFETY: guaranteed by the documented caller contract; size checked above.
            let bytes = unsafe { std::slice::from_raw_parts(state_root, state_root_length) };
            Some(std::str::from_utf8(bytes).map_err(|_| "invalid helpcode state root encoding")?)
        };
        let schemas = crate::list_custom_helpcode_schemas(resources, state_root)?;
        serde_json::to_value(schemas).map_err(|error| error.to_string())
    })
}

#[derive(Debug, Deserialize)]
struct SkinPackageRequest {
    directory: String,
    id: String,
}


/// What dictionary is installed, for the settings page to show.
///
/// The packaged dictionary ships with a manifest naming which specification it
/// was built to and which upstream commit it came from. Apple's settings read
/// it straight out of the app bundle; a host whose resources are staged into a
/// sandbox cannot, and the path is not something a settings page should be
/// told anyway.
///
/// Only two fields come back. The manifest also records journal modes, format
/// contracts and every third-party reference, none of which answers the
/// question the page is asking — which is "what do I have, and where did it
/// come from". A missing or unreadable manifest is reported rather than
/// guessed at: showing the wrong dictionary version is worse than showing
/// none.
/// # Safety
/// `resources` points to `length` readable UTF-8 bytes. Null is rejected.
#[no_mangle]
pub unsafe extern "C" fn msime_client_dictionary_manifest(
    resources: *const u8,
    length: usize,
) -> *mut c_char {
    #[derive(Deserialize)]
    struct Source {
        commit: String,
    }
    #[derive(Deserialize)]
    struct Manifest {
        profile: String,
        source: Source,
    }
    response(|| {
        if resources.is_null() || length > 16384 {
            return Err("dictionary_manifest_unavailable".into());
        }
        // SAFETY: guaranteed by the documented caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(resources, length) };
        let directory =
            std::str::from_utf8(bytes).map_err(|_| "dictionary_manifest_unavailable")?;
        let path = Path::new(directory);
        if !path.is_absolute() {
            return Err("dictionary_manifest_unavailable".into());
        }
        // Bounded before parsing: this is a packaged file, and one that has grown to megabytes is
        // not a manifest whatever it parses as.
        let file = path.join("msime-dictionary-manifest.json");
        let bytes = crate::bounded_file::read(
            crate::bounded_file::open_private(&file)
                .map_err(|_| "dictionary_manifest_unavailable")?,
            1024 * 1024,
        )
        .map_err(|_| "dictionary_manifest_unavailable")?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "dictionary_manifest_unavailable")?;
        let manifest: Manifest =
            serde_json::from_str(text).map_err(|_| "dictionary_manifest_unavailable")?;
        if manifest.profile.is_empty()
            || !is_bounded_text(&manifest.profile, 64)
            || !msime_client_core::is_ascii_hex(&manifest.source.commit, 40)
        {
            return Err("dictionary_manifest_unavailable".into());
        }
        Ok(json!({"profile": manifest.profile, "sourceCommit": manifest.source.commit}))
    })
}



/// Try to read preferences without waiting for the writer lock.
/// # Safety
/// `directory` must point to `length` readable bytes. Null is rejected.
#[no_mangle]
pub unsafe extern "C" fn msime_client_try_load_preferences(
    directory: *const u8,
    length: usize,
) -> *mut c_char {
    response(|| {
        if directory.is_null() || length > 16384 {
            return Err("invalid preferences directory buffer".into());
        }
        // SAFETY: guaranteed by the documented caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(directory, length) };
        let directory = super::parse_absolute_path(
            bytes,
            "invalid preferences directory encoding",
            "preferences directory must be absolute",
        )?;
        let snapshot = PreferencesStore::new(directory)
            .try_load()
            .map_err(|e| e.to_string())?;
        serde_json::to_value(snapshot).map_err(|e| e.to_string())
    })
}

/// Repair a preferences document that is not well-formed JSON, backing it up first.
/// See `PreferencesStore::recover_malformed`; a valid or missing document is left untouched.
/// # Safety
/// `directory` must point to `length` readable bytes. Null is rejected.
#[no_mangle]
pub unsafe extern "C" fn msime_client_recover_preferences(
    directory: *const u8,
    length: usize,
) -> *mut c_char {
    // SAFETY: forwarded under the same caller contract.
    unsafe { recover_preferences_in(directory, length, RecoveryScope::Malformed) }
}

/// The settings page's explicit 修复配置文件: repair a preferences document `load` rejects for any reason, a newer build's included, backing it up first.
/// See `PreferencesStore::recover`; a valid or missing document is left untouched. Only call it on the user's request: a document an older build cannot read is usually a newer build's, and the repair keeps only what this schema accepts.
/// # Safety
/// `directory` must point to `length` readable bytes. Null is rejected.
#[no_mangle]
pub unsafe extern "C" fn msime_client_repair_preferences(
    directory: *const u8,
    length: usize,
) -> *mut c_char {
    // SAFETY: forwarded under the same caller contract.
    unsafe { recover_preferences_in(directory, length, RecoveryScope::Unreadable) }
}

#[derive(Clone, Copy)]
enum RecoveryScope {
    /// Only a document that is not well-formed JSON; what input method hosts may do on their own.
    Malformed,
    /// Anything `load` rejects; only on the user's request.
    Unreadable,
}

/// # Safety
/// `directory` must point to `length` readable bytes. Null is rejected.
unsafe fn recover_preferences_in(
    directory: *const u8,
    length: usize,
    scope: RecoveryScope,
) -> *mut c_char {
    response(|| {
        if directory.is_null() || length > 16384 {
            return Err("invalid preferences directory buffer".into());
        }
        // SAFETY: guaranteed by the documented caller contract.
        let bytes = unsafe { std::slice::from_raw_parts(directory, length) };
        let directory = super::parse_absolute_path(
            bytes,
            "invalid preferences directory encoding",
            "preferences directory must be absolute",
        )?;
        let store = PreferencesStore::new(directory);
        let outcome = match scope {
            RecoveryScope::Malformed => store.recover_malformed(),
            RecoveryScope::Unreadable => store.recover(),
        }
        .map_err(|e| e.to_string())?;
        Ok(match outcome {
            msime_client_core::preferences::RecoveryOutcome::NotNeeded(snapshot) => json!({
                "recovered": false,
                "snapshot": snapshot,
            }),
            msime_client_core::preferences::RecoveryOutcome::Recovered {
                snapshot,
                backup_path,
                salvaged,
            } => json!({
                "recovered": true,
                "snapshot": snapshot,
                "backup_path": backup_path.to_string_lossy(),
                "backup_name": backup_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                "salvaged": salvaged,
            }),
        })
    })
}

/// Compare-and-swap save for a validated PreferencesSnapshot.
/// A successful save with clipboard history disabled clears the default history
/// under the shared preference/history locks, matching the desktop settings path.
///
/// # Safety
/// The caller must provide non-null pointers to readable UTF-8 buffers whose
/// lengths match the supplied lengths and remain valid for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn msime_client_save_preferences(
    directory: *const u8,
    directory_length: usize,
    expected_revision: u64,
    snapshot: *const u8,
    snapshot_length: usize,
) -> *mut c_char {
    response(|| {
        if directory.is_null()
            || snapshot.is_null()
            || directory_length > 16384
            || snapshot_length > PREFERENCES_DOCUMENT_LIMIT
        {
            return Err("invalid preferences save buffer".into());
        }
        let directory_bytes = unsafe { std::slice::from_raw_parts(directory, directory_length) };
        let directory = super::parse_absolute_path(
            directory_bytes,
            "invalid preferences directory encoding",
            "preferences directory must be absolute",
        )?;
        let snapshot_bytes = unsafe { std::slice::from_raw_parts(snapshot, snapshot_length) };
        // 提交来的偏好严格解析（Preferences::from_submission），不走读盘时的宽松读法：宿主送来本版本不认识的名字是宿主的毛病，要报出来，而不是悄悄整理掉再存下。
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Submitted {
            format_version: u32,
            #[allow(dead_code)]
            revision: u64,
            preferences: Value,
        }
        let snapshot: Submitted =
            serde_json::from_slice(snapshot_bytes).map_err(|_| "invalid preferences snapshot")?;
        if snapshot.format_version != 1 {
            return Err("unsupported preferences format".into());
        }
        let preferences =
            Preferences::from_submission(snapshot.preferences).map_err(|error| match error {
                msime_client_core::preferences::PreferencesError::Json(_) => {
                    "invalid preferences snapshot".to_owned()
                }
                other => other.to_string(),
            })?;
        let store = PreferencesStore::new(directory);
        let saved = store
            .save(expected_revision, preferences)
            .map_err(|e| e.to_string())?;
        if !saved.preferences.clipboard_history {
            store
                .clear_disabled_clipboard_history()
                .map_err(|e| e.to_string())?;
        }
        serde_json::to_value(saved).map_err(|e| e.to_string())
    })
}

