#include "../../../common/ToggleHotkeyChord.h"
#include "../input/ToggleHotkeyChord.h"
#include "../../../shared/contracts/windows_ipc.h"
#include "../../compat/WindowsDpi.h"
#include "../../../common/HostApiString.h"
#include "AuxListener.h"
#include "CandidateAppearance.h"
#include "CandidateThemeSettings.h"
#include "DesktopCandidates.h"
#include "CandidateWindowStyleSettings.h"
#include "ComponentFailure.h"
#include "DedicatedEnglishMailbox.h"
#include "EditPolicy.h"
#include "FloatingToolbarVisibilityPolicy.h"
#include "FirstRun.h"
#include "FloatingToolbarWindow.h"
#include "FloatingToolbarPosition.h"
#include "ToolbarTooltips.h"
#include "SystemPanels.h"
#include "FocusedSession.h"
#include "ClientAgent.h"
#include "InputSchemeTraits.h"
#include "FullscreenForeground.h"
#include "HelpcodeRoot.h"
#include "MaintenanceHotkey.h"
#include "NativeTextInput.h"
#include "ModeAuthority.h"
#include "AppInputModeRules.h"
#include "InputModeHudWindow.h"
#include "InputStateHudFeedback.h"
#include "ShuangpinKeymapWindow.h"
#include "ProcessImageName.h"
#include "ModeMailbox.h"
#include "PreviewConfig.h"
#include "PreviewDispatcher.h"
#include "ProcessPriority.h"
#include "ProductionDispatcher.h"
#include "ProductionPipeNames.h"
#include "SecureFieldProbe.h"
#include "ServerLaunch.h"
#include "ShellLauncher.h"
#include "StateRootLease.h"
#include "ToolbarModeCommand.h"
#include "TrayMenuDispatch.h"
#include "TrayMenuWindow.h"
#include "WatchdogPolicy.h"
#include "RevisionFence.h"
#include "WindowsServer.h"
#include "ipc_negotiation.h"
#include <windows.h>
#include <iostream>
#include <map>
#include <memory>
#include <mutex>
#include <set>
#include <unordered_map>
#include <cstdlib>
#include <exception>
#include <thread>
#ifdef _WIN32
#include "StateDirectory.h"
#endif

namespace {
constexpr std::size_t kMaxConfigBytes = 16 * 1024;

// The desktop shell is packaged beside this Server; a development build points
// at another copy with the same variable the Linux host reads.
std::filesystem::path executable_directory() {
  std::vector<wchar_t> path(32768);
  const DWORD length =
      GetModuleFileNameW(nullptr, path.data(), static_cast<DWORD>(path.size()));
  if (!length || length == path.size())
    return {};
  return std::filesystem::path(std::wstring(path.data(), length)).parent_path();
}
std::wstring configured_shell_command() {
  std::vector<wchar_t> value(32768);
  const DWORD length = GetEnvironmentVariableW(
      L"MSIME_CLIENT_SETTINGS_COMMAND", value.data(),
      static_cast<DWORD>(value.size()));
  return length && length < value.size() ? std::wstring(value.data(), length)
                                         : std::wstring{};
}
std::atomic<bool> stopping{false};
std::atomic<bool> restart_requested{false};
// Set by the maintenance stop shortcut. The Watchdog reads any other exit as a crash and starts the Server again, so a user's stop has to leave with stop_exit_code, as the reference's window hook does.
std::atomic<bool> stop_requested{false};
BOOL WINAPI console_control(DWORD event) {
  if (event != CTRL_C_EVENT && event != CTRL_BREAK_EVENT)
    return FALSE;
  stopping.store(true);
  return TRUE;
}
struct ConsoleControl {
  ConsoleControl() {
    if (!SetConsoleCtrlHandler(console_control, TRUE))
      throw std::runtime_error("Console control unavailable");
  }
  ~ConsoleControl() { SetConsoleCtrlHandler(console_control, FALSE); }
};
// The Server is a windows-subsystem program, so a managed launch (Watchdog or TSF DLL) never opens a console window. A preview or --help run from a terminal attaches to that terminal instead, so its status lines and Ctrl+C still work there. A managed launch never attaches: the TSF DLL starts the Server from inside whatever application has focus, and that may itself be a console program whose window must not receive our output.
void attach_launching_console(const msime::windows::ServerLaunch &launch) {
  if (launch.kind == msime::windows::ServerLaunchKind::Managed ||
      !AttachConsole(ATTACH_PARENT_PROCESS))
    return;
  FILE *stream = nullptr;
  freopen_s(&stream, "CONOUT$", "w", stdout);
  freopen_s(&stream, "CONOUT$", "w", stderr);
  std::cout.clear();
  std::cerr.clear();
}
bool contains(const std::filesystem::path &parent,
              const std::filesystem::path &child) {
  auto p = parent.begin(), c = child.begin();
  for (; p != parent.end(); ++p, ++c)
    if (c == child.end() || CompareStringOrdinal(p->c_str(), -1, c->c_str(), -1,
                                                 TRUE) != CSTR_EQUAL)
      return false;
  return true;
}
std::filesystem::path production_state_directory() {
#ifdef _WIN32
  return msime::windows::resolve_state_directory();
#else
  return {};
#endif
}
std::string read_document(const std::filesystem::path &path) {
  const auto document = msime::windows::read_private_file(path, kMaxConfigBytes);
  if (!document)
    throw std::runtime_error("Configuration unavailable");
  return *document;
}
void write_document_atomic(const std::filesystem::path &path, const std::string &document) {
  if (document.size() > kMaxConfigBytes)
    throw std::runtime_error("Configuration document oversized");
#ifdef _WIN32
  msime::windows::reject_reparse_ancestors(path.parent_path());
#endif
  wchar_t temporary_name[MAX_PATH] = {};
  if (!GetTempFileNameW(path.parent_path().c_str(), L"msi", 0, temporary_name))
    throw std::runtime_error("Configuration temporary file unavailable");
  const std::filesystem::path temporary(temporary_name);
  HANDLE handle = CreateFileW(temporary.c_str(), GENERIC_WRITE, 0, nullptr,
                              OPEN_EXISTING,
                              FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                              nullptr);
  if (handle == INVALID_HANDLE_VALUE || !msime::windows::handle_is_trusted_file(handle)) {
    if (handle != INVALID_HANDLE_VALUE)
      CloseHandle(handle);
    (void)msime::windows::remove_private_file(temporary);
    throw std::runtime_error("Configuration temporary file unavailable");
  }
  DWORD written = 0;
  const bool complete = document.size() <= MAXDWORD &&
                        WriteFile(handle, document.data(),
                                  static_cast<DWORD>(document.size()), &written,
                                  nullptr) &&
                        written == static_cast<DWORD>(document.size()) &&
                        FlushFileBuffers(handle);
  CloseHandle(handle);
  if (!complete) {
    (void)msime::windows::remove_private_file(temporary);
    throw std::runtime_error("Configuration write failed");
  }
  if (!MoveFileExW(temporary.c_str(), path.c_str(),
                   MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    (void)msime::windows::remove_private_file(temporary);
    throw std::runtime_error("Configuration replace failed");
  }
}
// 原生工具栏和 TSF 快捷键共用带修订号的偏好存储和单次工作线程，界面和输入队列不等存储锁。
bool persist_traditional_output(const std::filesystem::path &directory,
                                std::atomic<bool> &state,
                                std::optional<bool> desired = std::nullopt,
                                std::optional<bool> *changed = nullptr) {
  if (changed) changed->reset();
  try {
    const auto root = directory.u8string();
    auto loaded = msime::host_api::own_string(
        msime_client_load_preferences(
            reinterpret_cast<const uint8_t *>(root.data()), root.size()));
    if (!loaded)
      return false;
    const auto response = nlohmann::json::parse(loaded.get());
    if (!response.value("ok", false) || !response.at("value").is_object())
      return false;
    auto snapshot = response.at("value");
    const auto revision = snapshot.at("revision").get<uint64_t>();
    auto &preferences = snapshot.at("preferences");
    const bool enabled = preferences.value("traditional_chinese_output", false);
    const bool next = desired.value_or(!enabled);
    if (next == enabled) {
      state.store(next, std::memory_order_release);
      return true;
    }
    preferences["traditional_chinese_output"] = next;
    const auto serialized = snapshot.dump();
    auto saved = msime::host_api::own_string(
        msime_client_save_preferences(
            reinterpret_cast<const uint8_t *>(root.data()), root.size(),
            revision,
            reinterpret_cast<const uint8_t *>(serialized.data()),
            serialized.size()));
    if (!saved)
      return false;
    const auto saved_response = nlohmann::json::parse(saved.get());
    if (!saved_response.value("ok", false) ||
        !saved_response.at("value").is_object())
      return false;
    state.store(next, std::memory_order_release);
    if (changed) *changed = next;
    return true;
  } catch (...) {
    return false;
  }
}
// 把安装器「联网功能」页的选择写进刚准备好的共享偏好（Linux 的 msime-linux-prepare 做同样的事）。失败时保持共享默认值，也就是关闭，不阻止输入法启动。
void record_installer_cloud_choice(const std::filesystem::path &directory, bool enabled) {
  try {
    const auto root = directory.u8string();
    auto loaded = msime::host_api::own_string(
        msime_client_load_preferences(
            reinterpret_cast<const uint8_t *>(root.data()), root.size()));
    if (!loaded)
      return;
    const auto response = nlohmann::json::parse(loaded.get());
    if (!response.value("ok", false) || !response.at("value").is_object())
      return;
    auto snapshot = response.at("value");
    const auto revision = snapshot.at("revision").get<uint64_t>();
    snapshot.at("preferences")["cloud_candidates"] = enabled;
    const auto serialized = snapshot.dump();
    auto saved = msime::host_api::own_string(
        msime_client_save_preferences(
            reinterpret_cast<const uint8_t *>(root.data()), root.size(),
            revision,
            reinterpret_cast<const uint8_t *>(serialized.data()),
            serialized.size()));
  } catch (...) {
  }
}
// Read the stored preferences block, or nothing if it cannot be read. The
// shipped card has usable built-in defaults, so an unreadable store degrades
// to those rather than stopping the IME from starting.
std::optional<nlohmann::json>
load_preference_block(const std::filesystem::path &directory) {
  try {
    const auto root = directory.u8string();
    auto loaded = msime::host_api::own_string(
        msime_client_load_preferences(
            reinterpret_cast<const uint8_t *>(root.data()), root.size()));
    if (!loaded)
      return std::nullopt;
    const auto response = nlohmann::json::parse(loaded.get());
    if (!response.value("ok", false) || !response.at("value").is_object())
      return std::nullopt;
    const auto &snapshot = response.at("value");
    if (!snapshot.contains("preferences") ||
        !snapshot.at("preferences").is_object())
      return std::nullopt;
    return snapshot.at("preferences");
  } catch (...) {
    return std::nullopt;
  }
}
// "system" follows Windows. Absent or unreadable, keep the shipped dark card
// rather than guessing light and flashing a white panel over a dark desktop.
bool system_prefers_dark() {
  DWORD light = 0;
  DWORD size = sizeof(light);
  if (RegGetValueW(HKEY_CURRENT_USER,
                   L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\"
                   L"Personalize",
                   L"AppsUseLightTheme", RRF_RT_REG_DWORD, nullptr, &light,
                   &size) != ERROR_SUCCESS)
    return true;
  return light == 0;
}
// Flip a stored boolean through the same revisioned store the settings shell
// uses. The tray rows used to change only an in-process flag, so the choice was
// forgotten on every Server restart and disagreed with the settings page.
bool toggle_stored_flag(const std::filesystem::path &directory,
                        const char *section, const char *field, bool fallback,
                        bool &result) {
  try {
    const auto root = directory.u8string();
    auto loaded = msime::host_api::own_string(
        msime_client_load_preferences(
            reinterpret_cast<const uint8_t *>(root.data()), root.size()));
    if (!loaded)
      return false;
    const auto response = nlohmann::json::parse(loaded.get());
    if (!response.value("ok", false) || !response.at("value").is_object())
      return false;
    auto snapshot = response.at("value");
    const auto revision = snapshot.at("revision").get<uint64_t>();
    auto &preferences = snapshot.at("preferences");
    bool current = fallback;
    if (section) {
      if (!preferences.contains(section) || !preferences.at(section).is_object())
        preferences[section] = nlohmann::json::object();
      current = preferences.at(section).value(field, fallback);
      preferences[section][field] = !current;
    } else {
      current = preferences.value(field, fallback);
      preferences[field] = !current;
    }
    const auto serialized = snapshot.dump();
    auto saved = msime::host_api::own_string(
        msime_client_save_preferences(
            reinterpret_cast<const uint8_t *>(root.data()), root.size(),
            revision, reinterpret_cast<const uint8_t *>(serialized.data()),
            serialized.size()));
    if (!saved)
      return false;
    const auto saved_response = nlohmann::json::parse(saved.get());
    if (!saved_response.value("ok", false) ||
        !saved_response.at("value").is_object())
      return false;
    result = !current;
    return true;
  } catch (...) {
    return false;
  }
}
// 通过带版本的存储选择输入方案，并像设置页一样维护 last_chinese_scheme：中文方案（包括粤拼、注音和笔画）也是日文、韩文、越南文和藏文切回时回到的方案，选择这些语言之一时记住被替换的中文方案。在它们之间切换保留记住的方案，因为它们都不是存储会接受的中文方案。
bool store_input_scheme(const std::filesystem::path &directory,
                        const std::string &scheme, bool &changed) {
  changed = false;
  try {
    const auto root = directory.u8string();
    auto loaded = msime::host_api::own_string(
        msime_client_load_preferences(
            reinterpret_cast<const uint8_t *>(root.data()), root.size()));
    if (!loaded)
      return false;
    const auto response = nlohmann::json::parse(loaded.get());
    if (!response.value("ok", false) || !response.at("value").is_object())
      return false;
    auto snapshot = response.at("value");
    const auto revision = snapshot.at("revision").get<uint64_t>();
    auto &preferences = snapshot.at("preferences");
    const auto current =
        preferences.contains("scheme") && preferences.at("scheme").is_string()
            ? preferences.at("scheme").get<std::string>()
            : std::string("quanpin");
    if (current == scheme)
      return true;
    // 粤拼、注音和笔画是中文方案，和其他中文方案一样被记住；日文、韩文、越南文和藏文各是独立的语言（client-core 的 ChineseScheme）。
    if (msime::windows::scheme::is_chinese_scheme_name(scheme))
      preferences["last_chinese_scheme"] = scheme;
    else if (msime::windows::scheme::is_chinese_scheme_name(current))
      preferences["last_chinese_scheme"] = current;
    preferences["scheme"] = scheme;
    const auto serialized = snapshot.dump();
    auto saved = msime::host_api::own_string(
        msime_client_save_preferences(
            reinterpret_cast<const uint8_t *>(root.data()), root.size(),
            revision, reinterpret_cast<const uint8_t *>(serialized.data()),
            serialized.size()));
    if (!saved)
      return false;
    const auto saved_response = nlohmann::json::parse(saved.get());
    changed = saved_response.value("ok", false) &&
              saved_response.at("value").is_object();
    return changed;
  } catch (...) {
    return false;
  }
}
// The Cantonese, Zhuyin and Stroke dictionaries the package installed beside the resources, where host-api looks for them (language_dictionaries_beside). They arrive with a package, so one look at startup holds for the process.
msime::windows::scheme::LanguageDictionaryPresence
installed_language_dictionaries(const std::filesystem::path &resources) {
  const auto directory = resources.parent_path() / L"language-dictionaries";
  std::error_code error;
  return {std::filesystem::is_regular_file(directory / L"msime-cantonese.db", error),
          std::filesystem::is_regular_file(directory / L"msime-zhuyin.db", error),
          std::filesystem::is_regular_file(directory / L"msime-stroke.db", error)};
}
// The scheme the Engine runs for the stored preferences, which is the stored one unless it needs a dictionary that is not installed.
std::string running_scheme(
    const nlohmann::json &preferences,
    msime::windows::scheme::LanguageDictionaryPresence installed) {
  return std::string(msime::windows::scheme::scheme_name(
      msime::windows::scheme::effective_scheme(
          preferences.value("scheme", std::string("quanpin")),
          preferences.value("last_chinese_scheme", std::string("quanpin")),
          installed)));
}
// The stored preferences the tray card shows. The preference monitor publishes them and the UI thread reads them whenever the card is built.
struct TrayMenuPreferences {
  std::string scheme = "quanpin";
  std::string shuangpin_profile = "xiaohe";
  std::string language_hint;
  // 「繁体输出」旁是否写出 Ctrl + Shift + F，和「中文标点」是否被 punctuation_lock 钉住。
  bool character_set_shortcut = true;
  bool punctuation_locked = false;
};
TrayMenuPreferences tray_menu_preferences(
    const nlohmann::json &preferences,
    msime::windows::scheme::LanguageDictionaryPresence installed) {
  TrayMenuPreferences result;
  result.scheme = running_scheme(preferences, installed);
  result.shuangpin_profile =
      preferences.value("shuangpin_profile", std::string("xiaohe"));
  // The same defaults the TIP reads (FanyUtils::ReadConfiguredSwitchLanguageHotkeys).
  const auto bindings =
      preferences.value("keybindings", nlohmann::json::object());
  result.language_hint = msime::windows::tray_menu_language_hint(
      bindings.value("switch_language_shift", true),
      bindings.value("switch_language_ctrl", false),
      bindings.value("switch_language_ctrl_alt_space", true));
  // 与 TIP 读的默认值相同（FanyUtils::ReadConfiguredSwitchLanguageHotkeys 和 tsf_local_config 的 punctuation_lock）。
  result.character_set_shortcut =
      bindings.value("toggle_character_set_ctrl_shift_f", true);
  const auto lock = preferences.value("punctuation_lock", std::string("follow"));
  result.punctuation_locked = lock == "chinese" || lock == "english";
  return result;
}
// HUD 和菜单共用方案名称，避免双拼、五笔的配置名在两处不一致。
msime::windows::InputStateHudText scheme_hud(const TrayMenuPreferences &preferences) {
  msime::windows::TrayMenuState state;
  state.scheme = preferences.scheme;
  state.shuangpin_profile = preferences.shuangpin_profile;
  for (const auto &item : msime::windows::tray_menu_scheme_items({}, state)) {
    const auto *name = msime::windows::tray_menu_scheme(item.command);
    if (!name || preferences.scheme != name) continue;
    const int count = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS,
        item.label.data(), static_cast<int>(item.label.size()), nullptr, 0);
    if (count <= 0) break;
    std::wstring title(static_cast<size_t>(count), L'\0');
    if (MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, item.label.data(),
        static_cast<int>(item.label.size()), title.data(), count) != count) break;
    return {title, L"输入方案：" + title};
  }
  return {};
}
// Map the shared preferences onto the settings the TIP keeps in its own
// globals. The TIP consumes every one of these, but nothing ever sent them, so
// they sat at their compiled defaults: turning smart or paired punctuation off
// did nothing, the Microsoft shuangpin ';' key was never enabled, and the
// inline preedit style stayed "raw" whatever the user picked.
// The token for the provider actually in use.
//
// Tokens are kept one per provider so switching provider restores the matching key instead of sending the previous provider's key to the new endpoint.
msime::windows::TsfLocalConfig tsf_local_config(
    const nlohmann::json &preferences,
    msime::windows::scheme::LanguageDictionaryPresence installed) {
  msime::windows::TsfLocalConfig config;
  // The TIP keys the scheme the Engine runs: Zhuyin chosen without msime-zhuyin.db runs a pinyin scheme, and keying it as Zhuyin would swallow the tone digits.
  const auto scheme = running_scheme(preferences, installed);
  const auto navigation =
      preferences.value("navigation", nlohmann::json::object());
  config.paging_comma_period = navigation.value("comma_period", true);
  config.preedit_style = msime::windows::tsf_preedit_style(preferences);
  // PreviewConfig spells the pass-through case "local"; the TIP spells it "raw".
  if (config.preedit_style == "local")
    config.preedit_style = "raw";
  // 回退值与 client-core 的 smart_punctuation_default 相同：智能标点、重复转中文和两种直出默认开，空格转换默认关。偏好经 msime_client_load_preferences 读出，总带着这些键（macOS、Windows 上旧文档存着的旧默认 false 已在读入时一次性改过来），回退值只在键缺失时起作用。
  config.smart_punctuation = preferences.value("smart_punctuation", true);
  config.smart_punctuation_repeat_to_chinese =
      preferences.value("smart_punctuation_repeat", true);
  config.smart_punctuation_space_convert =
      preferences.value("smart_punctuation_space_convert", false);
  config.smart_punctuation_direct_digit =
      preferences.value("smart_punctuation_direct_digit", true);
  config.smart_punctuation_direct_letter =
      preferences.value("smart_punctuation_direct_letter", true);
  config.paired_punctuation = preferences.value("paired_punctuation", true);
  // 不只看方案名：自定义方案把韵母或零声母编码放在 `;` 上时，TIP 同样要把 `;` 当输入键。
  config.microsoft_shuangpin =
      scheme == "shuangpin" && msime::windows::shuangpin_uses_semicolon_key(preferences);
  config.input_mode = msime::windows::scheme::input_mode(scheme);
  const auto lock = preferences.value("punctuation_lock", std::string("follow"));
  config.punctuation_lock = lock == "chinese" ? 1 : lock == "english" ? 2 : 0;
  // The Engine opens V only in the pinyin schemes (`opens_local_modes`), and "/" and "@" in the pinyin schemes and Wubi (`opens_table_modes`); apply_local_mode_switches gates each switch on the common/InputSchemeTraits.h mirror that scripts/test-scheme-traits-parity.py checks. The switches are left out of the stored document while off.
  const auto local_modes =
      preferences.value("local_modes", nlohmann::json::object());
  msime::windows::apply_local_mode_switches(
      config, msime::windows::scheme::scheme_from_name(scheme),
      local_modes.value("expression", false),
      local_modes.value("command", false),
      local_modes.value("mention", false));
  // 与按键路由读同一个字段（InputState 经 preference_navigation），TIP 和 Server 对这两个键的归类才一致。
  config.second_third_candidate = msime::windows::preference_second_third_candidate(preferences);
  const auto wubi_selection = preferences.value("wubi_candidate_selection", nlohmann::json::object());
  config.wubi_candidate_selection = wubi_selection.value("enabled", false);
  const auto second = wubi_selection.value("second_key", std::string(";"));
  const auto third = wubi_selection.value("third_key", std::string("'"));
  if (second.size() == 1 && third.size() == 1) {
    config.wubi_second_key = static_cast<unsigned char>(second[0]);
    config.wubi_third_key = static_cast<unsigned char>(third[0]);
  } else {
    config.wubi_candidate_selection = false;
  }
  // 引号与方头括号样式：TIP 自己的标点表按它换字。缺键就是缺省（偏好文档不写等于缺省的值）。
  const auto string_preference = [&preferences](const char *key) {
    const auto found = preferences.find(key);
    return found != preferences.end() && found->is_string() ? found->get<std::string>() : std::string();
  };
  msime::windows::apply_punctuation_marks(config, string_preference("quote_style"),
                                          string_preference("bracket_style"),
                                          preferences.value("traditional_chinese_output", false));
  return config;
}

// 会话控制器停下的原因，写进停止那一行；与 ControllerFailure 一一对应。
const char *controller_failure_name(msime::windows::ControllerFailure failure) {
  using msime::windows::ControllerFailure;
  switch (failure) {
  case ControllerFailure::None:
    return "none";
  case ControllerFailure::Service:
    return "service";
  case ControllerFailure::InputQueue:
    return "input queue";
  case ControllerFailure::SessionWorkers:
    return "session workers";
  case ControllerFailure::Control:
    return "control";
  case ControllerFailure::Preferences:
    return "preferences";
  }
  return "unknown";
}

std::string production_preview_document(const std::string &runtime_document,
                                        const std::filesystem::path &fallback) {
  const auto host = nlohmann::json::parse(runtime_document);
  if (!host.is_object() || !host.at("resources").is_string())
    throw std::invalid_argument("Invalid production host options");
  const auto state = host.value("preferences_directory", fallback.u8string());
  if (state.empty())
    throw std::invalid_argument("Production state directory unavailable");
  nlohmann::json document{
      {"format_version", 1},
      {"resources", host.at("resources")},
      {"state_root", state},
      {"pipe_namespace", "production"},
      {"preedit_style", "local"},
  };
  const auto preferences = load_preference_block(std::filesystem::u8path(state));
  if (!preferences)
    return document.dump();
  document["preedit_style"] =
      msime::windows::tsf_preedit_style(*preferences);
  document["appearance"] = msime::windows::candidate_appearance(
      std::filesystem::u8path(state), *preferences, system_prefers_dark());
  msime::windows::apply_floating_toolbar(document, *preferences);
  // Last line of defence. The field filtering above is deliberately
  // conservative, but a preference shape nobody anticipated must still not
  // cost the user their IME: if the assembled document would not load, drop
  // the appearance and start with the built-in card.
  try {
    msime::windows::PreviewConfig::parse(document.dump());
  } catch (...) {
    document.erase("appearance");
    document.erase("floating_toolbar_enabled");
    document.erase("floating_toolbar_scale");
    document.erase("floating_toolbar_font_size");
    document.erase("floating_toolbar_items");
    document["preedit_style"] = "local";
  }
  return document.dump();
}
// 本版本的 TIP 有没有活动的输入模式，用一个命名的手动重置事件告诉别的版本的 Server：有信号表示活动。名字后面接版本后缀（full 是 .full）。只有生产 Server 发布它，预览实例不碰。
constexpr wchar_t server_mode_active_event_prefix[] = L"Local\\MetasequoiaImeServer_ModeActive";
// 另一个版本的 TIP 是否有活动的输入模式：看那个版本的 Server 发布的事件。那个版本没在运行时事件不存在，按不活动处理。
bool other_edition_mode_active() {
  for (const wchar_t *suffix : {MSIME_EDITIONS_NAME_SUFFIXES}) {
    if (std::wstring_view(suffix) == MSIME_EDITION_NAME_SUFFIX)
      continue;
    const std::wstring name = std::wstring(server_mode_active_event_prefix) + suffix;
    if (HANDLE event = OpenEventW(SYNCHRONIZE, FALSE, name.c_str())) {
      const bool active = WaitForSingleObject(event, 0) == WAIT_OBJECT_0;
      CloseHandle(event);
      if (active)
        return true;
    }
  }
  return false;
}
class ProductionInstance final {
public:
  ProductionInstance() {
    handle_ = CreateMutexW(nullptr, FALSE,
                           L"Local\\MetasequoiaImeServer_SingleInstance" MSIME_EDITION_NAME_SUFFIX);
    if (!handle_)
      throw std::runtime_error("Server instance guard unavailable");
    already_running_ = GetLastError() == ERROR_ALREADY_EXISTS;
    // 建不出来时别的版本只是看不到本版本的模式，维护快捷键在没有任何版本活动时照样有人处理，所以不算启动失败。
    if (!already_running_)
      mode_active_ = CreateEventW(nullptr, TRUE, FALSE,
                                  (std::wstring(server_mode_active_event_prefix) + MSIME_EDITION_NAME_SUFFIX).c_str());
  }
  ~ProductionInstance() {
    if (mode_active_) {
      ResetEvent(mode_active_);
      CloseHandle(mode_active_);
    }
    if (handle_)
      CloseHandle(handle_);
  }
  bool already_running() const { return already_running_; }
  // 主循环每一轮发布一次本版本的模式是否活动，只在变化时改事件。
  void publish_mode_active(bool active) {
    if (!mode_active_ || active == mode_active_published_)
      return;
    mode_active_published_ = active;
    if (active)
      SetEvent(mode_active_);
    else
      ResetEvent(mode_active_);
  }
private:
  HANDLE handle_ = nullptr;
  HANDLE mode_active_ = nullptr;
  bool mode_active_published_ = false;
  bool already_running_ = false;
};
// A Server that TSF revived after a crash (--production) has no Watchdog above it, so it starts the one packaged beside it, as the reference Server does. The Watchdog adopts this running Server instead of launching a second one, holds its own single-instance mutex, and exits on its own when the TIP profile is not enabled.
void start_watchdog(const std::filesystem::path &directory) {
  const auto watchdog = directory / L"MetasequoiaImeWatchdog.exe";
  if (directory.empty() || GetFileAttributesW(watchdog.c_str()) == INVALID_FILE_ATTRIBUTES)
    return;
  std::wstring command = L"\"" + watchdog.wstring() + L"\"";
  STARTUPINFOW startup{};
  startup.cb = sizeof(startup);
  PROCESS_INFORMATION process{};
  if (CreateProcessW(watchdog.c_str(), command.data(), nullptr, nullptr, FALSE, 0,
                     nullptr, directory.c_str(), &startup, &process)) {
    CloseHandle(process.hThread);
    CloseHandle(process.hProcess);
  }
}
// 诊断日志里前台呈现方式的名字。
const char *foreground_presentation_name(
    msime::windows::ForegroundPresentation value) {
  switch (value) {
  case msime::windows::ForegroundPresentation::Fullscreen:
    return "fullscreen";
  case msime::windows::ForegroundPresentation::ExclusiveFullscreen:
    return "exclusive";
  case msime::windows::ForegroundPresentation::Windowed:
    break;
  }
  return "windowed";
}
} // namespace
int wmain(int argc, wchar_t **argv) {
  // 登录任务的默认 Priority 7 把 CPU 调度类和内存优先级一起压低，看门狗和它拉起的
  // Server 都继承这个状态；TSF 拉起、手工启动和以后的拉起路径也可能带进来。复位放在
  // 任何业务线程创建之前，失败只记一行，不阻止启动。
  const auto process_priority =
      msime::windows::restore_interactive_process_priority();
  msime::windows::compat::initialize_process_dpi();
  using namespace msime::windows;
  const auto launch = parse_server_arguments(argc, argv);
  attach_launching_console(launch);
  if (launch.kind == ServerLaunchKind::Help) {
    std::cout << "MSIME Server: --config <absolute-json-path>\n"
                 "Managed launches use the installed TSF pipe names; preview "
                 "launches use names from the config. Ctrl+C stops.\n"
                 "Unsupported routes (including unobserved Enter) disconnect.\n";
    return 0;
  }
  const bool production = launch.kind == ServerLaunchKind::Managed;
  if (launch.kind == ServerLaunchKind::Invalid)
    return 2;
  // 只报告固定阶段，不把可能带路径、账号或凭据的异常内容写到日志。
  const char* startup_stage = "configuration";
  try {
    const auto default_state = production_state_directory();
    std::unique_ptr<ProductionInstance> instance;
    bool prepared_now = false;
    if (production) {
      instance = std::make_unique<ProductionInstance>();
      if (instance->already_running())
        return 0;
      if (!launch.supervised)
        start_watchdog(executable_directory());
      // 首次准备失败需要可见的重试入口，不能让无控制台启动只留下标准错误。
      for (;;) {
        try {
          prepared_now = prepare_first_run(executable_directory(), default_state,
                           [](const std::string &request) {
            auto response = msime::host_api::own_string(
                msime_client_prepare_host(
                    reinterpret_cast<const uint8_t *>(request.data()), request.size()));
            if (!response)
              throw std::runtime_error("Host preparation failed");
            return std::string(response.get());
          });
          break;
        } catch (...) {
          if (MessageBoxW(nullptr, L"运行资源未能准备完成。请确认安装文件完整、数据目录可写，然后重试。", L"水杉输入法首次准备", MB_RETRYCANCEL | MB_ICONWARNING | MB_SETFOREGROUND) != IDRETRY)
            throw;
        }
      }
      // 只在这次刚准备好状态时采用安装器的选择；已有状态属于用户，文件照样删掉。
      if (const auto cloud = take_installer_cloud_choice(default_state); prepared_now && cloud)
        record_installer_cloud_choice(default_state, *cloud);
    }
    const std::filesystem::path config_path =
        production ? default_state / L"runtime-options.json"
                    : std::filesystem::path(launch.config);
    if (!config_path.is_absolute())
      throw std::invalid_argument("Relative config path");
    auto document = read_document(config_path);
    if (production)
      document = production_preview_document(document, default_state);
    auto config = PreviewConfig::parse(document);
    config.resources = std::filesystem::canonical(config.resources);
    reject_reparse_ancestors(config.state_root);
    config.state_root = std::filesystem::weakly_canonical(config.state_root);
    if (contains(config.resources, config.state_root) ||
        contains(config.state_root, config.resources))
      throw std::invalid_argument("Resources and state must be disjoint");
    StateRootLease lease(config.state_root);
    // 诊断日志已移除：只在失败时写标准错误，不写状态目录。
    if (!process_priority.cpu_ok)
      std::cerr << "Failed to restore NORMAL_PRIORITY_CLASS: error "
                << process_priority.cpu_error << "\n";
    if (!process_priority.memory_ok)
      std::cerr << "Failed to restore MEMORY_PRIORITY_NORMAL: error "
                << process_priority.memory_error << "\n";
    // 脚本插件只在 Server 里运行；TIP 和 Server 共用同一份宿主 DLL，但 TIP 从不调用它（scripts/test-script-plugin-hosts.py）。开发构建和 release 构建一样打开：授权由设置应用（platforms/windows/settings）的授权页写入，那里写明了同一用户下的其他程序能伪造授权；提权窗口（ELEVATED）里从不运行脚本（msime_client.h）。
    // Operator notices go to the terminal of a preview run and, when the server switch is on, to the diagnostic file - the only place a managed Server's notices can be read.
    const auto notice = [](const std::string &line) {
      std::cerr << line << "\n";
    };
    startup_stage = "console_control";
    ConsoleControl console;
    startup_stage = "host_prepare";
    auto bootstrap_document =
        nlohmann::json{{"resources", config.resources.u8string()},
                       {"state_root", config.state_root.u8string()}};
    // 不是 full 的版本把版本 id 交给宿主库：它按版本选资源锁、收窄方案，并在状态根里记下版本。full 不带这个键，请求与引入版本之前相同。
    if constexpr (!MSIME_EDITION_IS_FULL)
      bootstrap_document["edition"] = MSIME_EDITION_ID;
    // 会话用的是这里准备出的文档，runtime-options.json 却只在首次运行时写下。设置窗口、msime-mcp 和各应用里的 TIP 都照那份文件工作，升级换了词库代次后它们还在旧代次上，设置里新增的词条写进旧代次，输入法看不到（#9434）。让宿主库把这次的代次写回文件。
    if (production)
      bootstrap_document["runtime_options"] = config_path.u8string();
    const auto bootstrap = bootstrap_document.dump();
    auto response = msime::host_api::own_string(
        msime_client_prepare_host(
            reinterpret_cast<const uint8_t *>(bootstrap.data()),
            bootstrap.size()));
    if (!response)
      throw std::runtime_error("Host preparation failed");
    const auto prepared = nlohmann::json::parse(response.get());
    if (!prepared.at("ok").get<bool>())
      throw std::runtime_error("Host preparation failed");
    if (stopping.load())
      return 0;
    startup_stage = "services";
    notice(std::string(production ? "Production" : "Preview") +
           " Server starting");
    startup_stage = "preferences";
    const auto language_dictionaries =
        installed_language_dictionaries(config.resources);
    auto traditional_output = std::make_shared<std::atomic<bool>>(
        prepared.at("value").at("preferences")
            .value("traditional_chinese_output", false));
    auto tsf_config = std::make_shared<msime::windows::TsfLocalConfig>(
        tsf_local_config(prepared.at("value").at("preferences"),
                         language_dictionaries));
    auto tsf_config_mutex = std::make_shared<std::mutex>();
    auto tray_preferences = std::make_shared<TrayMenuPreferences>(
        tray_menu_preferences(prepared.at("value").at("preferences"),
                              language_dictionaries));
    auto tray_preferences_mutex = std::make_shared<std::mutex>();
    // Set on every publication and on each focus session, so a TIP that
    // registers later is not left holding compiled defaults. A revision is
    // used instead of a bool so an older send cannot clear a newer update.
    auto tsf_config_revision = std::make_shared<msime::windows::RevisionFence>();
    // 工具栏、托盘菜单、语音浮层一律跟随颜色模式（HostCapabilities::surface_themes_follow），不读存下的 toolbar_theme、menu_theme、voice_theme。
    // Cross-application CN/EN authority, when the user asked for one state to
    // follow them between applications.
    auto mode_scope_global = std::make_shared<std::atomic<bool>>(
        prepared.at("value").at("preferences")
            .value("ime_mode_scope", std::string("app")) == "global");
    // 应用例外（按进程基名固定起始模式）和中英文切换提示的开关，同样随偏好发布更新。规则表只在焦点进入新的客户端时读一次，用锁保护就够了。
    auto app_mode_rules = std::make_shared<std::pair<std::mutex, AppInputModeRules>>();
    app_mode_rules->second =
        ReadAppInputModeRules(prepared.at("value").at("preferences"));
    auto input_mode_hud_enabled = std::make_shared<std::atomic<bool>>(
        prepared.at("value").at("preferences").value("input_mode_hud", true));
    // 徽标画不画 logo 单独跟着 `show_app_logo` 走，不借工具栏那份设置：工具栏尺寸越界时那份设置不发布，logo 不该跟着丢。
    auto input_mode_hud_logo = std::make_shared<std::atomic<bool>>(
        hud_show_app_logo(prepared.at("value").at("preferences")));
    // 工具栏、托盘菜单、语音浮层一律跟随系统颜色模式。
    auto toolbar_enabled = std::make_shared<std::atomic<bool>>(
        prepared.at("value").at("preferences")
            .value("floating_toolbar", nlohmann::json::object())
            .value("enabled", true));
    // 候选窗口跟随光标, likewise published rather than read once.
    auto follow_cursor = std::make_shared<std::atomic<bool>>(
        prepared.at("value").at("preferences")
            .value("candidate_follow_cursor", true));
    // 游戏兜底锚点放客户区上部。多数游戏的聊天输入框在左下角，默认的落点就是按那里定的；
    // 输入框在画面上沿的游戏（DNF）把这一项打开，候选窗才落在输入处附近。
    auto game_anchor_top = std::make_shared<std::atomic<bool>>(
        prepared.at("value").at("preferences")
            .value("game_compatibility", nlohmann::json::object())
            .value("overlay_anchor_top", false));
    // 「输入时显示双拼键位提示」（共享偏好 `shuangpin_keymap_hint`，没选过时文档里没有这一项，按关处理），同样经原子量发布，改了设置不用重启 Server。
    auto shuangpin_keymap_enabled = std::make_shared<std::atomic<bool>>(
        shuangpin_keymap_preference(prepared.at("value").at("preferences")));
    // 「鼠标滚轮翻页」同样经原子量发布，候选窗每轮取用，改了设置不用重启 Server。
    auto candidate_mouse_wheel = std::make_shared<std::atomic<bool>>(
        config.navigation.mouse_wheel);
    // The TSF Ctrl+Shift+F route is delivered through the same bounded worker
    // as the toolbar button. It must exist before WindowsServer construction:
    // a newly connected client may dispatch its first key immediately.
    startup_stage = "workers";
    InputStateHudFeedback hud_feedback;
    // 提示由别的线程发布或改变，界面线程绘制，而界面线程在消息循环里最多等 50 毫秒才看一轮。给界面线程投一条空消息把这一轮叫醒，提示不必等满这一拍才出现。两处要叫醒：信箱里发布了提示（切简繁、日语、清缓存这些工作线程），以及管道上的模式事件落地（单按 Shift 切中英文走的是这条，界面线程靠读模式才知道要出提示，见下面的 pump_presentation.mode_event）。
    const DWORD ui_thread = GetCurrentThreadId();
    const auto wake_ui_thread = [ui_thread] { PostThreadMessageW(ui_thread, WM_NULL, 0, 0); };
    hud_feedback.set_wake(wake_ui_thread);
    InputStateHudModeTracker hud_mode_tracker;
    CharacterSetClickWorker character_set_clicks(
        [traditional_output, &hud_feedback, directory = config.state_root](
            const CharacterSetClick &click) {
          const auto foreground = click.foreground ? click.foreground :
              reinterpret_cast<uintptr_t>(GetForegroundWindow());
          std::optional<bool> changed;
          if (persist_traditional_output(directory, *traditional_output, click.desired, &changed) && changed)
            hud_feedback.publish(character_set_hud(*changed), {}, foreground);
        });
    auto candidate_fonts = std::make_shared<CandidateFontMailbox>();
    auto candidate_style = std::make_shared<CandidateWindowStyleMailbox>();
    auto toolbar_settings = std::make_shared<FloatingToolbarMailbox>();
    auto candidate_theme = std::make_shared<CandidateThemeMailbox>();
    // logo 开关、页码开关和释义预留行数从启动时的偏好取，第一次发布之前候选卡片就按它们排版。预留行数还要看装了哪些离线释义词典，启动时查一次。
    const auto offline_gloss_languages = installed_offline_gloss_languages(config.resources);
    const auto stored_candidate_layout =
        candidate_layout_settings(prepared.at("value").at("preferences"), offline_gloss_languages)
            .value_or(CandidateLayoutSettings{});
    auto candidate_layout = std::make_shared<std::atomic<unsigned>>(
        CandidateLayoutSettings{
            config.horizontal_candidates, config.candidate_show_preedit, true,
            stored_candidate_layout.show_app_logo,
            stored_candidate_layout.reserved_gloss_lines,
            stored_candidate_layout.show_page_number}
            .encode());
    WindowsServerOptions options;
    options.pipes.names = production ? production_pipe_names()
                                     : config.pipe_names();
    // GameHostCandidate 在开发版和预览版里也宣告，否则它们永远走不到游戏会话的兜底定位和独占抑制。FieldTraits 同理：没有它，密码框里的音乐和提权窗口的属性在开发版里无从验证。
    options.pipes.capabilities =
        production ? (FanyImeProtocol::Capabilities |
                      FanyImeProtocol::CharacterSetShortcut |
                      FanyImeProtocol::CandidateContinuation |
                      FanyImeProtocol::UiLessCandidates |
                      FanyImeProtocol::HostOnlineCandidates |
                      FanyImeProtocol::GroupedPanelKeys |
                      FanyImeProtocol::OrderedPreferences |
                      FanyImeProtocol::GameHostCandidate |
                      FanyImeProtocol::FieldTraits |
                      FanyImeProtocol::QuickPhraseCursor)
                   : (FanyImeProtocol::RequiredCapabilities |
                      FanyImeProtocol::CandidateContinuation |
                      FanyImeProtocol::UiLessCandidates |
                      FanyImeProtocol::HostOnlineCandidates |
                      FanyImeProtocol::GroupedPanelKeys |
                      FanyImeProtocol::OrderedPreferences |
                      FanyImeProtocol::GameHostCandidate |
                      FanyImeProtocol::FieldTraits | FanyImeProtocol::QuickPhraseCursor);
    // 管道对端身份对不上时记下 pid 和错误码，用来区分「DLL 加载了但 Server 拒了连接」和「DLL 根本没加载」，比如反作弊剥掉了 Server 打开游戏进程所需的权限。
    // 在握手线程上运行，而 notice 会写盘：反作弊持续拒绝时 TSF 每次重连都会走到这里，所以同一个 pid 一分钟只记一条，免得拖慢排队中的握手。
    auto identity_rejected_logged =
        std::make_shared<std::pair<std::mutex, std::unordered_map<DWORD, uint64_t>>>();
    options.pipes.identity_rejected = [&notice, identity_rejected_logged](DWORD pid, DWORD error) {
      const uint64_t now = GetTickCount64();
      {
        std::lock_guard<std::mutex> lock(identity_rejected_logged->first);
        auto &logged = identity_rejected_logged->second;
        const auto found = logged.find(pid);
        if (found != logged.end() && now - found->second < 60000)
          return;
        if (logged.size() >= 64)
          logged.clear();
        logged[pid] = now;
      }
      notice("Pipe identity rejected: pid=" + std::to_string(pid) +
             " error=" + std::to_string(error));
    };
    options.preferences_directory = config.state_root.u8string();
    options.preferences_published =
        [&, traditional_output,
         toolbar_enabled, follow_cursor, game_anchor_top, shuangpin_keymap_enabled, candidate_mouse_wheel, candidate_fonts, candidate_style,
         mode_scope_global, app_mode_rules,
         input_mode_hud_enabled, input_mode_hud_logo, tsf_config, candidate_layout,
         tsf_config_mutex, tray_preferences, tray_preferences_mutex,
         tsf_config_revision, candidate_theme, toolbar_settings,
         language_dictionaries](const PreferenceSnapshot &snapshot) {
          const auto preferences =
              nlohmann::json::parse(snapshot.serialized()).at("preferences");
          candidate_theme->publish(preferences);
          if (auto settings = floating_toolbar_settings(preferences))
            toolbar_settings->publish(snapshot.revision(), *settings);
          if (auto fonts = candidate_font_settings(preferences))
            candidate_fonts->publish(snapshot.revision(), std::move(*fonts));
          if (auto style = candidate_window_style(preferences))
            candidate_style->publish(snapshot.revision(), *style);
          if (auto layout = candidate_layout_settings(preferences, offline_gloss_languages))
            candidate_layout->store(layout->encode(), std::memory_order_release);
          traditional_output->store(
              preferences.value("traditional_chinese_output", false),
              std::memory_order_release);
          mode_scope_global->store(
              preferences.value("ime_mode_scope", std::string("app")) ==
                  "global",
              std::memory_order_release);
          {
            auto rules = ReadAppInputModeRules(preferences);
            std::lock_guard<std::mutex> lock(app_mode_rules->first);
            app_mode_rules->second = std::move(rules);
          }
          input_mode_hud_enabled->store(preferences.value("input_mode_hud", true),
                                        std::memory_order_release);
          input_mode_hud_logo->store(hud_show_app_logo(preferences), std::memory_order_release);
          // Publish the TSF-local settings; the loop pushes them to the
          // focused TIP, since the server is constructed after this handler.
          {
            std::lock_guard<std::mutex> lock(*tsf_config_mutex);
            const bool dedicated_english = tsf_config->dedicated_english;
            *tsf_config = tsf_local_config(preferences, language_dictionaries);
            tsf_config->dedicated_english = dedicated_english;
            tsf_config->preference_revision = snapshot.revision();
            tsf_config_revision->mark_changed();
          }
          {
            auto menu = tray_menu_preferences(preferences, language_dictionaries);
            std::lock_guard<std::mutex> lock(*tray_preferences_mutex);
            *tray_preferences = std::move(menu);
          }
          // The settings page owns this too; without reconciling it here the
          // toolbar only followed the preference across a restart.
          const auto toolbar_preferences =
              preferences.value("floating_toolbar", nlohmann::json::object());
          toolbar_enabled->store(toolbar_preferences.value("enabled", true),
                                 std::memory_order_release);
          follow_cursor->store(
              preferences.value("candidate_follow_cursor", true),
              std::memory_order_release);
          game_anchor_top->store(
              preferences.value("game_compatibility", nlohmann::json::object())
                  .value("overlay_anchor_top", false),
              std::memory_order_release);
          shuangpin_keymap_enabled->store(shuangpin_keymap_preference(preferences),
                                          std::memory_order_release);
          candidate_mouse_wheel->store(
              preferences.value("navigation", nlohmann::json::object())
                  .value("mouse_wheel", false),
              std::memory_order_release);
        };
    auto session_options = prepared.at("value");
    name_builtin_helpcodes(session_options, config.state_root);
    startup_stage = "pipe_server";
    SessionPump::Presentation pump_presentation;
    pump_presentation.delivered = [&](const FocusLease &lease, const PendingReply &reply,
                                     const FanyImeNamedpipeData &) {
      if (reply.dedicated_english_change)
        hud_feedback.publish(dedicated_english_hud(*reply.dedicated_english_change), lease);
    };
    // 模式事件落地、焦点锁放开之后叫醒界面线程，它这一轮就能读到新模式并出中英切换提示。
    pump_presentation.mode_event = [wake_ui_thread](const FocusLease &) { wake_ui_thread(); };
    // 单个按键被拒（RequestRejected）只断开那一个客户端，原因写进 Server 日志；不然日志里只剩随后的重连，看不出是哪一项校验没过。
    pump_presentation.rejected = [&notice](const std::string &line) {
      notice(line);
    };
    WindowsServer server(
        options, session_options.dump(),
        production
              ? production_key_handler(
                  [&character_set_clicks](bool desired) {
                    return character_set_clicks.submit(
                        CharacterSetClick{desired, reinterpret_cast<uintptr_t>(GetForegroundWindow())});
                  })
            : preview_key_handler(config),
        [&](const FocusRoute &route, const FanyImeNamedpipeData &packet) {
          if (!route.route) return true;
          std::optional<InputStateHudText> feedback;
          switch (packet.event_type) {
          case FanyImePipeEventType::StatusSnapshot:
          case FanyImePipeEventType::FocusRestored:
            (void)hud_mode_tracker.observe(*route.route, packet.modifiers_down != 0,
                                          packet.pinyin_length != 0, false);
            break;
          case FanyImePipeEventType::DoubleSingleByteSwitch:
            feedback = hud_mode_tracker.observe(*route.route, packet.keycode != 0, {}, true);
            break;
          case FanyImePipeEventType::PuncSwitch:
            feedback = hud_mode_tracker.observe(*route.route, {}, packet.keycode != 0, true);
            break;
          }
          if (feedback) hud_feedback.publish(std::move(*feedback), *route.route);
          return true;
        }, std::move(pump_presentation));
    // 托盘和 TSF 的偏好写入方都已移除，但设置窗口是另一个进程直接改文件；
    // Server 构造期间落盘的写入可能错过监视器回调，构造完再唤醒一次补查。
    auto wake_preferences =
        std::make_shared<std::pair<std::mutex, std::function<void()>>>();
    // Server 构造期间已可接收切换；安装与调用共用锁，异常展开也先撤销并等完旧回调。
    struct PreferenceWakeBinding {
      decltype(wake_preferences) &callback;
      ~PreferenceWakeBinding() {
        std::lock_guard lock(callback->first);
        callback->second = {};
      }
    } preference_wake_binding{wake_preferences};
    {
      std::lock_guard lock(wake_preferences->first);
      wake_preferences->second = [&server] { server.wake_preferences(); };
      // 补查构造期间已完成、当时尚无回调接收的偏好写入。
      wake_preferences->second();
    }
    CandidateClickWorker clicks([&](const CandidateClick &click) {
      if (click.action == CandidateAction::Select) {
        if (server.request_selection(click.lease, click.session,
                                     click.generation, click.index,
                                     click.translation_page,
                                     click.expanded_text) ==
            SelectionRequestResult::Failed)
          throw std::runtime_error("Candidate selection failed");
        return;
      }
      const auto acted = server.request_candidate_action(
          click.lease, click.session, click.generation, click.index,
          click.translation_page, click.action, click.position);
      if (acted == CandidateActionRequestResult::Failed)
          throw std::runtime_error("Candidate action failed");
    });
    CandidatePageWorker pages([&](const CandidatePage &page) {
      if (server.request_page(page) == CandidatePageRequestResult::Failed)
        throw std::runtime_error("Candidate paging failed");
    });
    ModeClickWorker mode_clicks([&](const ModeClick &click) {
      if (click.mode == WorkerMode::English && server.dedicated_english_state(click.lease).value_or(false)) {
        bool changed = false;
        if (!server.exit_dedicated_english(click.lease, &changed))
          throw std::runtime_error("Dedicated English exit failed");
        if (changed) hud_feedback.publish(dedicated_english_hud(false), click.lease);
        return;
      }
      if (server.request_mode(click.lease, click.mode) == ModeRequestResult::WriteFailed)
        throw std::runtime_error("Mode request failed");
    });
    DedicatedEnglishMailbox english_state;
    SingleClickWorker<FocusLease> english_reads([&](const FocusLease &lease) {
      if (auto value = server.dedicated_english_state(lease))
        english_state.publish(lease, *value);
    });
    uint64_t english_read_at = 0;
    struct ClickShutdown {
      WindowsServer &server;
      CandidateClickWorker &clicks;
      CandidatePageWorker &pages;
      ModeClickWorker &modes;
      CharacterSetClickWorker &character_sets;
      SingleClickWorker<FocusLease> &english;
      ~ClickShutdown() {
        clicks.request_stop();
        pages.request_stop();
        modes.request_stop();
        character_sets.request_stop();
        english.request_stop();
        server.request_stop();
        clicks.stop();
        pages.stop();
        modes.stop();
        character_sets.stop();
        english.stop();
      }
    } click_shutdown{server, clicks, pages, mode_clicks, character_set_clicks, english_reads};
    // 托盘「英文候选模式」：在焦点会话上设置 Engine 的英文模式，最多等 2 秒，所以不在 UI 线程上做。成功后立刻发布，下一轮循环就把新状态推给 TIP（DedicatedEnglishChanged），重新打开的卡片也不必等 250 毫秒一次的读取。声明在 click_shutdown 之后，先于它析构，等它做完时 Server 仍在运行。
    SingleClickWorker<DedicatedEnglishSwitch> english_switches(
        [&](const DedicatedEnglishSwitch &request) {
          bool changed = false;
          if (server.set_dedicated_english(request.lease, request.enabled, &changed)) {
            english_state.publish(request.lease, request.enabled);
            if (changed)
              hud_feedback.publish(dedicated_english_hud(request.enabled), request.lease);
          }
        });
    startup_stage = "candidate_window";
    DesktopCandidateSettings candidate_settings;
    // 启动配置里没有字重，先按常规画；读到偏好后字重随 candidate_font_settings 一起发过来。
    candidate_settings.fonts = {config.candidate_font, config.candidate_fallback_fonts,
        static_cast<unsigned>(config.candidate_font_size),
        static_cast<unsigned>(config.candidate_preedit_font_size),
        candidate_regular_font_weight};
    candidate_settings.layout = CandidateLayoutSettings::decode(candidate_layout->load(std::memory_order_acquire));
    candidate_settings.mouse_wheel = candidate_mouse_wheel->load(std::memory_order_acquire);
    DesktopCandidates candidates({
        [&] { return server.candidate_view(); },
        [&](const CandidateClick &click) { (void)clicks.submit(click); },
        [&](const CandidatePage &page) { (void)pages.submit(page); },
        [&](const CandidatePresentation &value) { server.candidate_rendered(value.lease, value.render_serial); },
        [&](const FocusLease &lease, bool open) { server.set_candidate_expansion(lease, open); },
        [&](const FocusLease &lease) { return server.candidate_lease_current(lease); }});
    // 空格、数字键和点选的渲染屏障要等回执时叫醒候选窗立即绘制，屏障的 30 毫秒只花在真正的绘制上，不花在两级 16 毫秒的轮询上（#9519）。守卫声明在 candidates 之后、先于它析构：撤掉之后 Server 不会再调用这个回调。
    server.set_candidate_render_request([&candidates] { candidates.wake(); });
    struct CandidateRenderRequestGuard {
      WindowsServer &server;
      ~CandidateRenderRequestGuard() { server.set_candidate_render_request({}); }
    } candidate_render_request_guard{server};
    // The global theme colours the card, the toolbar and the menus. 皮肤包已移除：只按明暗取原生配色，不再向共享层解析皮肤资源。
    startup_stage = "candidate_theme";
    auto current_candidate_theme = candidate_theme_values(
        prepared.at("value").at("preferences"));
    auto resolved_theme = [](bool, bool, bool = false)
        -> const CandidateThemeResolution & {
      static const CandidateThemeResolution empty;
      return empty;
    };
    auto card_palette = [&](const CandidateThemeResolution &theme, bool dark) {
      return candidate_theme_palette(theme, dark);
    };
    bool candidate_theme_dirty = true;
    bool candidate_dark_applied = config.dark_theme;
    bool candidate_horizontal_applied = config.horizontal_candidates;
    // Bumped whenever the card's theme is re-resolved, so the toolbar and the menu follow it.
    uint64_t candidate_theme_generation = 0;
    uint64_t candidate_theme_check_at = 0;
    bool system_dark = system_prefers_dark();
    {
      const auto &theme =
          resolved_theme(config.dark_theme, config.horizontal_candidates);
      auto palette = card_palette(theme, config.dark_theme);
      auto keymap_palette = palette;
      if (config.candidate_selected_bar)
        palette.show_selected_bar = keymap_palette.show_selected_bar =
            *config.candidate_selected_bar;
      candidate_settings.palette = palette;
      candidate_settings.keymap_palette = keymap_palette;
    }
    candidate_settings.follow_cursor = follow_cursor->load(std::memory_order_acquire);
    candidate_settings.game_anchor_top = game_anchor_top->load(std::memory_order_acquire);
    candidate_settings.keymap_enabled = shuangpin_keymap_enabled->load(std::memory_order_acquire);
    candidates.start(candidate_settings);
    // The toolbar and the menus draw the card's theme in their own light/dark mode.
    auto surface_palette = [&](bool dark) {
      return card_palette(resolved_theme(dark, candidate_horizontal_applied), dark);
    };
    auto toolbar_surface_palette = [&](bool dark) {
      return toolbar_palette(card_palette(
          resolved_theme(dark, candidate_horizontal_applied), dark));
    };
    bool toolbar_visible = toolbar_enabled->load(std::memory_order_acquire);
    startup_stage = "floating_toolbar";
    FloatingToolbarWindow toolbar(
        [&] { return server.mode_view(); },
        [&](const ModeClick &click) { (void)mode_clicks.submit(click); });
    // The toolbar follows the global theme in its own light/dark mode.
    bool toolbar_dark_applied = system_dark;
    uint64_t toolbar_theme_applied = candidate_theme_generation;
    toolbar.set_palette(toolbar_surface_palette(toolbar_dark_applied));
    // 中英文切换提示，穿工具栏的配色，按 `show_app_logo` 画不画 logo；尺寸是自己的固定紧凑尺寸，不跟工具栏的字号和缩放。建不出窗口时记一条诊断，Server 照常服务输入，和工具栏失败时一样。
    std::optional<InputModeHudWindow> mode_hud;
    try {
      mode_hud.emplace();
      mode_hud->set_palette(toolbar_surface_palette(toolbar_dark_applied));
      mode_hud->set_show_logo(input_mode_hud_logo->load(std::memory_order_acquire));
    } catch (const std::exception &) {
      mode_hud.reset();
      notice("Input mode HUD unavailable; continuing without it");
    }
    bool mode_hud_failure_reported = false;
    // 应用例外按焦点客户端的进程查，结果留到换客户端为止，免得每一轮都去开进程句柄。
    uint32_t mode_rule_pid = 0;
    std::optional<bool> mode_rule;
    toolbar.set_scale(config.floating_toolbar_scale);
    toolbar.set_font_size(config.floating_toolbar_font_size);
    toolbar.set_items(config.floating_toolbar_items);
    // 共享偏好里的切换输入方案、手写、语音按钮和 logo 开关不在启动配置里。第一份偏好快照要等偏好监视器发布，这里先按已经读到的偏好画，免得工具栏先以旧的按钮组出现、随即变宽。
    if (const auto settings =
            floating_toolbar_settings(prepared.at("value").at("preferences")))
      toolbar.set_settings(*settings);
    // 生产 Server 的位置记在状态目录的单独文件里（FloatingToolbarPosition.h），预览实例仍写在自己的配置文件里。
    const auto toolbar_position_path = config.state_root / floating_toolbar_position_file;
    if (production)
      if (const auto stored = read_private_file(toolbar_position_path, 4096))
        if (const auto position = parse_floating_toolbar_position(*stored)) {
          config.floating_toolbar_x = position->x;
          config.floating_toolbar_y = position->y;
        }
    if (config.floating_toolbar_x && config.floating_toolbar_y)
      toolbar.set_position(POINT{*config.floating_toolbar_x, *config.floating_toolbar_y});
    if (production) {
      toolbar.set_position_changed([toolbar_position_path](POINT position) {
        try {
          write_document_atomic(
              toolbar_position_path,
              serialize_floating_toolbar_position(
                  {static_cast<int>(position.x), static_cast<int>(position.y)}));
        } catch (...) {
          // 写不进去只是下次启动回到默认位置，不能因此停掉输入服务。
        }
      });
    } else {
      toolbar.set_position_changed([&document, &config_path](POINT position) {
        try {
          auto updated = nlohmann::json::parse(document);
          updated["floating_toolbar_x"] = position.x;
          updated["floating_toolbar_y"] = position.y;
          const auto serialized = updated.dump();
          write_document_atomic(config_path, serialized);
          document = serialized;
        } catch (...) {
          // A transient write failure must not tear down the input server.
        }
      });
    }
    toolbar.set_active_reader([&] { return server.mode_active(); });
    toolbar.set_character_set_reader([traditional_output] {
      return std::optional<bool>(
          traditional_output->load(std::memory_order_acquire));
    });
    // Server 自己管悬浮工具栏，其余各行都在另一个进程里打开界面：设置窗口或面板宿主不在本 Server 旁边时，这些行显示但不可用，不接受一次什么都不做的点击。面板宿主由 MSIME_WINDOWS_PANEL_HOST 选定，启动时读一次，可用性判断和实际启动用同一个选择。
    const auto shell_directory = executable_directory();
    const auto configured_shell = configured_shell_command();
    const auto panel_host = configured_panel_host();
    const auto settings_request = shell_surface_request(TrayMenuCommand::OpenSettings);
    const auto settings_shell = settings_request
                                    ? shell_executable(shell_directory, configured_shell,
                                                       *settings_request, panel_host)
                                    : std::nullopt;
    const ShellLaunchContext shell_context{
        config.state_root, config.state_root / L"runtime-options.json"};
    const auto launch_shell = [&](const ShellSurfaceRequest &request) {
      const auto executable =
          shell_executable(shell_directory, configured_shell, request, panel_host);
      return executable && launch_shell_surface(*executable, request, shell_context);
    };
    // 首次准备运行配置后立即打开设置首页。安装器只负责安装文件和启动 Server，
    // 如果这里不接上，Windows 用户必须自己从开始菜单找到设置窗口，输入法的首次设置
    // 就没有完成闭环。设置窗口顶部会继续显示“去添加/设为默认”的状态卡；已有状态的
    // Server 启动（包括 TSF 按需拉起）不重复打扰用户。
    if (prepared_now && settings_request)
      (void)launch_shell(*settings_request);
    toolbar.set_character_set_action([&] {
      (void)character_set_clicks.submit(CharacterSetClick{{}, reinterpret_cast<uintptr_t>(GetForegroundWindow())});
    });
    toolbar.set_shell_available(settings_shell.has_value());
    toolbar.set_settings_action([&] {
      const auto request = shell_surface_request(TrayMenuCommand::OpenSettings);
      if (request) (void)launch_shell(*request);
    });
    TrayMenuCapabilities menu_capabilities;
    menu_capabilities.settings = settings_shell.has_value();
    // 主题、皮肤和在线面板已移除，托盘只保留本地输入相关入口。
    std::vector<TrayMenuTheme> theme_rows;
    // 托盘卡片和悬浮工具栏的两个弹出菜单（切换输入方案、右键实用菜单）共用这一个命令处理和这一份状态，所以同一行从哪里点都走同一条路径。
    const auto tray_command = [&](TrayMenuCommand command) -> bool {
          const auto command_foreground = reinterpret_cast<uintptr_t>(GetForegroundWindow());
          // The mode rows send what the toolbar button for the same mode sends, to the focused TIP, through the same worker. The card never takes focus, so the session the rows were drawn for is still the focused one.
          if (tray_menu_mode_row(command)) {
            const auto view = server.mode_view();
            if (!view)
              return false;
            const auto request = tray_menu_mode_command(
                command, view->chinese, view->fullwidth,
                view->chinese_punctuation,
                english_state.snapshot(view->lease).value_or(false));
            if (!request.known)
              return false;
            if (!request.mode)
              return true;
            return mode_clicks.submit(ModeClick{view->lease, *request.mode});
          }
          if (const char *scheme = tray_menu_scheme(command)) {
            bool changed = false;
            if (!store_input_scheme(config.state_root, scheme, changed))
              return false;
            std::lock_guard<std::mutex> lock(*tray_preferences_mutex);
            tray_preferences->scheme = scheme;
            if (changed)
              hud_feedback.publish(scheme_hud(*tray_preferences), {}, command_foreground);
            return true;
          }
          // 工具栏右键菜单的「隐藏悬浮状态栏」和托盘的工具栏开关写同一个偏好，与 macOS 一样把开关记成关，重启后也不再出现。
          if (command == TrayMenuCommand::ToggleFloatingToolbar ||
              command == TrayMenuCommand::HideFloatingToolbar) {
            if (command == TrayMenuCommand::HideFloatingToolbar && !toolbar_visible)
              return true;
            // Write it back, so the choice survives a restart and the settings
            // page and this row cannot disagree. A store that refuses the write
            // leaves the row unhandled rather than showing a state that was
            // never saved.
            bool next = !toolbar_visible;
            if (!toggle_stored_flag(config.state_root, "floating_toolbar",
                                    "enabled", toolbar_visible, next))
              return false;
            // Publish immediately as well as writing the store: the file
            // monitor reports the change a moment later, and the refresh loop
            // reads this flag, so without it the toolbar would flip back until
            // the monitor caught up.
            toolbar_enabled->store(next, std::memory_order_release);
            toolbar_visible = next;
            if (!toolbar_visible)
              toolbar.hide();
            return true;
          }
          if (command == TrayMenuCommand::OpenSystemEmoji)
            return open_system_emoji_panel();
          // 繁体输出和工具栏的简繁按钮、Ctrl+Shift+F 走同一个工作线程，翻转存储的 traditional_chinese_output。
          if (command == TrayMenuCommand::ToggleTraditionalOutput)
            return character_set_clicks.submit(CharacterSetClick{{}, command_foreground});
          // 英文候选模式按 TIP 报告的状态进入或退出 Engine 的英文模式，规则和 TIP 的 Ctrl+Shift+E 相同（tray_menu_dedicated_english_action）。
          if (command == TrayMenuCommand::ToggleDedicatedEnglish) {
            const auto view = server.mode_view();
            if (!view)
              return false;
            std::string scheme;
            {
              std::lock_guard<std::mutex> lock(*tray_preferences_mutex);
              scheme = tray_preferences->scheme;
            }
            const auto action = tray_menu_dedicated_english_action(
                view->chinese, english_state.snapshot(view->lease).value_or(false),
                scheme);
            if (action == TrayMenuEnglishAction::Unavailable)
              return false;
            return english_switches.submit(DedicatedEnglishSwitch{
                view->lease, action == TrayMenuEnglishAction::Enter});
          }
          const auto request = shell_surface_request(command);
          // Report only what was observed: a row that could not start the
          // shell stays unhandled, so the menu does not close on a promise.
          return request && launch_shell(*request);
    };
    const auto tray_state = [&] {
          TrayMenuState state;
          state.floating_toolbar = toolbar_visible;
          if (const auto view = server.mode_view()) {
            state.chinese = view->chinese;
            state.fullwidth = view->fullwidth;
            state.chinese_punctuation = view->chinese_punctuation;
            state.dedicated_english =
                english_state.snapshot(view->lease).value_or(false);
          }
          {
            std::lock_guard<std::mutex> lock(*tray_preferences_mutex);
            state.scheme = tray_preferences->scheme;
            state.shuangpin_profile = tray_preferences->shuangpin_profile;
            state.language_hint = tray_preferences->language_hint;
            state.character_set_shortcut = tray_preferences->character_set_shortcut;
            state.punctuation_locked = tray_preferences->punctuation_locked;
          }
          state.traditional_output =
              traditional_output->load(std::memory_order_acquire);
          // candidate_theme_values keeps global_theme only when it is a string.
          state.theme = current_candidate_theme.value("global_theme", std::string("system"));
          state.themes = theme_rows;
          return state;
    };
    const auto tray_card_state = tray_state;
    startup_stage = "tray_menu";
    TrayMenuWindow tray(menu_capabilities, tray_command, tray_card_state);
    // 悬浮工具栏的两个弹出菜单，画法与托盘卡片相同。方案菜单只放「输入方案」那一组行，缺词典跑不起来的方案不列出，与 macOS 工具栏的方案菜单一样；右键菜单是 macOS 设置按钮的实用菜单。
    TrayMenuWindow scheme_menu(
        [&] { return tray_menu_scheme_items(menu_capabilities, tray_state(), true); },
        tray_command);
    TrayMenuWindow utility_menu(
        [&] { return toolbar_utility_menu_items(menu_capabilities); }, tray_command);
    // 弹出菜单和托盘卡片一样不抢焦点，靠下面的循环轮询指针决定何时收起（tray_menu_dismissal）。`dismissed_at` 记下轮询收起它的时刻：在打开它的按钮上再按一下时，按下那一刻轮询已经把它收起，松开时不应当又把它打开。
    struct ToolbarPopup {
      TrayMenuWindow &menu;
      uint64_t shown_at = 0;
      uint64_t pointer_left_at = 0;
      uint64_t dismissed_at = 0;
      HWND foreground = nullptr;
    };
    ToolbarPopup scheme_popup{scheme_menu};
    ToolbarPopup utility_popup{utility_menu};
    // The menu follows the global theme in its own light/dark mode, like the toolbar.
    bool menu_dark_applied = system_dark;
    uint64_t menu_theme_applied = candidate_theme_generation;
    tray.set_palette(tray_menu_palette(surface_palette(menu_dark_applied)));
    // 再按一次打开它的按钮就收起；刚被轮询收起的不立刻重开。打开时取菜单当前的明暗配色，另一个弹出菜单和托盘卡片先收起。
    const auto toggle_popup = [&](ToolbarPopup &popup, ToolbarPopup &other,
                                  const ToolbarMenuAnchor &anchor) {
      const uint64_t now = GetTickCount64();
      if (popup.menu.visible()) {
        popup.menu.hide();
        return;
      }
      if (now - popup.dismissed_at < tray_menu_debounce_milliseconds)
        return;
      other.menu.hide();
      tray.hide();
      popup.menu.set_palette(tray_menu_palette(surface_palette(menu_dark_applied)));
      if (popup.menu.open_beside(anchor.center_x, anchor.top, anchor.bottom)) {
        popup.shown_at = now;
        popup.pointer_left_at = now;
        popup.foreground = GetForegroundWindow();
      }
    };
    toolbar.set_input_scheme_action([&](const ToolbarMenuAnchor &anchor) {
      toggle_popup(scheme_popup, utility_popup, anchor);
    });
    toolbar.set_context_menu_action([&](const ToolbarMenuAnchor &anchor) {
      toggle_popup(utility_popup, scheme_popup, anchor);
    });
    toolbar.set_hide_action([&] { (void)tray_command(TrayMenuCommand::HideFloatingToolbar); });
    // 空闲隐藏：按键（下面的维护钩子报告，加上 Server 经管道收到的按键）、在工具栏上按鼠标都重新计时。
    FloatingToolbarIdleTimer toolbar_idle;
    toolbar.set_activity_action([&] { toolbar_idle.note_input(GetTickCount64()); });
    // The Server is the Caps Lock authority: the TIP only sampled GetKeyState
    // at activation, so pressing Caps mid-session left its indicator stale.
    ModeAuthorityState mode_authority;
    // Seeded from the configured default, and marked as seeded so the first
    // client does not silently become the authority. 默认输入状态 is documented
    // as the state a new focus session starts in, so pushing it to the first
    // client is the behaviour that option promises.
    mode_authority.chinese =
        prepared.at("value").at("preferences")
            .value("default_ime_mode", std::string("chinese")) != "english";
    mode_authority.seeded = true;
    std::atomic<bool> caps_lock{(GetKeyState(VK_CAPITAL) & 1) != 0};
    msime::windows::RevisionFence caps_lock_revision;
    // Starts true: the Server is launched by the TIP, so the IME is active by
    // the time this runs, and waiting for the first edge would hide the toolbar
    // until the user switched focus once.
    std::atomic<bool> ime_active{true};
    // The language bar sends a right click over the Aux pipe; without a
    // listener the tray menu - and with it every shared-shell entry - is
    // unreachable. A failure here costs the menu, never the IME.
    TrayMenuMailbox tray_mailbox;
    DWORD aux_error = ERROR_SUCCESS;
    const std::wstring aux_name =
        production ? FANY_IME_AUX_NAMED_PIPE : config.aux_pipe_name();
    auto aux = AuxListener::create(
        aux_name,
        [&tray_mailbox](const TrayMenuAnchor &anchor) {
          tray_mailbox.publish(anchor);
        },
        aux_error,
        [](const std::wstring &message) {
          if (message == L"RestartServer") {
            restart_requested.store(true);
            stopping.store(true);
          }
        },
        [&ime_active](AuxActivation activation) {
          ime_active.store(activation == AuxActivation::Activated,
                           std::memory_order_release);
        },
        // The DLL falls back to this when its Main-pipe deactivate write
        // fails, then polls for a literal "OK" for up to 150 ms - blocking the
        // sending TSF thread for that whole window when nobody answers. The
        // answer is only sent once the named client really is not focused
        // under that token, so an "OK" never reports a teardown that did not
        // happen.
        [&server](const AuxTerminalDeactivation &terminal) {
          return server.deactivate_terminal(terminal.client_id,
                                            terminal.focus_token);
        },
        // Dictionary maintenance runs in the settings process and needs the
        // exclusive lock every Engine session holds a share of. Releasing it
        // means dropping the sessions: the Engine has those files open, and
        // leaving it alive while they are swapped underneath would have it
        // reading a tree that no longer exists. The clients stay connected
        // and get a session back on resume.
        [&server](AuxDictionaryMaintenance request) {
          return request == AuxDictionaryMaintenance::Quiesce
                     ? server.quiesce_dictionaries()
                     : server.resume_dictionaries();
        });
    if (!aux)
      notice("Tray menu unavailable: language bar endpoint not started");
    // The four shortcuts the shared settings page documents. They must work
    // while another application has focus, so they sit on a low-level keyboard
    // hook rather than the TSF key sink.
    MaintenanceHotkeyController maintenance([&](MaintenanceHotkey hotkey) {
      // 几个版本的 Server 同时运行时，各自的低级键盘钩子都会看到这个按键，后装的钩子先看到，处理了就吞掉。焦点上的 TIP 属于别的版本时交给下一个钩子，让那个版本的 Server 处理；没有任何版本的模式活动时（焦点在别的输入法上，或 TIP 会话在崩溃后断开，正是要用重启快捷键的时候）谁先看到谁处理，不能都放过。只装一个版本时与引入版本之前相同。
      if (hotkey.action != MaintenanceAction::DeleteCandidate &&
          !server.mode_active() && other_edition_mode_active())
        return false;
      switch (hotkey.action) {
      case MaintenanceAction::Restart:
        restart_requested.store(true);
        stopping.store(true);
        return true;
      case MaintenanceAction::Stop:
        stop_requested.store(true);
        stopping.store(true);
        return true;
      case MaintenanceAction::ClearCache: {
        const auto foreground = reinterpret_cast<uintptr_t>(GetForegroundWindow());
        if (!server.reset_cache()) return false;
        hud_feedback.publish({L"缓存已清理", L"输入缓存已清理"}, {}, foreground);
        return true;
      }
      case MaintenanceAction::DeleteCandidate: {
        // Only meaningful while a candidate list is on screen; otherwise the
        // stroke belongs to the focused application and must not be eaten.
        const auto view = server.candidate_view();
        if (!view || !view->visible || hotkey.slot >= view->candidates.size())
          return false;
        const auto &candidate = view->candidates[hotkey.slot];
        const bool removed = server.request_candidate_action(
                                 view->lease, candidate.session, candidate.generation,
                                 candidate.index, view->translation_page,
                                 CandidateAction::Remove) ==
                             CandidateActionRequestResult::Sent;
        return removed;
      }
      }
      return false;
    },
    [&](bool caps) {
      // The Server owns the indicator; publish and let the loop deliver it, so
      // the hook callback never touches the transport.
      caps_lock.store(caps, std::memory_order_release);
      caps_lock_revision.mark_changed();
    },
    // 每次真实按键都让空闲隐藏的工具栏回来并重新计时，与 macOS 每个按键都唤醒工具栏一致。
    [&] { toolbar_idle.note_input(GetTickCount64()); });
    if (!maintenance.installed())
      notice("Maintenance shortcuts unavailable; continuing without them");
    uint64_t tray_shown_at = 0;
    uint64_t pointer_left_at = 0;
    HWND tray_foreground = nullptr;
    // Keep the operator-facing status truthful in both launch modes. The
    // managed Server uses the production TSF pipe, while the preview binary
    // uses an isolated endpoint; conflating them makes support logs suggest
    // that a preview instance is serving the installed input method.
    std::cout << (production ? "Production" : "Preview")
              << " Server running; candidate selection and mode controls enabled.\n";
    // 工具栏失败不结束 Server：它只是方便切换模式的附件，Server 记一条诊断、去掉工具栏继续服务输入。曾经它也在这个条件里，某台 Windows 11 上工具栏一失败 Server 就在启动后约 100 ms 退出，日志却只写了一句正常停止。
    bool toolbar_failure_reported = false;
    // 主桌面的前台呈现方式变化与 QUNS 耗时；候选窗的诊断由所属桌面线程汇总。
    auto logged_presentation = ForegroundPresentation::Windowed;
    bool quns_cost_logged = false;
    uint64_t tsf_config_applied_revision = 0;
    uint64_t caps_lock_applied_revision = 0;
    startup_stage = "message_loop";
    while (!stopping.load() && server.failure() == ControllerFailure::None &&
           !candidates.failed() && !clicks.failed() && !pages.failed() &&
           !mode_clicks.failed() &&
           !character_set_clicks.failed() && !english_reads.failed() &&
           !english_switches.failed()) {
      if (!toolbar_failure_reported && toolbar.failed()) {
        toolbar_failure_reported = true;
        toolbar.hide();
        notice(component_failure("Floating toolbar", toolbar.failure_site()) +
               "; continuing without it");
      }
      MSG message{};
      // Bound each batch so a message flood cannot starve stop/focus polling.
      for (size_t i = 0;
           i < 64 && PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE); ++i) {
        if (message.message == WM_QUIT) {
          stopping.store(true);
          break;
        }
        TranslateMessage(&message);
        DispatchMessageW(&message);
      }
      if (stopping.load())
        break;
      if (auto fonts = candidate_fonts->take())
        candidate_settings.fonts = *fonts;
      if (auto style = candidate_style->take())
        candidate_settings.style = *style;
      const auto next_candidate_layout = CandidateLayoutSettings::decode(
          candidate_layout->load(std::memory_order_acquire));
      candidate_settings.layout = next_candidate_layout;
      if (auto theme = candidate_theme->take()) {
        if (*theme != current_candidate_theme) {
          current_candidate_theme = std::move(*theme);
          candidate_theme_dirty = true;
        }
      }
      const bool candidate_horizontal = next_candidate_layout.horizontal;
      const auto theme_now = GetTickCount64();
      if (candidate_theme_dirty || candidate_horizontal != candidate_horizontal_applied ||
          theme_now >= candidate_theme_check_at) {
        candidate_theme_check_at = theme_now + 500;
        system_dark = system_prefers_dark();
        const bool dark =
            candidate_theme_dark(current_candidate_theme, system_dark);
        if (candidate_theme_dirty || dark != candidate_dark_applied ||
            candidate_horizontal != candidate_horizontal_applied) {
          const auto &theme = resolved_theme(dark, candidate_horizontal);
          auto next_palette = card_palette(theme, dark);
          auto keymap_palette = next_palette;
          if (config.candidate_selected_bar)
            next_palette.show_selected_bar = keymap_palette.show_selected_bar =
                *config.candidate_selected_bar;
          candidate_settings.palette = next_palette;
          candidate_settings.keymap_palette = keymap_palette;
          candidate_dark_applied = dark;
          candidate_horizontal_applied = candidate_horizontal;
          candidate_theme_dirty = false;
          ++candidate_settings.appearance_revision;
          ++candidate_theme_generation;
        }
      }
      // 前台和它的呈现方式每轮只算一次：候选窗的兜底定位、独占抑制、锁存、置顶和工具栏的全屏判断都用这一份。
      const HWND foreground = GetForegroundWindow();
      std::optional<uint64_t> quns_microseconds;
      const auto presentation = foreground_presentation(
          foreground, GetTickCount64(),
          quns_cost_logged ? nullptr : &quns_microseconds);
      if (quns_microseconds) {
        quns_cost_logged = true;
        notice("Foreground QUNS query took " +
               std::to_string(*quns_microseconds) + " us");
      }
      if (presentation != logged_presentation) {
        logged_presentation = presentation;
        notice(std::string("Foreground presentation: ") +
               foreground_presentation_name(presentation));
      }
      for (auto &line : candidates.take_diagnostics())
        notice(std::move(line));
      // The settings page may have published a new value since the last pass.
      toolbar_visible = toolbar_enabled->load(std::memory_order_acquire);
      if (auto settings = toolbar_settings->take())
        toolbar.set_settings(*settings);
      if (mode_hud)
        mode_hud->set_show_logo(input_mode_hud_logo->load(std::memory_order_acquire));
      if (const bool dark = system_dark;
          dark != menu_dark_applied || menu_theme_applied != candidate_theme_generation) {
        menu_dark_applied = dark;
        menu_theme_applied = candidate_theme_generation;
        tray.set_palette(tray_menu_palette(surface_palette(dark)));
      }
      if (const bool dark = system_dark;
          dark != toolbar_dark_applied || toolbar_theme_applied != candidate_theme_generation) {
        toolbar_dark_applied = dark;
        toolbar_theme_applied = candidate_theme_generation;
        toolbar.set_palette(toolbar_surface_palette(dark));
        if (mode_hud)
          mode_hud->set_palette(toolbar_surface_palette(dark));
      }
      // The toolbar is topmost, so without this it floats over full-screen
      // video and presentations. ShouldShowFloatingToolbar was ported long ago
      // but nothing ever supplied its fullscreen argument, leaving the whole
      // predicate dead outside its unit test.
      // Push the TSF-local settings whenever they changed, so turning smart
      // punctuation off takes effect on the text being typed now.
      const auto current_tsf_config_revision = tsf_config_revision->snapshot();
      if (current_tsf_config_revision != tsf_config_applied_revision) {
        if (const auto view = server.mode_view()) {
          msime::windows::TsfLocalConfig pending;
          {
            std::lock_guard<std::mutex> lock(*tsf_config_mutex);
            pending = *tsf_config;
          }
          if (server.send_tsf_config(pending))
            if (tsf_config_revision->is_current(current_tsf_config_revision))
              tsf_config_applied_revision = current_tsf_config_revision;
        }
      }
      // One CN/EN state follows the user between applications when the scope
      // is global. Each TSF client keeps its own mode, so a newly focused one
      // reports whatever it holds and the Server pushes its own back.
      // 应用例外排在作用域前面：焦点进入有规则的应用时推规则里的模式。
      {
        const auto view = server.mode_view();
        const bool focused = view.has_value() && view->chinese.has_value();
        const uint64_t client = focused ? view->lease.transport.client : 0;
        const uint32_t app = focused ? client_pid(view->lease.transport) : 0;
        // 焦点令牌是每个 TIP 线程自己数的，两个应用常拿着同一个数字，所以按客户端和令牌一起认会话。
        if (focused && (client != mode_authority.client ||
                        view->lease.token != mode_authority.session || app != mode_rule_pid)) {
          mode_rule_pid = app;
          mode_rule.reset();
          if (const auto process = process_image_base_name(app)) {
            std::lock_guard<std::mutex> lock(app_mode_rules->first);
            mode_rule = AppInputModeRuleFor(app_mode_rules->second, *process);
          }
        }
        const auto decision = mode_authority_step(
            mode_authority, mode_scope_global->load(std::memory_order_acquire),
            focused, client, view ? view->lease.token : 0,
            view && view->chinese ? *view->chinese : true, app,
            focused ? mode_rule : std::nullopt);
        mode_authority = decision.next;
        // 事务锁正忙（这个客户端的激活或按键正在处理）时 request_mode 什么也不发；记下来下一轮重推，不留着一个没发出去的「已推送」。
        if (decision.push && view &&
            server.request_mode(view->lease, decision.push_chinese ? WorkerMode::Chinese
                                                                   : WorkerMode::English) !=
                ModeRequestResult::Sent)
          mode_authority = mode_authority_push_failed(mode_authority);
        // mode_view 也会因锁忙返回空，仍有活动会话时留到下一轮校验，不能误当失焦丢掉提示。
        std::optional<InputStateHudText> feedback;
        if (view || !server.mode_active())
          feedback = hud_feedback.take(view ? std::optional<FocusLease>(view->lease) : std::nullopt,
                                       reinterpret_cast<uintptr_t>(foreground));
        if (!feedback && decision.user_changed && view && view->chinese) {
          std::lock_guard lock(*tray_preferences_mutex);
          feedback = input_language_hud(*view->chinese, tray_preferences->scheme);
        }
        if (mode_hud && should_show_input_mode_hud(
                input_mode_hud_enabled->load(std::memory_order_acquire) ||
                    (feedback && feedback->answers_action),
                feedback.has_value(),
                presentation != ForegroundPresentation::Windowed)) {
          auto caret = InputModeHudWindow::system_caret(foreground);
          if (!caret)
            if (const auto shown = server.candidate_view();
                view &&
                shown && shown->lease.token == view->lease.token &&
                shown->lease.epoch == view->lease.epoch &&
                same_ticket(shown->lease.transport, view->lease.transport) &&
                shown->caret_anchor_known) {
              const long line = MulDiv(20, static_cast<int>(msime::windows::compat::system_dpi()), USER_DEFAULT_SCREEN_DPI);
              caret = HudRect{shown->caret_anchor_x, shown->caret_anchor_y - line,
                              shown->caret_anchor_x + 1, shown->caret_anchor_y};
            }
          mode_hud->show(std::move(*feedback), caret, foreground);
        }
        if (mode_hud && mode_hud->failed() && !mode_hud_failure_reported) {
          mode_hud_failure_reported = true;
          notice(component_failure("Input mode HUD", mode_hud->failure_site()) +
                 "; continuing without it");
        }
      }
      candidate_settings.follow_cursor =
          follow_cursor->load(std::memory_order_acquire);
      candidate_settings.game_anchor_top =
          game_anchor_top->load(std::memory_order_acquire);
      candidate_settings.mouse_wheel =
          candidate_mouse_wheel->load(std::memory_order_acquire);
      candidate_settings.keymap_enabled = shuangpin_keymap_enabled->load(std::memory_order_acquire);
      candidates.publish(candidate_settings);
      // 语言按钮在 Caps Lock 开着时显示 'A'，日文模式显示 日，韩文模式显示 한，粤拼、注音、越南文、藏文、笔画分别显示 粤、注、越、ཀ、笔（任务栏语言栏图标的藏文仍是「藏」），引擎自己的英文模式显示不带下划线的 "En"，所以它要跟随这些状态。Caps Lock 开着时显示 中 会让用户误判下一个字母键的作用。
      {
        ToolbarLanguageState language;
        language.caps_lock = caps_lock.load(std::memory_order_acquire);
        if (const auto view = server.mode_view()) {
          language.dedicated_english = english_state.snapshot(view->lease).value_or(false);
          const auto now = GetTickCount64();
          if (now >= english_read_at && english_reads.submit(view->lease))
            english_read_at = now + 250;
        }
        {
          std::lock_guard<std::mutex> lock(*tsf_config_mutex);
          language.mode = tsf_config->input_mode;
          // The TIP's V-mode key rule follows the focused session's English mode (Ctrl+Shift+E, the toolbar exit, a focus change): the next pass pushes the trigger frame again.
          if (tsf_config->dedicated_english != language.dedicated_english) {
            tsf_config->dedicated_english = language.dedicated_english;
            tsf_config_revision->mark_changed();
          }
        }
        // 正在运行的方案：语言按钮在双拼和五笔下显示 双 和 五，悬停提示带上方案名。
        {
          std::lock_guard<std::mutex> lock(*tray_preferences_mutex);
          language.scheme = msime::windows::scheme::scheme_from_name(tray_preferences->scheme);
          toolbar.set_scheme_title(toolbar_scheme_title(
              tray_preferences->scheme, tray_preferences->shuangpin_profile, ""));
        }
        toolbar.set_language_state(language);
      }
      const auto current_caps_lock_revision = caps_lock_revision.snapshot();
      if (current_caps_lock_revision != caps_lock_applied_revision) {
        if (const auto view = server.mode_view()) {
          const bool enabled = caps_lock.load(std::memory_order_acquire);
          if (server.send_caps_lock(view->lease, enabled) &&
              caps_lock_revision.is_current(current_caps_lock_revision))
            caps_lock_applied_revision = current_caps_lock_revision;
        }
      }
      const bool fullscreen = presentation != ForegroundPresentation::Windowed;
      // The DLL's activation edges, not the mode view: a temporary focus
      // suspension (Win+. for instance) empties the view without deactivating
      // anything, and gating on the view made the toolbar blink away each time.
      const bool ime_now_active = ime_active.load(std::memory_order_acquire);
      const bool show_toolbar = ShouldShowFloatingToolbar(
          toolbar_visible, fullscreen, ime_now_active);
      // 工具栏的弹出菜单开着时用户正在用它，不算空闲。
      if (scheme_menu.visible() || utility_menu.visible())
        toolbar_idle.note_input(GetTickCount64());
      // 钩子看不到的按键（屏幕键盘等注入的按键、发往提权窗口的按键、钩子没装上）只要经过了 Server，也让工具栏回来。
      toolbar_idle.note_key_activity(ServerKeyActivity::instance().count(), GetTickCount64());
      const bool toolbar_idle_hidden =
          toolbar_idle.idle_hidden(GetTickCount64(), toolbar_visible && ime_now_active);
      toolbar.refresh(show_toolbar && !toolbar_idle_hidden);
      // 工具栏不在了，从它弹出的菜单也不该留着。
      if (!show_toolbar) {
        scheme_menu.hide();
        utility_menu.hide();
      }
      // The listener thread owns no window; the anchor is applied here, on the
      // thread that created the tray card.
      const uint64_t now = GetTickCount64();
      if (const auto anchor = tray_mailbox.take()) {
        switch (tray_menu_request_action(tray.visible(), now, tray_shown_at)) {
        case TrayMenuRequestAction::Show: {
          // The reference presenter resolves its theme on every opening.
          // Sample Windows now rather than relying on the periodic candidate
          // theme check, so a menu opened immediately after a system theme
          // change never flashes the previous palette.
          const bool dark = system_prefers_dark();
          if (dark != menu_dark_applied ||
              menu_theme_applied != candidate_theme_generation) {
            menu_dark_applied = dark;
            menu_theme_applied = candidate_theme_generation;
            tray.set_palette(tray_menu_palette(surface_palette(dark)));
          }
          if (tray.open(anchor->center_x, anchor->top)) {
            tray_shown_at = now;
            pointer_left_at = now;
            tray_foreground = GetForegroundWindow();
          }
          break;
        }
        case TrayMenuRequestAction::Hide:
          tray.hide();
          break;
        case TrayMenuRequestAction::None:
          break;
        }
      }
      if (tray.visible()) {
        const bool inside = tray.pointer_inside();
        if (inside)
          pointer_left_at = now;
        // 用键盘导航时指针多半停在卡片外的托盘图标上，按键也算在用卡片，不按闲置收起。
        pointer_left_at = (std::max)(pointer_left_at, tray.keyboard_activity());
        const bool button_down =
            (GetAsyncKeyState(VK_LBUTTON) | GetAsyncKeyState(VK_RBUTTON)) &
            0x8000;
        if (tray_menu_dismissal(true, now, tray_shown_at, pointer_left_at,
                                inside, button_down != 0,
                                GetForegroundWindow() != tray_foreground))
          tray.hide();
      }
      for (auto *popup : {&scheme_popup, &utility_popup}) {
        if (!popup->menu.visible())
          continue;
        const bool inside = popup->menu.pointer_inside();
        if (inside)
          popup->pointer_left_at = now;
        // 和托盘卡片一样，用键盘导航时指针多半还停在打开它的工具栏按钮上，按键也算在用菜单。
        popup->pointer_left_at =
            (std::max)(popup->pointer_left_at, popup->menu.keyboard_activity());
        const bool button_down =
            (GetAsyncKeyState(VK_LBUTTON) | GetAsyncKeyState(VK_RBUTTON)) & 0x8000;
        if (tray_menu_dismissal(true, now, popup->shown_at, popup->pointer_left_at,
                                inside, button_down != 0,
                                GetForegroundWindow() != popup->foreground)) {
          popup->menu.hide();
          popup->dismissed_at = now;
        }
      }
      if (instance)
        instance->publish_mode_active(server.mode_active());
      if (MsgWaitForMultipleObjectsEx(0, nullptr, 50, QS_ALLINPUT,
                                      MWMO_INPUTAVAILABLE) == WAIT_FAILED)
        throw std::runtime_error("Candidate message wait failed");
    }
    candidates.stop();
    for (auto &line : candidates.take_diagnostics())
      notice(std::move(line));
    // Stop I/O first; it invalidates queued work without waiting for this
    // thread. Retire the matching review before Server/focus teardown.
    toolbar.hide();
    // Stop the listener and close the mailbox before the window goes away, so a
    // late anchor cannot reach a card that is being destroyed.
    if (aux)
      aux->stop();
    tray_mailbox.stop();
    tray.hide();
    clicks.request_stop();
    character_set_clicks.request_stop();
    mode_clicks.request_stop();
    english_reads.request_stop();
    english_switches.request_stop();
    server.stop();
    clicks.stop();
    character_set_clicks.stop();
    mode_clicks.stop();
    english_reads.stop();
    english_switches.stop();
    if (restart_requested.load()) {
      notice("Server stopping: restart requested");
      return msime::windows::watchdog::restart_exit_code;
    }
    if (stop_requested.load()) {
      notice("Server stopping: stop requested");
      return msime::windows::watchdog::stop_exit_code;
    }
    // 每个能结束主循环的组件都要在这一行里点名，否则日志只说"有组件失败"，无从查起。翻页曾经结束主循环却被记成正常停止。
    std::vector<std::string> failures;
    if (const auto failure = server.failure(); failure != ControllerFailure::None)
      failures.push_back(std::string("session controller failed (") +
                         controller_failure_name(failure) + ")");
    if (candidates.failed())
      failures.push_back(component_failure("candidate desktop coordinator", std::nullopt));
    if (clicks.failed())
      failures.push_back(component_failure("candidate click worker", std::nullopt));
    if (pages.failed())
      failures.push_back(component_failure("candidate page worker", std::nullopt));
    if (mode_clicks.failed())
      failures.push_back(component_failure("mode click worker", std::nullopt));
    if (character_set_clicks.failed())
      failures.push_back(component_failure("character set click worker", std::nullopt));
    if (english_reads.failed())
      failures.push_back(component_failure("English state reader", std::nullopt));
    if (english_switches.failed())
      failures.push_back(component_failure("English mode switcher", std::nullopt));
    notice(server_stop_line(failures));
    return failures.empty() ? 0 : 1;
  } catch (...) {
    const std::string line =
        std::string("Server failed at stage=") + startup_stage +
        "; verify configuration, resources, state ownership and pipe availability.";
    std::cerr << line << "\n";
    return 1;
  }
}
