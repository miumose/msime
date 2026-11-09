#include "InputMethodSetup.h"
// windows.h comes first so shellapi.h sees its types; the two macros it defines that collide with WinRT member names are dropped before any WinRT header is read.
#include <windows.h>

#include <shellapi.h>
#include <shobjidl_core.h>
#include "../common/StateDirectory.h"
#include "SettingsInstance.h"
#undef GetCurrentTime
#undef GetObject

#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <cwctype>
#include <filesystem>
#include <fstream>
#include <functional>
#include <initializer_list>
#include <iterator>
#include <limits>
#include <map>
#include <optional>
#include <set>
#include <stdexcept>
#include <string>
#include <string_view>
#include <type_traits>
#include <utility>
#include <vector>

#include "../../../shared/contracts/msime_edition.h"
#include "../../../shared/contracts/windows_ipc.h"
#include "../tsf/IPC/AuxNamedpipeSender.h"
#include "../../common/HostApiString.h"
#include "../common/GameCompatProfile.h"
#include "../common/SurfaceRouteArgument.h"
#include "AboutPageModel.h"
#include "AppInputModeRuleList.h"
#include "CandidateFontPresets.h"
#include "CandidateFontWeight.h"
#include "CandidatePalette.h"
#include "CandidateWindowStyle.h"
#include "FeedbackReport.h"
#include "FloatingToolbarSizeModel.h"
#include "GameProcessList.h"
#include "HelpcodeModesModel.h"
#include "InputSettingsModel.h"
#include "PreferencePath.h"
#include "PreferencesSaveFailure.h"
#include "PunctuationSettings.h"
#include "SettingsDocumentFile.h"
#include "SettingsMascotIntro.h"
#include "SettingsNavIcons.h"
#include "SettingsNavigation.h"
#include "SettingsStateDirectory.h"
#include "SettingsText.h"
#include "ToolbarIcons.h"
#include "ToolbarLayout.h"
#include "../src/input/ToggleHotkeyChord.h"
#include "msime_client.h"

#include <winrt/base.h>
#include <microsoft.ui.xaml.window.h>

#include <winrt/Microsoft.UI.Dispatching.h>
#include <winrt/Microsoft.UI.Input.h>
#include <winrt/Microsoft.UI.Windowing.h>
#include <winrt/Microsoft.UI.Xaml.Automation.Peers.h>
#include <winrt/Microsoft.UI.Xaml.Automation.h>
#include <winrt/Microsoft.UI.Xaml.Controls.Primitives.h>
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.Documents.h>
#include <winrt/Microsoft.UI.Xaml.Input.h>
#include <winrt/Microsoft.UI.Xaml.Markup.h>
#include <winrt/Microsoft.UI.Xaml.Media.Animation.h>
#include <winrt/Microsoft.UI.Xaml.Media.Imaging.h>
#include <winrt/Microsoft.UI.Xaml.Media.h>
#include <winrt/Microsoft.UI.Xaml.Shapes.h>
#include <winrt/Microsoft.UI.Xaml.XamlTypeInfo.h>
#include <winrt/Microsoft.UI.Xaml.h>
#include <winrt/Microsoft.UI.h>
#include <winrt/Windows.ApplicationModel.DataTransfer.h>
#include <winrt/Windows.Data.Json.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.h>
#include <winrt/Windows.Storage.Streams.h>
#include <winrt/Windows.System.h>
#include <winrt/Windows.UI.Text.h>
#include <winrt/Windows.UI.ViewManagement.h>
#include <winrt/Windows.UI.Xaml.Interop.h>
#include <winrt/Windows.UI.h>

using namespace winrt;
using namespace winrt::Microsoft::UI::Xaml;
using namespace winrt::Microsoft::UI::Xaml::Controls;
using namespace winrt::Microsoft::UI::Xaml::Media;
using namespace winrt::Windows::Data::Json;

namespace {

// WRL 同时声明全局 Microsoft/Windows；本文件的界面名称始终指向 WinRT 投影。
namespace Microsoft = winrt::Microsoft;
namespace Windows = winrt::Windows;

// windows.h may declare the COM ::IInspectable as well; this alias always means the WinRT projection.
using Inspectable = winrt::Windows::Foundation::IInspectable;
using Color = winrt::Windows::UI::Color;
using A11y = Microsoft::UI::Xaml::Automation::AutomationProperties;
namespace nav = msime::settings;
namespace ps = msime::settings::punctuation;
namespace ism = msime::settings::input;

struct Response {
  std::string text;
  bool ok = false;
};

std::string utf8(hstring value) { return to_string(value); }

hstring text(std::string_view value) { return to_hstring(std::string(value)); }

Response take_response(char *raw) {
  auto owned = msime::host_api::own_string(raw);
  if (!owned) {
    return {"{\"ok\":false,\"error\":\"empty host response\"}", false};
  }
  std::string value(owned.get());
  try {
    const auto object = JsonObject::Parse(text(value));
    return {std::move(value), object.GetNamedBoolean(L"ok", false)};
  } catch (...) {
    return {std::move(value), false};
  }
}

struct AccountNamePresentation {
  std::wstring normalized;
  std::wstring preferred;
  bool valid = false;
  int32_t maximum_utf16_units = 0;
};

// An environment variable, empty when it is unset. GetEnvironmentVariableW rather than the CRT's _wgetenv, which MSVC deprecates (C4996) and /WX would turn into an error.
std::wstring environment(const wchar_t *name) {
  std::wstring value(64, L'\0');
  for (;;) {
    const DWORD length = GetEnvironmentVariableW(
        name, value.data(), static_cast<DWORD>(value.size()));
    if (length == 0)
      return {};
    if (length < value.size()) {
      value.resize(length);
      return value;
    }
    value.resize(length);
  }
}

std::filesystem::path injected_state_directory() {
  return std::filesystem::path(environment(L"MSIME_CLIENT_STATE_DIR"));
}

std::filesystem::path runtime_options_directory() {
  if (auto injected = injected_state_directory(); injected.is_absolute())
    return injected;
  return msime::windows::resolve_state_directory();
}

std::filesystem::path configured_preferences_directory(
    const std::filesystem::path &directory) {
  if (directory.empty())
    return {};
  std::error_code error;
  const auto file = directory / L"runtime-options.json";
  if (!std::filesystem::is_regular_file(file, error))
    return {};
  std::ifstream stream(file, std::ios::binary);
  const std::string content((std::istreambuf_iterator<char>(stream)),
                            std::istreambuf_iterator<char>());
  JsonObject options{nullptr};
  if (!JsonObject::TryParse(text(content), options) ||
      !options.HasKey(L"preferences_directory"))
    return {};
  const auto value = options.Lookup(L"preferences_directory");
  if (value.ValueType() != JsonValueType::String)
    return {};
  return std::filesystem::path(std::wstring(value.GetString().c_str()));
}

std::filesystem::path state_directory() {
  const auto root = msime::windows::resolve_state_directory();
  return msime::settings::settings_state_directory(
      injected_state_directory(), root, configured_preferences_directory(root));
}

std::string path_utf8(const std::filesystem::path &path) {
  const auto value = path.wstring();
  if (value.empty())
    return {};
  const int length = WideCharToMultiByte(
      CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
      static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
  if (length <= 0)
    return {};
  std::string result(static_cast<size_t>(length), '\0');
  WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
                      static_cast<int>(value.size()), result.data(), length,
                      nullptr, nullptr);
  return result;
}

// WinUI 与 Win7 使用同一套系统状态查询和修复操作。
using msime::settings::input_method::InputMethodState;
using msime::settings::input_method::input_method_state;
using msime::settings::input_method::add_input_method;
using msime::settings::input_method::make_input_method_default;

// 本版本是否提供这个方案（版本表 input_schemes）。不提供的方案不出现在输入方案的选项里。
bool edition_offers_scheme(std::wstring_view scheme) {
  for (const char *name : {MSIME_EDITION_INPUT_SCHEMES}) {
    const std::string_view narrow(name);
    if (scheme.size() == narrow.size() &&
        std::equal(narrow.begin(), narrow.end(), scheme.begin(),
                   [](char left, wchar_t right) { return static_cast<wchar_t>(left) == right; }))
      return true;
  }
  return false;
}

// 保留启动请求的类别，窗口解析承载页时也恢复旧路由对应的顶部标签。
std::string route_page() {
  const auto route = msime::windows::requested_surface_route("settings");
  return route && route->starts_with("settings:") ? route->substr(9) : std::string();
}

// The runtime options file the Server hands this window, or the one the Server reads at startup when started from the Start menu. None before the input method is set up.
std::optional<std::filesystem::path> runtime_options_file() {
  std::error_code error;
  if (const auto raw = environment(L"MSIME_CLIENT_HOST_OPTIONS"); !raw.empty()) {
    std::filesystem::path path(raw);
    if (path.is_absolute() && std::filesystem::is_regular_file(path, error))
      return path;
  }
  const auto directory = runtime_options_directory();
  if (directory.empty())
    return std::nullopt;
  auto path = directory / L"runtime-options.json";
  if (!std::filesystem::is_regular_file(path, error))
    return std::nullopt;
  return path;
}

// The host options the input method runs with; absent until the setup wizard has prepared them.
std::optional<std::string> runtime_options_document() {
  const auto file = runtime_options_file();
  if (!file)
    return std::nullopt;
  std::ifstream stream(*file, std::ios::binary);
  std::string content((std::istreambuf_iterator<char>(stream)),
                      std::istreambuf_iterator<char>());
  JsonObject parsed{nullptr};
  if (content.empty() || !JsonObject::TryParse(text(content), parsed))
    return std::nullopt;
  return content;
}

std::wstring named_string(JsonObject const &object, wchar_t const *key) {
  const auto value = object.GetNamedValue(key, JsonValue::CreateNullValue());
  return value.ValueType() == JsonValueType::String ? std::wstring(value.GetString().c_str())
                                                    : std::wstring();
}

// 本程序的完整路径；第三方组件许可声明按它找安装目录。
std::filesystem::path settings_executable() {
  std::vector<wchar_t> path(32768);
  const DWORD length =
      GetModuleFileNameW(nullptr, path.data(), static_cast<DWORD>(path.size()));
  if (!length || length == path.size())
    return {};
  return std::filesystem::path(std::wstring(path.data(), length));
}

// PreferencePath.h walks preference paths over this: WinRT JSON objects share by reference, so a nested object found on the way is edited in place.
struct JsonPreferenceTree {
  using Object = JsonObject;
  using Value = IJsonValue;
  static std::optional<JsonObject> object(JsonObject const &parent, std::wstring const &name) {
    const hstring key(name);
    if (!parent.HasKey(key))
      return std::nullopt;
    const auto value = parent.Lookup(key);
    if (value.ValueType() != JsonValueType::Object)
      return std::nullopt;
    return value.GetObject();
  }
  static IJsonValue value(JsonObject const &parent, std::wstring const &name) {
    const hstring key(name);
    return parent.HasKey(key) ? parent.Lookup(key) : nullptr;
  }
  static IJsonValue none() { return nullptr; }
  static JsonObject copy(JsonObject const &source) { return JsonObject::Parse(source.Stringify()); }
  static JsonObject empty() { return JsonObject(); }
  static void set(JsonObject const &parent, std::wstring const &name, IJsonValue const &value) {
    parent.SetNamedValue(hstring(name), value);
  }
  static void set_object(JsonObject const &parent, std::wstring const &name, JsonObject const &child) {
    parent.SetNamedValue(hstring(name), child);
  }
  static void remove(JsonObject const &parent, std::wstring const &name) {
    if (const hstring key(name); parent.HasKey(key))
      parent.Remove(key);
  }
};

std::optional<JsonObject> present(JsonObject const &object) {
  return object ? std::optional<JsonObject>(object) : std::nullopt;
}

// The stored value at a path (see PreferencePath.h), or null when it is absent.
IJsonValue lookup(JsonObject const &root, std::vector<std::wstring> const &path) {
  return nav::find_preference<JsonPreferenceTree>(present(root), path);
}

uint64_t strict_revision(JsonObject const &object, std::wstring_view key,
                         uint64_t fallback) {
  if (!object.HasKey(hstring(key))) return fallback;
  const auto value = object.Lookup(hstring(key));
  if (value.ValueType() != JsonValueType::Number) throw std::runtime_error("invalid revision");
  const double raw = value.GetNumber();
  // WinRT exposes JSON numbers as double; keep only exact non-negative integers.
  constexpr double kMaxExactJsonInteger = 9007199254740991.0; // 2^53 - 1
  if (!std::isfinite(raw) || raw < 0 || std::trunc(raw) != raw || raw > kMaxExactJsonInteger)
    throw std::runtime_error("invalid revision");
  return static_cast<uint64_t>(raw);
}

// 保存后立即刷新持久授权；窗口存活期间另有独立观察器跟随其他进程的修改。

class PreferencesDocument {
public:
  PreferencesDocument() { reset_defaults(); }

  bool Load(std::wstring &error) {
    const auto directory = state_directory();
    if (directory.empty()) {
      error = L"找不到水杉输入法的数据目录。请先完成输入法安装。";
      return false;
    }
    const auto path = path_utf8(directory);
    auto response = take_response(msime_client_load_preferences(
        reinterpret_cast<const uint8_t *>(path.data()), path.size()));
    if (!response.ok) {
      error = L"读取设置失败，请稍后重试。";
      reset_defaults();
      return false;
    }
    try {
      document_ =
          JsonObject::Parse(text(response.text)).GetNamedObject(L"value");
      preferences_ = document_.GetNamedObject(L"preferences");
      revision_ = strict_revision(document_, L"revision", 0);
      return true;
    } catch (...) {
      error = L"设置文件格式无效。";
      return false;
    }
  }

  // 成功时返回空。失败时文档保持调用前的样子（包括没存下的修改），返回失败属于哪一类（见 PreferencesSaveFailure.h），由调用方决定重读还是保留修改稍后重试。
  std::optional<msime::settings::PreferencesSaveFailure> Save() {
    const auto directory = state_directory();
    const auto path = path_utf8(directory);
    document_.SetNamedValue(L"preferences", preferences_);
    const auto snapshot = utf8(document_.Stringify());
    auto response = take_response(msime_client_save_preferences(
        reinterpret_cast<const uint8_t *>(path.data()), path.size(), revision_,
        reinterpret_cast<const uint8_t *>(snapshot.data()), snapshot.size()));
    if (!response.ok) {
      std::string error;
      try {
        error = utf8(JsonObject::Parse(text(response.text)).GetNamedString(L"error", L""));
      } catch (...) {
      }
      return msime::settings::classify_preferences_save_failure(error);
    }
    try {
      document_ =
          JsonObject::Parse(text(response.text)).GetNamedObject(L"value");
      preferences_ = document_.GetNamedObject(L"preferences");
      revision_ = strict_revision(document_, L"revision", revision_);
      return std::nullopt;
    } catch (...) {
      // 设置已写入，但返回内容读不了：调用方重读已存的设置。
      return msime::settings::PreferencesSaveFailure::rejected;
    }
  }

  uint64_t revision() const { return revision_; }

  // The stored value, else the shipped default, else null. A stored null (an unset optional field) is returned as it is.
  // A key is a dotted path of any depth, such as `plugins.key_sound.enabled`.
  IJsonValue Value(std::wstring_view key) const {
    return value_at(nav::preference_path(key));
  }

  // The shipped default for a key, for the rows that restore defaults.
  IJsonValue Default(std::wstring_view key) const {
    return lookup(defaults_, nav::preference_path(key));
  }

  bool Boolean(std::wstring_view key, bool fallback) const {
    const auto value = Value(key);
    return value && value.ValueType() == JsonValueType::Boolean
               ? value.GetBoolean()
               : fallback;
  }

  std::wstring String(std::wstring_view key, std::wstring_view fallback) const {
    const auto value = Value(key);
    return value && value.ValueType() == JsonValueType::String
               ? std::wstring(value.GetString().c_str())
               : std::wstring(fallback);
  }

  double Number(std::wstring_view key, double fallback) const {
    const auto value = Value(key);
    return value && value.ValueType() == JsonValueType::Number
               ? value.GetNumber()
               : fallback;
  }

  std::vector<std::wstring> Strings(std::wstring_view key) const {
    std::vector<std::wstring> result;
    const auto value = Value(key);
    if (!value || value.ValueType() != JsonValueType::Array)
      return result;
    result.reserve(value.GetArray().Size());
    for (auto const &item : value.GetArray())
      if (item.ValueType() == JsonValueType::String)
        result.emplace_back(item.GetString().c_str());
    return result;
  }

  void SetBoolean(std::wstring_view key, bool value) {
    SetValue(key, JsonValue::CreateBooleanValue(value));
  }

  void SetString(std::wstring_view key, std::wstring_view value) {
    SetValue(key, JsonValue::CreateStringValue(hstring(value)));
  }

  void SetNumber(std::wstring_view key, double value) {
    SetValue(key, JsonValue::CreateNumberValue(value));
  }

  // Stores the list sorted and without repeats, for the lists whose order means nothing.
  void SetStrings(std::wstring_view key, std::vector<std::wstring> values) {
    std::sort(values.begin(), values.end());
    values.erase(std::unique(values.begin(), values.end()), values.end());
    SetOrderedStrings(key, values);
  }

  // Stores the list exactly as given, for the lists whose order is the user's.
  void SetOrderedStrings(std::wstring_view key, std::vector<std::wstring> const &values) {
    JsonArray array;
    for (auto const &value : values)
      array.Append(JsonValue::CreateStringValue(hstring(value)));
    SetValue(key, array);
  }

  // One entry of a string map such as `ai_assistant.tokens`. The entry key is taken literally, dots included, so an origin can be a key.
  std::wstring Entry(std::wstring_view map_key, std::wstring_view entry,
                     std::wstring_view fallback) const {
    const auto value = value_at(nav::preference_entry_path(map_key, entry));
    return value && value.ValueType() == JsonValueType::String
               ? std::wstring(value.GetString().c_str())
               : std::wstring(fallback);
  }

  void SetEntry(std::wstring_view map_key, std::wstring_view entry, std::wstring_view value) {
    set_at(nav::preference_entry_path(map_key, entry), JsonValue::CreateStringValue(hstring(value)));
  }

  void RemoveEntry(std::wstring_view map_key, std::wstring_view entry) {
    if (preferences_)
      nav::remove_preference<JsonPreferenceTree>(preferences_,
                                                 nav::preference_entry_path(map_key, entry));
  }

  // Replaces a value with a copy of its shipped default. A key without a default is left as it is.
  void Restore(std::wstring_view key) {
    const auto value = Default(key);
    if (value)
      SetValue(key, JsonValue::Parse(value.Stringify()));
  }

  // Creates the objects missing on the way, each from its shipped default (see PreferencePath.h).
  void SetValue(std::wstring_view key, IJsonValue const &value) {
    set_at(nav::preference_path(key), value);
  }

  // Drops a stored key at any depth, so reads fall back to the shipped default. Nothing is created on the way.
  void Remove(std::wstring_view key) {
    if (preferences_)
      nav::remove_preference<JsonPreferenceTree>(preferences_, nav::preference_path(key));
  }

private:
  void reset_defaults() {
    auto defaults = take_response(msime_client_default_preferences());
    if (!defaults.ok) {
      document_ = JsonObject();
      preferences_ = JsonObject();
      defaults_ = JsonObject();
      revision_ = 0;
      return;
    }
    try {
      defaults_ =
          JsonObject::Parse(text(defaults.text)).GetNamedObject(L"value");
      preferences_ = JsonObject::Parse(defaults_.Stringify());
      document_ = JsonObject();
      document_.SetNamedValue(L"format_version",
                              JsonValue::CreateNumberValue(1));
      document_.SetNamedValue(L"revision", JsonValue::CreateNumberValue(0));
      document_.SetNamedValue(L"preferences", preferences_);
      revision_ = 0;
    } catch (...) {
      document_ = JsonObject();
      preferences_ = JsonObject();
      defaults_ = JsonObject();
      revision_ = 0;
    }
  }

  // Value() and Entry() on a path already split.
  IJsonValue value_at(std::vector<std::wstring> const &path) const {
    if (auto value = lookup(preferences_, path))
      return value;
    return lookup(defaults_, path);
  }

  void set_at(std::vector<std::wstring> const &path, IJsonValue const &value) {
    if (preferences_)
      nav::set_preference<JsonPreferenceTree>(preferences_, present(defaults_), path, value);
  }

  JsonObject document_{nullptr};
  JsonObject preferences_{nullptr};
  JsonObject defaults_{nullptr};
  uint64_t revision_ = 0;
};

// ---- Palette ----

Color rgba(int red, int green, int blue, double alpha) {
  return Color{static_cast<uint8_t>(std::lround(alpha * 255)),
               static_cast<uint8_t>(red), static_cast<uint8_t>(green),
               static_cast<uint8_t>(blue)};
}

Color transparent_color() { return Color{0, 0, 0, 0}; }

// 设置与输入浮层采用 macOS 的品牌绿及分层表面；Windows 控件仍由 WinUI 提供。`window_bg` 是侧栏（窗口底色），`content_bg` 是正文和它上方的标题栏那一列，`edge` 是侧栏右侧的边线，`title_edge` 是标题栏内容列底部的细线，`hover` 是侧栏导航项悬停、按下和选中共用的中性底色，数值取自共享设置 UI 的 macOS 外壳（`settings-style.ts` 的 `sidebar`、`content`、`macosToolbar` 和 `platform-tokens.ts` 的 `hover`）。`accent` 只用于开关、复选框、滑块这类不带文字的填充；底上是白字的填充（主按钮、提示条按钮、头像、社区资源图块）用 `accent_strong`，深浅两种外观都是浅色模式的品牌绿，与共享页的 `--accent-strong` 一样保证白字可读。
struct Palette {
  bool dark = false;
  Color text, sub, faint, subtle, subtle2, card, stroke, stroke2, divider,
      accent, accent_strong, on_accent, seg_well, preview_bg, window_bg, content_bg, edge,
      title_edge, hover;
};

Palette make_palette(bool dark) {
  Palette p;
  p.dark = dark;
  p.accent = dark ? rgba(0x5F, 0xBF, 0x84, 1) : rgba(0x2C, 0x7A, 0x4B, 1);
  p.accent_strong = rgba(0x2C, 0x7A, 0x4B, 1);
  p.on_accent = rgba(255, 255, 255, 1);
  if (dark) {
    p.text = rgba(0xF5, 0xF5, 0xF7, 1);
    p.sub = rgba(0x98, 0x98, 0x9D, 1);
    p.faint = rgba(255, 255, 255, .6);
    p.subtle = rgba(255, 255, 255, .0605);
    p.subtle2 = rgba(255, 255, 255, .0837);
    // macOS 的深色分组底是 #1F1F21 正文上叠 4.5% 白（rgba(255,255,255,.045)），这里取合成后的不透明色。
    p.card = rgba(0x29, 0x29, 0x2B, 1);
    p.stroke = rgba(255, 255, 255, .07);
    p.stroke2 = rgba(255, 255, 255, .093);
    p.divider = rgba(255, 255, 255, .0837);
    p.seg_well = rgba(0, 0, 0, .12);
    p.preview_bg = rgba(0x2C, 0x2C, 0x2C, 1);
    p.window_bg = rgba(0x2E, 0x2E, 0x30, 1);
    p.content_bg = rgba(0x1F, 0x1F, 0x21, 1);
    p.edge = rgba(0, 0, 0, .5);
    p.title_edge = rgba(255, 255, 255, .08);
    p.hover = rgba(255, 255, 255, .1);
  } else {
    p.text = rgba(0x1D, 0x1D, 0x1F, 1);
    p.sub = rgba(0x6E, 0x6E, 0x73, 1);
    p.faint = rgba(0, 0, 0, .45);
    p.subtle = rgba(0, 0, 0, .0373);
    p.subtle2 = rgba(0, 0, 0, .0578);
    p.card = rgba(255, 255, 255, 1);
    p.stroke = rgba(0, 0, 0, .0578);
    p.stroke2 = rgba(0, 0, 0, .0803);
    p.divider = rgba(0, 0, 0, .0803);
    p.seg_well = rgba(0, 0, 0, .04);
    p.preview_bg = rgba(255, 255, 255, 1);
    p.window_bg = rgba(0xE8, 0xE8, 0xEA, 1);
    p.content_bg = rgba(0xF2, 0xF2, 0xF4, 1);
    p.edge = rgba(0, 0, 0, .08);
    p.title_edge = rgba(0, 0, 0, .07);
    p.hover = rgba(0, 0, 0, .06);
  }
  return p;
}

// A colour of the candidate palette (straight alpha in [0,1]) as a XAML colour.
Color to_color(msime::windows::CandidateColor const &color) {
  auto channel = [](float value) {
    return static_cast<uint8_t>(
        std::lround(std::clamp(value, 0.0f, 1.0f) * 255.0f));
  };
  return Color{channel(color.a), channel(color.r), channel(color.g),
               channel(color.b)};
}

SolidColorBrush brush(Color const &color) { return SolidColorBrush(color); }

// "#RRGGBB" from the theme catalog.

// ---- Page labels ----

// 页面名称与图标字形，按稳定的路由 id 取值。主题皮肤、账号同步、社区、插件、剪贴板、统计、AI、词库、语音、屏幕键盘和诊断页已移除。
struct PageLabel {
  std::string_view id;
  const wchar_t *title;
  wchar_t glyph;
};

constexpr std::array<PageLabel, 5> page_labels{{
    {"typing", L"输入", 0xE765},
    {"shortcuts", L"快捷键", 0xEDA7},
    {"candidate", L"候选窗口", 0xE8FD},
    {"toolbar", L"悬浮工具栏", 0xE7F4},
    {"about", L"关于", 0xE946},
}};
static_assert(page_labels.size() == nav::pages.size());
static_assert(page_labels[0].id == nav::default_page);

// 未知 id 取默认页面的标签。
const PageLabel &page_label(std::string_view id) {
  for (const auto &label : page_labels)
    if (label.id == id)
      return label;
  return page_labels[0];
}

std::vector<std::wstring_view> page_search_names(std::string_view id) {
  return {page_label(id).title};
}

// 侧栏搜索没有结果时结果列表里的那句话，与 macOS 相同。
constexpr std::wstring_view no_search_results = L"没有匹配的设置";

constexpr const wchar_t *download_url = L"https://msime.app/download/";
constexpr const wchar_t *privacy_url = L"https://msime.app/privacy/";
// 共享设置「帮助」页「完整文档」打开的地址（`packages/ui/src/settings/app-resources.ts` 的 `documentationUrl`）。
constexpr const wchar_t *documentation_url = L"https://msime.app/docs/";
constexpr const wchar_t *license_url =
    L"https://github.com/metasequoiaime/msime/blob/develop/LICENSE";
constexpr const wchar_t *releases_url =
    L"https://github.com/metasequoiaime/msime/releases";

// 本次构建运行时的版本号：发布构建取 `Build-Client.ps1` 打上的值（`TargetVersion`），开发构建取 `platforms/windows/version.txt`，与 Server 报告的值相同。
constexpr std::wstring_view app_version = L"" MSIME_WINDOWS_VERSION;

// 本次构建的发布日期 `YYYY-MM-DD`，由 `MSIME.Settings.vcxproj` 依次取 `MSIME_RELEASE_DATE`、`platforms/windows/release-date.txt` 和 git 当前提交的日期，都没有时为空。只加在「关于」页的版本文字上，检查更新和反馈报告仍用 `app_version`。
constexpr std::wstring_view app_release_date = L"" MSIME_WINDOWS_RELEASE_DATE;

// 「关于」页的版本文字：`v0.2.1（2026-10-19 发布）`，没有发布日期时只是 `v0.2.1`。
std::wstring app_version_label() {
  std::wstring label = L"v" + std::wstring(app_version);
  if (!app_release_date.empty())
    label += L"（" + std::wstring(app_release_date) + L" 发布）";
  return label;
}


// 系统版本「主.次.内部版本号」，写进反馈报告的诊断信息。取自 ntdll 的 `RtlGetVersion`：`GetVersionEx` 按应用清单声明的兼容系统作答，可能报低。Windows 11 的主版本号仍是 10，内部版本号从 22000 起。读不到时为空，报告改写「平台：windows」。
std::wstring windows_os_version() {
  using RtlGetVersion = LONG(WINAPI *)(PRTL_OSVERSIONINFOW);
  const auto ntdll = GetModuleHandleW(L"ntdll.dll");
  const auto get = ntdll ? reinterpret_cast<RtlGetVersion>(reinterpret_cast<void *>(
                               GetProcAddress(ntdll, "RtlGetVersion")))
                         : nullptr;
  RTL_OSVERSIONINFOW info{};
  info.dwOSVersionInfoSize = sizeof(info);
  if (!get || get(&info) != 0)
    return {};
  return std::to_wstring(info.dwMajorVersion) + L"." + std::to_wstring(info.dwMinorVersion) +
         L"." + std::to_wstring(info.dwBuildNumber);
}


std::wstring lowercase(std::wstring_view value) {
  std::wstring result(value);
  for (auto &c : result)
    c = static_cast<wchar_t>(std::towlower(c));
  return result;
}

std::wstring number_text(double value) {
  return std::to_wstring(static_cast<long long>(std::llround(value)));
}

hstring glyph_text(wchar_t glyph) { return hstring(std::wstring(1, glyph)); }

// The largest PNG frame of the product icon (RCDATA 102 is msime.ico), for the title bar and the about card. Empty when the resource is missing or malformed: those places then show no picture.
std::vector<uint8_t> const &logo_png() {
  static const std::vector<uint8_t> bytes = [] {
    std::vector<uint8_t> result;
    const HMODULE instance = GetModuleHandleW(nullptr);
    const HRSRC resource = FindResourceW(instance, MAKEINTRESOURCEW(102),
                                         MAKEINTRESOURCEW(10) /* RT_RCDATA */);
    if (!resource)
      return result;
    const HGLOBAL handle = LoadResource(instance, resource);
    const DWORD size = SizeofResource(instance, resource);
    const auto *data =
        handle ? static_cast<const uint8_t *>(LockResource(handle)) : nullptr;
    if (!data || size < 6)
      return result;
    auto u16 = [data](size_t at) {
      return static_cast<uint32_t>(data[at] | (data[at + 1] << 8));
    };
    auto u32 = [data](size_t at) {
      return static_cast<uint32_t>(data[at]) |
             (static_cast<uint32_t>(data[at + 1]) << 8) |
             (static_cast<uint32_t>(data[at + 2]) << 16) |
             (static_cast<uint32_t>(data[at + 3]) << 24);
    };
    if (u16(0) != 0 || u16(2) != 1)
      return result;
    const uint32_t count = u16(4);
    uint32_t best_width = 0;
    uint32_t best_offset = 0;
    uint32_t best_size = 0;
    for (uint32_t i = 0; i < count; ++i) {
      const size_t entry = 6 + static_cast<size_t>(i) * 16;
      if (entry + 16 > size)
        break;
      const uint32_t width =
          data[entry] == 0 ? 256u : static_cast<uint32_t>(data[entry]);
      const uint32_t length = u32(entry + 8);
      const uint32_t offset = u32(entry + 12);
      if (offset > size || length > size - offset || length < 8)
        continue;
      static constexpr uint8_t png[] = {0x89, 'P', 'N', 'G'};
      if (!std::equal(std::begin(png), std::end(png), data + offset))
        continue;
      if (width > best_width) {
        best_width = width;
        best_offset = offset;
        best_size = length;
      }
    }
    if (best_width != 0)
      result.assign(data + best_offset, data + best_offset + best_size);
    return result;
  }();
  return bytes;
}

// Decodes PNG bytes into `bitmap`. The bytes must outlive the decode: callers pass the static logo copy or a resource, which stays mapped as long as the process runs. Must start on the UI thread; awaiting a WinRT operation resumes on this apartment.
fire_and_forget load_png(Imaging::BitmapImage bitmap, uint8_t const *data, size_t size) {
  if (size == 0)
    co_return;
  try {
    Windows::Storage::Streams::InMemoryRandomAccessStream stream;
    Windows::Storage::Streams::DataWriter writer(stream);
    writer.WriteBytes(array_view<uint8_t const>(data, data + size));
    co_await writer.StoreAsync();
    writer.DetachStream();
    stream.Seek(0);
    co_await bitmap.SetSourceAsync(stream);
  } catch (hresult_error const &) {
    // A picture that cannot be decoded leaves the space empty; nothing else depends on it.
  }
}

// Shows PNG bytes in `image`; the picture appears once `load_png` has decoded it. `pixels` is the decode width, 0 for the picture's own.
void show_png(Image image, uint8_t const *data, size_t size, int32_t pixels) {
  if (size == 0)
    return;
  Imaging::BitmapImage bitmap;
  if (pixels > 0)
    bitmap.DecodePixelWidth(pixels);
  image.Source(bitmap);
  load_png(bitmap, data, size);
}

void show_logo(Image image, int32_t pixels) {
  const auto &bytes = logo_png();
  show_png(image, bytes.data(), bytes.size(), pixels);
}

// 「关于」页反馈渠道的品牌图标在 Settings.rc 里的资源号（72 × 72 的 PNG，许可见 assets/feedback/NOTICE.md）。GitHub 的图标是单色的，共享页把它遮罩成正文色，这里按明暗各打一份。
constexpr int github_light_icon = 120;
constexpr int github_dark_icon = 121;
constexpr int qq_icon = 122;
constexpr int telegram_icon = 123;

// 把 Settings.rc 里资源号为 `resource_id` 的 PNG（RCDATA）显示到 `image` 上，`pixels` 同 show_png；资源缺失时什么都不画。
void show_resource_png(Image image, int resource_id, int32_t pixels) {
  const HMODULE instance = GetModuleHandleW(nullptr);
  const HRSRC resource = FindResourceW(instance, MAKEINTRESOURCEW(resource_id),
                                       MAKEINTRESOURCEW(10) /* RT_RCDATA */);
  if (!resource)
    return;
  const HGLOBAL handle = LoadResource(instance, resource);
  const auto *data =
      handle ? static_cast<const uint8_t *>(LockResource(handle)) : nullptr;
  if (!data)
    return;
  show_png(image, data, SizeofResource(instance, resource), pixels);
}

// 页首的水杉娘插画：Settings.rc 里那张 PNG（RCDATA）解码成的位图。位图立刻返回，解码完自己显示出来；调用方按场景缓存它（`mascot_bitmaps_`），再画页首时直接复用，插画不会因为重新解码而闪一下。资源缺失时返回空，页首只剩说明文字。
Imaging::BitmapImage load_mascot(nav::MascotScene scene) {
  const HMODULE instance = GetModuleHandleW(nullptr);
  const HRSRC resource =
      FindResourceW(instance, MAKEINTRESOURCEW(nav::mascot_resource(scene)),
                    MAKEINTRESOURCEW(10) /* RT_RCDATA */);
  if (!resource)
    return nullptr;
  const HGLOBAL handle = LoadResource(instance, resource);
  const auto *data =
      handle ? static_cast<const uint8_t *>(LockResource(handle)) : nullptr;
  if (!data)
    return nullptr;
  Imaging::BitmapImage bitmap;
  load_png(bitmap, data, SizeofResource(instance, resource));
  return bitmap;
}

struct Option {
  std::wstring value;
  std::wstring label;
  // 不可选的选项照常显示，但选不了：分段控件的按钮按不下去（「候选字体」在 Windows 上的「圆体」），下拉框里那一项是灰的（「麦克风」里已忽略的其他平台录音设置和未连接的已选设备）。
  bool enabled = true;
};

// ---- 系统字体列表 ----

// 共享页用 localeCompare 给字体列表排序；这里按用户的区域设置比较。
bool font_name_before(std::wstring const &a, std::wstring const &b) {
  return CompareStringEx(LOCALE_NAME_USER_DEFAULT, 0, a.c_str(), static_cast<int>(a.size()),
                         b.c_str(), static_cast<int>(b.size()), nullptr, nullptr,
                         0) == CSTR_LESS_THAN;
}

// 共享页用 toLocaleLowerCase 筛选字体；这里按用户的区域设置转小写，ASCII 以外的字母也能不分大小写地匹配。转换失败时保留原样。
std::wstring font_name_lower(std::wstring_view value) {
  if (value.empty())
    return {};
  constexpr DWORD flags = LCMAP_LOWERCASE | LCMAP_LINGUISTIC_CASING;
  const int length = static_cast<int>(value.size());
  const int needed = LCMapStringEx(LOCALE_NAME_USER_DEFAULT, flags, value.data(), length,
                                   nullptr, 0, nullptr, nullptr, 0);
  if (needed <= 0)
    return std::wstring(value);
  std::wstring result(static_cast<std::size_t>(needed), L'\0');
  if (LCMapStringEx(LOCALE_NAME_USER_DEFAULT, flags, value.data(), length, result.data(),
                    needed, nullptr, nullptr, 0) != needed)
    return std::wstring(value);
  return result;
}

// 已安装的字体名，与 Tauri 的 list_font_families 读的是同一份目录（host-api 的 system_fonts）。读取失败、返回内容不是字符串数组或含不合法的名字时为空，页面显示读取失败。枚举系统字体，只在 UI 线程之外调用。
std::optional<std::vector<std::wstring>> read_font_catalog() {
  const auto response = take_response(msime_client_list_font_families());
  if (!response.ok)
    return std::nullopt;
  try {
    const auto value = JsonObject::Parse(text(response.text)).GetNamedArray(L"value");
    if (value.Size() > nav::font_catalog_limit)
      return std::nullopt;
    std::vector<std::wstring> fonts;
    fonts.reserve(value.Size());
    for (auto const &item : value) {
      if (item.ValueType() != JsonValueType::String)
        return std::nullopt;
      fonts.emplace_back(item.GetString().c_str());
    }
    return nav::normalize_font_catalog(std::move(fonts), font_name_before);
  } catch (...) {
    return std::nullopt;
  }
}

// ---- 辅助码方案的来源 ----

// 自定义辅助码表，与 Tauri 的 list_helpcode_schemas 相同：输入法宿主选项（runtime-options.json）里状态根 preferences_directory（即 DataDir）和资源目录 resources 下的 helpcodes/custom，同一主干取状态根那份。还没有宿主选项时是失败的响应，页面当作没有自定义码表。读目录，只在 UI 线程之外调用。
Response helpcode_schemas_call() {
  const auto options = runtime_options_document();
  std::wstring resources;
  std::wstring state_root;
  if (options) {
    try {
      const auto document = JsonObject::Parse(text(*options));
      resources = document.GetNamedString(L"resources", L"").c_str();
      state_root = document.GetNamedString(L"preferences_directory", L"").c_str();
    } catch (...) {
      resources.clear();
      state_root.clear();
    }
  }
  // 与 Tauri 一样只接受绝对路径。
  if (resources.empty() || !std::filesystem::path(resources).is_absolute())
    return {"{\"ok\":false,\"error\":\"no host options\"}", false};
  if (!state_root.empty() && !std::filesystem::path(state_root).is_absolute())
    state_root.clear();
  const auto path = utf8(hstring(resources));
  const auto state = utf8(hstring(state_root));
  return take_response(msime_client_helpcode_schemas(
      reinterpret_cast<const uint8_t *>(path.data()), path.size(),
      state.empty() ? nullptr : reinterpret_cast<const uint8_t *>(state.data()), state.size()));
}

// 自定义码表在下拉框里的名字，与共享页的 customSchemaLabel 相同：中英文名都有且不同时是「中文名 (英文名)」，否则依次退到中文名、英文名、文件名和方案 id。
std::wstring custom_helpcode_label(JsonObject const &schema) {
  const auto name = named_string(schema, L"name");
  const auto name_en = named_string(schema, L"name_en");
  if (!name.empty() && !name_en.empty() && name != name_en)
    return name + L" (" + name_en + L")";
  for (auto const &label : {name, name_en, named_string(schema, L"file_stem")})
    if (!label.empty())
      return label;
  return named_string(schema, L"schema");
}

// msime_client_helpcode_schemas 的回答变成下拉框选项（值是 custom/<文件名>）；失败或读不懂时没有自定义码表。
std::vector<Option> custom_helpcode_options(Response const &response) {
  std::vector<Option> options;
  if (!response.ok)
    return options;
  try {
    for (auto const &item : JsonObject::Parse(text(response.text)).GetNamedArray(L"value")) {
      const auto schema = item.GetObject();
      const auto value = named_string(schema, L"schema");
      if (value.starts_with(L"custom/"))
        options.push_back({value, custom_helpcode_label(schema)});
    }
  } catch (...) {
    options.clear();
  }
  return options;
}

struct Check {
  std::wstring label;
  bool on = false;
  std::function<void(bool)> apply;
  bool enabled = true;
};

// 侧栏搜索的一条结果：整页（`whole_page`，只写页名，打开那一页）或页里的一行（打开那一页并把焦点交给这一行）。
struct SearchEntry {
  std::wstring title;
  std::string page;
  // 条目所在的页内标签（`page_tab_bar`），不在标签里时为空；选中这条结果时先切到这个标签。
  std::wstring tab;
  // 结果第二行的位置「页 › 标签 › 分组」，没有标签或分组时省掉那一段，整页结果为空。
  std::wstring path;
  bool whole_page = false;
};

// 候选窗口预览的固定样例：拼音 ni'mf 的候选和各自的辅助码。它只示意排列、每页数量和字号，不是词库。
constexpr std::array<std::pair<const wchar_t *, const wchar_t *>, 9>
    preview_words{{{L"你们", L"rR"},
                   {L"你", L"rX"},
                   {L"尼", L"uV"},
                   {L"妮", L"nV"},
                   {L"泥", L"dV"},
                   {L"逆", L"zQ"},
                   {L"拟", L"fR"},
                   {L"腻", L"oD"},
                   {L"拧", L"nG"}}};

Color faded(Color color, double opacity) {
  color.A = static_cast<uint8_t>(std::lround(std::clamp(opacity, 0.0, 1.0) * 255));
  return color;
}

Windows::Foundation::Point point_at(double x, double y) {
  return {static_cast<float>(x), static_cast<float>(y)};
}
struct MainWindow : WindowT<MainWindow> {
  explicit MainWindow(std::string page)
      : current_page_(nav::offered_page_for_route(page, false)) {
    // 已移除功能的旧路由由 SettingsNavigation.h 落回默认页。
    // 窗口标题与 macOS 设置窗口相同，写作「水杉输入法 · 设置」（任务栏和 Alt+Tab 里看得到）；标题和侧栏的产品名按版本取，full 仍是「水杉输入法」。
    Title(MSIME_EDITION_DISPLAY_NAME L" · 设置");
    ExtendsContentIntoTitleBar(true);
    reload_document();
    system_dark_ = system_dark();
    input_method_ = input_method_state();
    palette_ = make_palette(resolve_dark());
    // 初始尺寸与 macOS 设置窗口相同（`tauri.macos.conf.json` 的 1000 × 780），按系统缩放换算。
    const double scale = static_cast<double>(GetDpiForSystem()) / 96.0;
    AppWindow().Resize(
        {static_cast<int32_t>(std::lround(1000 * scale)),
         static_cast<int32_t>(std::lround(780 * scale))});
    // 最小尺寸不跟 macOS 的 360 × 540：macOS 窄于 601 时收起侧栏，这个窗口没有窄窗口布局，侧栏常驻，800 × 560 以下正文会被挤到无法操作。
    if (const auto presenter = AppWindow().Presenter().try_as<Microsoft::UI::Windowing::OverlappedPresenter>()) {
      presenter.PreferredMinimumWidth(
          box_value(static_cast<int32_t>(std::lround(800 * scale)))
              .as<Windows::Foundation::IReference<int32_t>>());
      presenter.PreferredMinimumHeight(
          box_value(static_cast<int32_t>(std::lround(560 * scale)))
              .as<Windows::Foundation::IReference<int32_t>>());
    }
    build_shell(0.0);
    Activated({this, &MainWindow::on_activated});
    Closed([this](Inspectable const &, WindowEventArgs const &) {
      // A save refused while closing has no window left to report in; the stored settings stay as they were.
      closed_ = true;
      flush_pending();
      // 写盘失败后保留着的修改在关闭前最后再存一次；仍失败就只能留在磁盘上的旧值。
      retry_unsaved_edit();
      if (save_retry_timer_) save_retry_timer_.Stop();
    });
  }

  // 第二次启动只恢复已有窗口；带页面的请求沿用首启的类别和页内标签。
  bool open_route(std::string_view route) {
    if (closed_) return false;
    if (route == "settings") return true;
    if (!route.starts_with("settings:")) return false;
    const std::string page(route.substr(9));
    navigate(std::string(nav::offered_page_for_route(page, false)));
    render_page(false);
    return true;
  }

private:
  // ---- Document and palette ----

  void reload_document() {
    PreferencesDocument fresh;
    std::wstring error;
    if (fresh.Load(error)) {
      document_ = std::move(fresh);
      discard_unsaved_edit();
      loaded_ = true;
      load_error_.clear();
    } else if (!loaded_) {
      document_ = std::move(fresh);
      discard_unsaved_edit();
      load_error_ = error;
    }
  }


  bool system_dark() {
    const auto background = ui_settings_.GetColorValue(
        Windows::UI::ViewManagement::UIColorType::Background);
    return background.R < 128;
  }

  // 设置窗口跟随颜色模式（HostCapabilities::surface_themes_follow），不读存下的 settings_theme；跟随系统时看 Windows 的应用模式。
  bool resolve_dark() {
    const auto theme = document_.String(L"theme", L"system");
    if (theme == L"dark")
      return true;
    if (theme == L"light")
      return false;
    return system_dark_;
  }

  // The candidate window's own light/dark mode, decided as the Server decides it (candidate_theme_dark in src/candidate/CandidateThemeSettings.h): the 候选窗口主题 override, else 颜色模式.
  bool candidate_dark() const {
    const auto surface = document_.String(L"candidate_theme", L"follow");
    if (surface == L"dark" || surface == L"light")
      return surface == L"dark";
    const auto theme = document_.String(L"theme", L"system");
    return theme == L"system" ? system_dark_ : theme != L"light";
  }

  // 主题皮肤已移除：预览一律使用原生配色，只按当前明暗取值。
  msime::windows::CandidatePalette candidate_theme(
      std::wstring const &, bool = true, std::optional<bool> appearance = std::nullopt) {
    return msime::windows::candidate_native_palette(
        appearance.value_or(candidate_dark()));
  }

  // Re-reads the palette and redraws: the whole window when the palette changed, otherwise the current page.
  void refresh() {
    const bool dark = resolve_dark();
    if (dark != palette_.dark) {
      palette_ = make_palette(dark);
      rebuild_shell();
      return;
    }
    render_page(true);
  }

  // 切换明暗时保留滚动位置和侧栏搜索内容；写回文字时结果列表随之重建。
  void rebuild_shell() {
    const double offset = scroll_ ? scroll_.VerticalOffset() : 0.0;
    const hstring query = search_ ? search_.Text() : hstring();
    build_shell(offset);
    search_.Text(query);
  }

  void on_activated(Inspectable const &, WindowActivatedEventArgs const &args) {
    if (args.WindowActivationState() == WindowActivationState::Deactivated) {
      flush_pending();
      retry_unsaved_edit();
      return;
    }
    // Another window (the tray or the shared app) may have changed the settings, or Windows may have switched between light and dark while this window was in the background.
    bool changed = false;
    PreferencesDocument fresh;
    std::wstring error;
    if (fresh.Load(error) &&
        (!loaded_ || fresh.revision() != document_.revision())) {
      // 写盘失败后保留着的修改还没存下，别处却已经存了新的修订号：与保存时遇到的冲突一样，以别处的设置为准并说明。
      if (pending_save_failures_ > 0) {
        discard_unsaved_edit();
        notice_ = msime::settings::preferences_reload_failure_message(
            msime::settings::PreferencesSaveFailure::conflict);
        notice_severity_ = InfoBarSeverity::Error;
      }
      document_ = std::move(fresh);
      loaded_ = true;
      load_error_.clear();
      changed = true;
    }
    if (const bool dark = system_dark(); dark != system_dark_) {
      system_dark_ = dark;
      changed = true;
    }
    // The keyboard list is edited in Windows Settings, which is where the banner's fallback sends the user.
    if (const auto state = input_method_state(); state != input_method_) {
      input_method_ = state;
      changed = true;
    }
    if (!changed || !root_)
      return;
    // A click that activates the window is still on its way to a control: the press and release arrive after this event, and a redraw between them would replace the control under the pointer and drop the click. That redraw waits for the release. Every other activation redraws once this event has returned.
    if (args.WindowActivationState() == WindowActivationState::PointerActivated) {
      defer_refresh_until_pointer_up();
      return;
    }
    queue_refresh();
  }

  void defer_refresh_until_pointer_up() {
    refresh_after_pointer_ = true;
    // A click on the title bar's drag region or caption buttons reaches no XAML element, so no release would ever arrive: the timer redraws once it is clear that no press followed in the window's content.
    if (!pointer_timer_) {
      pointer_timer_ = DispatcherQueue().CreateTimer();
      pointer_timer_.Interval(std::chrono::milliseconds(400));
      pointer_timer_.IsRepeating(false);
      pointer_timer_.Tick([this](Microsoft::UI::Dispatching::DispatcherQueueTimer const &,
                                 Inspectable const &) {
        if (!pointer_down_)
          release_deferred_refresh();
      });
    }
    pointer_timer_.Stop();
    pointer_timer_.Start();
  }

  void release_deferred_refresh() {
    if (!refresh_after_pointer_)
      return;
    refresh_after_pointer_ = false;
    if (pointer_timer_)
      pointer_timer_.Stop();
    queue_refresh();
  }

  // Tracks the pointer over the whole window, including presses and releases a control has already handled (a button handles its own), so a deferred redraw runs only after the control has seen its click.
  void track_pointer(UIElement const &root) {
    using Microsoft::UI::Xaml::Input::PointerEventHandler;
    using Microsoft::UI::Xaml::Input::PointerRoutedEventArgs;
    root.AddHandler(UIElement::PointerPressedEvent(),
                    box_value(PointerEventHandler(
                        [this](Inspectable const &, PointerRoutedEventArgs const &) {
                          pointer_down_ = true;
                        })),
                    true);
    const auto released = box_value(PointerEventHandler(
        [this](Inspectable const &, PointerRoutedEventArgs const &) {
          pointer_down_ = false;
          release_deferred_refresh();
        }));
    root.AddHandler(UIElement::PointerReleasedEvent(), released, true);
    root.AddHandler(UIElement::PointerCanceledEvent(), released, true);
    root.AddHandler(UIElement::PointerCaptureLostEvent(), released, true);
  }

  // ---- Saving ----

  // 每个控件改动后立即保存，返回是否已经存下。被拒绝的保存（别处先存了新修订号，或这组设置不被接受）重读已存的设置并在页首说明。写盘失败（设置文件被别的程序占用等）不重读：修改留在文档里、页面照它显示，按 PreferencesSaveFailure.h 的间隔自动重试，见 keep_unsaved_edit。
  bool commit() {
    const auto failure = document_.Save();
    if (!failure) {
      settle_unsaved_edit();
      return true;
    }
    if (*failure == msime::settings::PreferencesSaveFailure::storage) {
      keep_unsaved_edit();
      return false;
    }
    discard_unsaved_edit();
    notice_ = msime::settings::preferences_reload_failure_message(*failure);
    notice_severity_ = InfoBarSeverity::Error;
    // The refused edit is still in memory; the stored settings replace it, and if they cannot be read the controls are disabled rather than showing a value that was never saved.
    PreferencesDocument fresh;
    std::wstring reload_error;
    const bool reloaded = fresh.Load(reload_error);
    document_ = std::move(fresh);
    loaded_ = reloaded;
    load_error_ = reloaded ? std::wstring() : reload_error;
    queue_refresh();
    return false;
  }

  // 写盘失败后保留修改：文档不动，说明还会自动重试，并排好下一次。自动重试用完后不再排，pending_save_failures_ 仍大于零，切到别的窗口、关闭窗口或下一次修改时再存。
  void keep_unsaved_edit() {
    ++pending_save_failures_;
    if (closed_)
      return;
    const auto delay = msime::settings::preferences_save_retry_delay(pending_save_failures_);
    auto message = msime::settings::preferences_storage_failure_message(delay.has_value());
    // 同一句说明已经显示着时不重画整页，免得每次重试都打断正在操作的控件。
    if (message != notice_)
      show_notice(std::move(message), delay ? InfoBarSeverity::Warning : InfoBarSeverity::Error);
    if (!delay)
      return;
    if (!save_retry_timer_) {
      save_retry_timer_ = DispatcherQueue().CreateTimer();
      save_retry_timer_.IsRepeating(false);
      save_retry_timer_.Tick([this](Microsoft::UI::Dispatching::DispatcherQueueTimer const &,
                                    Inspectable const &) { retry_unsaved_edit(); });
    }
    save_retry_timer_.Stop();
    save_retry_timer_.Interval(*delay);
    save_retry_timer_.Start();
  }

  // 有写盘失败后保留着的修改时再存一次。
  void retry_unsaved_edit() {
    if (pending_save_failures_ > 0)
      commit();
  }

  // 保存成功：之前保留着的修改也一并存下了，换掉写盘失败的说明。
  void settle_unsaved_edit() {
    if (pending_save_failures_ == 0)
      return;
    discard_unsaved_edit();
    show_notice(std::wstring(msime::settings::preferences_saved_after_retry_message),
                InfoBarSeverity::Success);
  }

  // 文档被已存的设置换掉（重读、别处改过）时，保留着的修改随之作废，不再重试。
  void discard_unsaved_edit() {
    pending_save_failures_ = 0;
    if (save_retry_timer_)
      save_retry_timer_.Stop();
  }

  void change(std::function<void(PreferencesDocument &)> const &edit,
              bool redraw) {
    flush_pending();
    edit(document_);
    // 写盘失败时修改仍留在文档里，预览照样跟着它走。
    if (commit() || pending_save_failures_ > 0) {
      update_preview();
      update_toolbar_preview();
      if (redraw)
        queue_refresh();
    }
  }

  // Redraws after the current event handler returns, so a control is never torn down inside its own event.
  void queue_refresh() {
    if (refresh_queued_ || closed_)
      return;
    refresh_queued_ = true;
    DispatcherQueue().TryEnqueue([weak = get_weak()] {
      if (auto self = weak.get()) {
        self->refresh_queued_ = false;
        self->refresh();
      }
    });
  }

  // Sliders write the document as they move and save once they rest.
  void schedule_save() {
    pending_save_ = true;
    if (!save_timer_) {
      save_timer_ = DispatcherQueue().CreateTimer();
      save_timer_.Interval(std::chrono::milliseconds(350));
      save_timer_.IsRepeating(false);
      save_timer_.Tick([this](Microsoft::UI::Dispatching::DispatcherQueueTimer const &,
                              Inspectable const &) { flush_pending(); });
    }
    save_timer_.Stop();
    save_timer_.Start();
  }

  void flush_pending() {
    if (save_timer_)
      save_timer_.Stop();
    if (!pending_save_)
      return;
    pending_save_ = false;
    commit();
  }

  // 直接覆盖控件模板查询的资源；全局强调色不会重算 WinUI 已解析的静态别名。
  void apply_accent_resources(ResourceDictionary const &resources) {
    if (Windows::UI::ViewManagement::AccessibilitySettings().HighContrast())
      return;
    for (const auto *key : {L"SystemAccentColor", L"SystemAccentColorLight1",
                            L"SystemAccentColorLight2", L"SystemAccentColorLight3",
                            L"SystemAccentColorDark1", L"SystemAccentColorDark2",
                            L"SystemAccentColorDark3"})
      resources.Insert(box_value(hstring(key)), box_value(palette_.accent));
    for (const auto *key : {L"SystemControlHighlightAccentBrush", L"AccentFillColorDefaultBrush",
                            L"AccentTextFillColorPrimaryBrush", L"ToggleSwitchFillOn",
                            L"ToggleSwitchFillOnPointerOver", L"ToggleSwitchFillOnPressed"})
      resources.Insert(box_value(hstring(key)), brush(palette_.accent));
    for (const auto *key : {L"ToggleSwitchKnobFillOn", L"ToggleSwitchKnobFillOnPointerOver",
                            L"ToggleSwitchKnobFillOnPressed"})
      resources.Insert(box_value(hstring(key)), brush(palette_.on_accent));

    for (const auto &[suffix, opacity] :
         std::array<std::pair<const wchar_t *, double>, 3>{
             {{L"", 1.0}, {L"PointerOver", 0.9}, {L"Pressed", 0.8}}}) {
      for (const auto *key : {
               L"CheckBoxCheckBackgroundFillChecked", L"CheckBoxCheckBackgroundStrokeChecked",
               L"CheckBoxCheckBackgroundFillIndeterminate", L"CheckBoxCheckBackgroundStrokeIndeterminate",
               L"RadioButtonOuterEllipseCheckedFill", L"RadioButtonOuterEllipseCheckedStroke",
               L"SliderTrackValueFill", L"SliderThumbBackground", L"HyperlinkButtonForeground"})
        resources.Insert(box_value(hstring(std::wstring(key) + suffix)),
                         brush(faded(palette_.accent, opacity)));
      for (const auto *key : {L"CheckBoxCheckGlyphForegroundChecked",
                              L"CheckBoxCheckGlyphForegroundIndeterminate",
                              L"RadioButtonCheckGlyphFill"})
        resources.Insert(box_value(hstring(std::wstring(key) + suffix)),
                         brush(palette_.on_accent));
    }
  }

  // ---- Window chrome ----

  void build_shell(double offset) {
    Grid root;
    root.RequestedTheme(palette_.dark ? ElementTheme::Dark : ElementTheme::Light);
    root.Background(brush(palette_.window_bg));
    apply_accent_resources(root.Resources());

    RowDefinition title_row;
    title_row.Height(GridLength{48, GridUnitType::Pixel});
    RowDefinition body_row;
    body_row.Height(GridLength{1, GridUnitType::Star});
    root.RowDefinitions().Append(title_row);
    root.RowDefinitions().Append(body_row);

    titlebar_ = make_titlebar();
    Grid::SetRow(titlebar_, 0);
    root.Children().Append(titlebar_);

    navigation_ = make_navigation();
    Grid::SetRow(navigation_, 1);
    root.Children().Append(navigation_);

    track_pointer(root);
    root.SizeChanged([this](Inspectable const &, SizeChangedEventArgs const &) {
      update_intro_size();
    });
    root_ = root;
    Content(root);
    SetTitleBar(titlebar_);
    style_caption_buttons();
    select_navigation_item();
    render_page_at(offset);
  }

  void style_caption_buttons() {
    auto bar = AppWindow().TitleBar();
    bar.PreferredHeightOption(Microsoft::UI::Windowing::TitleBarHeightOption::Tall);
    bar.ButtonBackgroundColor(transparent_color());
    bar.ButtonInactiveBackgroundColor(transparent_color());
    bar.ButtonForegroundColor(palette_.text);
    bar.ButtonInactiveForegroundColor(palette_.faint);
    bar.ButtonHoverBackgroundColor(palette_.subtle);
    bar.ButtonHoverForegroundColor(palette_.text);
    bar.ButtonPressedBackgroundColor(palette_.subtle2);
    bar.ButtonPressedForegroundColor(palette_.text);
  }

  // 侧栏宽度，与 macOS 设置侧栏（`settings-style.ts` 的 `mac:w-[260px]`）相同。
  static constexpr double sidebar_width = 260;

  Grid make_titlebar() {
    Grid bar;
    bar.Height(48);
    // 侧栏列与 `make_navigation` 的 OpenPaneLength 同宽（macOS 侧栏的 260），两处要一起改，否则品牌行和侧栏对不齐。
    ColumnDefinition sidebar;
    sidebar.Width(GridLength{sidebar_width, GridUnitType::Pixel});
    ColumnDefinition content;
    content.Width(GridLength{1, GridUnitType::Star});
    ColumnDefinition caption;
    caption.Width(GridLength{140, GridUnitType::Pixel});
    bar.ColumnDefinitions().Append(sidebar);
    bar.ColumnDefinitions().Append(content);
    bar.ColumnDefinitions().Append(caption);

    // 与 macOS 的页面工具栏一样，标题栏里正文上方那一列（连同窗口按钮）用正文底色，底部一条细线；左边那条 1 DIP 边线接着下面侧栏右侧的边线。侧栏那一列留给窗口底色，和侧栏连成一片。只是着色，整条标题栏仍是 SetTitleBar 的拖动区。
    Border content_fill;
    content_fill.Background(brush(palette_.content_bg));
    content_fill.BorderBrush(brush(palette_.edge));
    content_fill.BorderThickness(Thickness{1, 0, 0, 0});
    Border content_rule;
    content_rule.BorderBrush(brush(palette_.title_edge));
    content_rule.BorderThickness(Thickness{0, 0, 0, 1});
    content_fill.Child(content_rule);
    Grid::SetColumn(content_fill, 1);
    Grid::SetColumnSpan(content_fill, 2);
    bar.Children().Append(content_fill);

    // 品牌行照 macOS 侧栏顶部：24 DIP 图标、8 DIP 间距、15 DIP 半粗体产品名，左缘取导航项的 12 DIP 外边距加 10 DIP 内边距，与下面各项的图标列对齐。图标不另裁圆角：msime.ico 的四角本来就是透明的。
    StackPanel identity;
    identity.Orientation(Orientation::Horizontal);
    identity.Spacing(8);
    identity.Margin(Thickness{22, 0, 12, 0});
    identity.VerticalAlignment(VerticalAlignment::Center);
    Image logo;
    logo.Width(24);
    logo.Height(24);
    show_logo(logo, 48);
    identity.Children().Append(logo);
    auto product = make_text(MSIME_EDITION_DISPLAY_NAME, 15, palette_.text);
    product.FontWeight(Windows::UI::Text::FontWeight{600});
    product.VerticalAlignment(VerticalAlignment::Center);
    identity.Children().Append(product);
    bar.Children().Append(identity);

    // 页名只写在这里，正文不再画大标题，与 macOS 的页面工具栏（`settings-style.ts` 的 `macosToolbar`：左右 20 DIP 内边距，15 DIP 粗体、正文色，放不下时截断）相同。macOS 没有跨页面的历史返回，这里也不放返回按钮，整条标题栏都是拖动区，不需要放行区域。
    toolbar_title_ = make_text(page_label(current_page_).title, 15, palette_.text);
    toolbar_title_.FontWeight(Windows::UI::Text::FontWeight{700});
    toolbar_title_.TextWrapping(TextWrapping::NoWrap);
    toolbar_title_.TextTrimming(TextTrimming::CharacterEllipsis);
    toolbar_title_.Margin(Thickness{20, 0, 20, 0});
    toolbar_title_.VerticalAlignment(VerticalAlignment::Center);
    Grid::SetColumn(toolbar_title_, 1);
    bar.Children().Append(toolbar_title_);
    return bar;
  }

  // 侧栏搜索框照 macOS（`settings-style.ts` 的 `sidebarSearch`、`settings-navigation-search.tsx`）：32 DIP 高、圆角 8、没有描边，底色浅色 rgba(0,0,0,.055)、深色 rgba(255,255,255,.07)，悬停不变；左边 10 DIP 处一个 15 DIP 的次要色放大镜，相隔 8 是 13 DIP 的文字，占位文字「搜索」；聚焦时是 2 DIP 的强调色边框。WinUI 的 `TextBox` 没有左侧图标，放大镜叠在输入框上、不接收指针，输入框用左内边距给它让位；底色、边框和文字色经模板资源 `TextControl*` 覆盖，模板的视觉状态才不会把它们换回系统色。读屏名仍是「搜索设置」，自动化 ID 是 `settings-search`。
  FrameworkElement make_search_field() {
    Grid field;
    field.Height(32);
    field.Margin(Thickness{12, 4, 12, 8});
    field.HorizontalAlignment(HorizontalAlignment::Stretch);
    field.VerticalAlignment(VerticalAlignment::Center);

    search_ = TextBox();
    search_.PlaceholderText(L"搜索");
    search_.FontSize(13);
    search_.Height(32);
    search_.MinHeight(32);
    search_.Padding(Thickness{33, 7, 10, 7});
    search_.CornerRadius(CornerRadius{8, 8, 8, 8});
    search_.BorderThickness(Thickness{0, 0, 0, 0});
    search_.HorizontalAlignment(HorizontalAlignment::Stretch);
    const auto fill = palette_.dark ? rgba(255, 255, 255, .07) : rgba(0, 0, 0, .055);
    auto resources = search_.Resources();
    auto put = [&resources](const wchar_t *key, Color const &color) {
      resources.Insert(box_value(hstring(key)), brush(color));
    };
    for (const auto *suffix : {L"", L"PointerOver", L"Focused"}) {
      put((std::wstring(L"TextControlBackground") + suffix).c_str(), fill);
      put((std::wstring(L"TextControlForeground") + suffix).c_str(), palette_.text);
      put((std::wstring(L"TextControlPlaceholderForeground") + suffix).c_str(), palette_.sub);
    }
    put(L"TextControlBorderBrush", transparent_color());
    put(L"TextControlBorderBrushPointerOver", transparent_color());
    put(L"TextControlBorderBrushFocused", palette_.accent);
    // 模板的 Focused 状态把边框厚度换成这个资源（不聚焦时用控件自己的 0），四边都是 2，就是 macOS 聚焦时的强调色外框。
    resources.Insert(box_value(L"TextControlBorderThemeThicknessFocused"),
                     box_value(Thickness{2, 2, 2, 2}));
    A11y::SetName(search_, L"搜索设置");
    A11y::SetAutomationId(search_, L"settings-search");
    search_.TextChanged([this](Inspectable const &, TextChangedEventArgs const &) {
      update_search_results(std::wstring(search_.Text().c_str()));
    });
    // 与 macOS 一样，组字期间不抢回车和 Esc（`settings-navigation-search.tsx` 的 `isComposingKeyEvent`）：输入法的组字起止由这两个事件告知。
    search_composing_ = false;
    search_.TextCompositionStarted(
        [this](TextBox const &, TextCompositionStartedEventArgs const &) { search_composing_ = true; });
    search_.TextCompositionEnded(
        [this](TextBox const &, TextCompositionEndedEventArgs const &) { search_composing_ = false; });
    // 有查询时上下键在结果里移动高亮、回车打开高亮的那条，Esc 清空查询。
    search_.PreviewKeyDown([this](Inspectable const &, Input::KeyRoutedEventArgs const &args) {
      if (search_composing_)
        return;
      const auto key = args.Key();
      if (key == Windows::System::VirtualKey::Escape) {
        if (search_.Text().empty())
          return;
        args.Handled(true);
        clear_search();
        return;
      }
      if (key != Windows::System::VirtualKey::Down && key != Windows::System::VirtualKey::Up &&
          key != Windows::System::VirtualKey::Enter)
        return;
      const std::wstring typed(search_.Text().c_str());
      if (search_needle(typed).empty())
        return;
      args.Handled(true);
      // UI 自动化直接写值后马上回车时，TextChanged 可能还没轮到，先按框里的文字重查。
      if (typed != search_query_)
        update_search_results(typed);
      if (search_hits_.empty())
        return;
      if (key == Windows::System::VirtualKey::Enter) {
        open_search_result(search_hits_[std::min(search_active_, search_hits_.size() - 1)]);
        return;
      }
      const size_t count = search_hits_.size();
      search_active_ = key == Windows::System::VirtualKey::Down
                           ? (search_active_ + 1) % count
                           : (search_active_ + count - 1) % count;
      paint_search_results();
      search_buttons_[search_active_].StartBringIntoView();
      // 焦点一直在搜索框里，结果按钮也不进 Tab 顺序，读屏不会自己念换到的那条（macOS 靠搜索框的 `aria-activedescendant`）：由搜索框发一条通知，念标题和位置。
      auto const &active = search_hits_[search_active_];
      if (auto peer = Automation::Peers::FrameworkElementAutomationPeer::FromElement(search_))
        peer.RaiseNotificationEvent(
            Automation::Peers::AutomationNotificationKind::Other,
            Automation::Peers::AutomationNotificationProcessing::ImportantMostRecent,
            hstring(active.path.empty() ? active.title : active.title + L"，" + active.path),
            L"settings-search-active");
    });
    field.Children().Append(search_);

    // 放大镜照 macOS 的 16 × 16 图形：圆心 (7, 7)、半径 5 的圆和从 (11, 11) 到 (14.5, 14.5) 的柄，1.8 粗、圆头，整体缩到 15 DIP。XAML 的椭圆把描边画在宽高之内，所以外径是 10 加一个线宽。
    const double unit = 15.0 / 16.0;
    Canvas glyph;
    glyph.Width(15);
    glyph.Height(15);
    glyph.Margin(Thickness{10, 0, 0, 0});
    glyph.HorizontalAlignment(HorizontalAlignment::Left);
    glyph.VerticalAlignment(VerticalAlignment::Center);
    glyph.IsHitTestVisible(false);
    Shapes::Ellipse lens;
    lens.Width(11.8 * unit);
    lens.Height(11.8 * unit);
    lens.Stroke(brush(palette_.sub));
    lens.StrokeThickness(1.8 * unit);
    Canvas::SetLeft(lens, 1.1 * unit);
    Canvas::SetTop(lens, 1.1 * unit);
    glyph.Children().Append(lens);
    Shapes::Line handle;
    handle.X1(11 * unit);
    handle.Y1(11 * unit);
    handle.X2(14.5 * unit);
    handle.Y2(14.5 * unit);
    handle.Stroke(brush(palette_.sub));
    handle.StrokeThickness(1.8 * unit);
    handle.StrokeStartLineCap(PenLineCap::Round);
    handle.StrokeEndLineCap(PenLineCap::Round);
    glyph.Children().Append(handle);
    field.Children().Append(glyph);
    return field;
  }

  // 侧栏搜索结果，照 macOS（`use-settings-shell-navigation.tsx` 的 `navSearchResults`）：有查询时页面列表整个换成结果列表，不是下拉框。结果放在导航视图的 PaneCustomContent 里（模板中排在页面列表之上），查询期间页面列表的各项收起；列表自己滚动，最高到侧栏底部（`fit_search_results`）。读屏名「设置搜索结果」，自动化 ID 是 `settings-search-results`。
  ScrollViewer make_search_results() {
    search_list_ = StackPanel();
    search_list_.Margin(Thickness{12, 0, 12, 0});
    search_scroll_ = ScrollViewer();
    search_scroll_.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
    search_scroll_.HorizontalScrollBarVisibility(ScrollBarVisibility::Disabled);
    search_scroll_.Content(search_list_);
    search_scroll_.Visibility(Visibility::Collapsed);
    A11y::SetName(search_scroll_, L"设置搜索结果");
    A11y::SetAutomationId(search_scroll_, L"settings-search-results");
    return search_scroll_;
  }

  NavigationView make_navigation() {
    NavigationView view;
    view.PaneDisplayMode(NavigationViewPaneDisplayMode::Left);
    view.OpenPaneLength(sidebar_width);
    view.CompactModeThresholdWidth(0);
    view.ExpandedModeThresholdWidth(0);
    view.IsPaneToggleButtonVisible(false);
    view.IsBackButtonVisible(NavigationViewBackButtonVisible::Collapsed);
    view.IsSettingsVisible(false);
    view.IsTitleBarAutoPaddingEnabled(false);
    view.AlwaysShowHeader(false);

    // 通过模板资源统一侧栏和正文配色，保留原生导航行为。
    auto resources = view.Resources();
    auto put = [&resources](const wchar_t *key, Color const &color) {
      resources.Insert(box_value(hstring(key)), brush(color));
    };
    put(L"NavigationViewContentBackground", palette_.content_bg);
    // 侧栏右侧的 1 DIP 边线由正文格的左边框画出，正文格不再有 Windows 11 那个带上边框的左上圆角：上方是标题栏底部的细线，与 macOS 的侧栏和工具栏一致。
    put(L"NavigationViewContentGridBorderBrush", palette_.edge);
    resources.Insert(box_value(L"NavigationViewContentGridBorderThickness"),
                     box_value(Thickness{1, 0, 0, 0}));
    resources.Insert(box_value(L"NavigationViewContentGridCornerRadius"),
                     box_value(CornerRadius{0, 0, 0, 0}));
    put(L"NavigationViewExpandedPaneBackground", transparent_color());
    put(L"NavigationViewDefaultPaneBackground", transparent_color());
    put(L"NavigationViewSelectionIndicatorForeground", transparent_color());
    // 选中项与 macOS 一样只铺一层中性底色，文字和图标仍是正文色，没有强调色胶囊；悬停和按下用同一层底色（macOS 的 `--p-hover`）。模板里图标的前景也取 NavigationViewItemForeground* 这组键。
    put(L"NavigationViewItemBackgroundSelected", palette_.hover);
    put(L"NavigationViewItemBackgroundSelectedPointerOver", palette_.hover);
    put(L"NavigationViewItemBackgroundPointerOver", palette_.hover);
    put(L"NavigationViewItemBackgroundPressed", palette_.hover);
    put(L"NavigationViewItemForegroundPressed", palette_.text);
    put(L"NavigationViewItemForegroundSelected", palette_.text);
    put(L"NavigationViewItemForegroundSelectedPointerOver", palette_.text);
    put(L"NavigationViewItemForegroundSelectedPressed", palette_.text);
    put(L"NavigationViewItemIconForegroundSelected", palette_.text);
    put(L"NavigationViewItemIconForegroundSelectedPointerOver", palette_.text);
    put(L"NavigationViewItemIconForegroundSelectedPressed", palette_.text);
    put(L"NavigationViewItemBackgroundSelectedPressed", palette_.hover);
    // 导航项照 macOS 侧栏：可见的底色块高 36 DIP，项与项之间 2 DIP。模板在底色块外留 NavigationViewItemButtonMargin（默认左右 4、上下 2），这里收成上下各 1，左右的间距改由导航项自己的 12 DIP 外边距给出，与侧栏搜索框的左右边距一致；所以导航项本身高 38。
    resources.Insert(box_value(L"NavigationViewItemOnLeftMinHeight"), box_value(36.0));
    resources.Insert(box_value(L"NavigationViewItemButtonMargin"),
                     box_value(Thickness{0, 1, 0, 1}));
    // 模板把图标装进这个高度的 Viewbox 里缩放（默认 16），macOS 侧栏的图标是 15。
    resources.Insert(box_value(L"NavigationViewItemOnLeftIconBoxHeight"), box_value(15.0));
    put(L"NavigationViewItemSeparatorForeground", palette_.divider);

    view.PaneHeader(make_search_field());
    view.PaneCustomContent(make_search_results());
    view.SizeChanged([this](Inspectable const &, SizeChangedEventArgs const &) {
      fit_search_results();
    });

    // 与 macOS 侧栏一样平铺页面，不插分组标题；顺序取自 `SettingsNavigation.h`。
    for (const auto &page : nav::pages) {
      const auto &label = page_label(page.id);
      NavigationViewItem item;
      item.Content(box_value(hstring(label.title)));
      // 产品图标照 macOS 侧栏：15 DIP、不透明度 0.9（`settings-style.ts` 的 `navIcon`）。前景交给模板的 NavigationViewItemForeground* 这组键，也就是正文色。
      auto icon = make_nav_icon(page.id, 15);
      icon.Opacity(0.9);
      item.Icon(icon);
      item.FontSize(14);
      item.MinHeight(38);
      item.Height(38);
      // 圆角经模板的 TemplateBinding 落到底色块上；不能改 ControlCornerRadius，那会连正文里的控件一起改掉。
      item.CornerRadius(CornerRadius{8, 8, 8, 8});
      item.Margin(Thickness{12, 0, 12, 0});
      item.Padding(Thickness{10, 0, 10, 0});
      item.VerticalContentAlignment(VerticalAlignment::Center);
      item.Tag(box_value(text(page.id)));
      A11y::SetAutomationId(item, text(page.id));
      view.MenuItems().Append(item);
    }
    view.SelectionChanged(
        [this](NavigationView const &sender,
               NavigationViewSelectionChangedEventArgs const &) {
          if (selecting_)
            return;
          const auto item = sender.SelectedItem().try_as<NavigationViewItem>();
          if (item)
            navigate(utf8(unbox_value<hstring>(item.Tag())));
        });

    scroll_ = ScrollViewer();
    A11y::SetAutomationId(scroll_, L"settings-content");
    scroll_.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
    scroll_.HorizontalScrollBarVisibility(ScrollBarVisibility::Disabled);
    view.Content(scroll_);
    return view;
  }

  void select_navigation_item() {
    for (auto const &value : navigation_.MenuItems()) {
      const auto item = value.try_as<NavigationViewItem>();
      if (item && utf8(unbox_value<hstring>(item.Tag())) == current_page_) {
        selecting_ = true;
        navigation_.SelectedItem(item);
        selecting_ = false;
        return;
      }
    }
  }

  void navigate(std::string const &page) {
    const auto *target = nav::find_page(page);
    const std::string id(target ? target->id : nav::default_page);
    flush_pending();
    if (id == current_page_ && focus_row_.empty())
      return;
    // 每次进入「输入」页都重读自定义辅助码表；插件和整句模型资源包已移除。
    if (id == "typing" && id != current_page_)
      helpcode_schemas_requested_ = false;
    current_page_ = id;
    select_navigation_item();
    render_page(false);
  }

  // ---- Search ----

  void rebuild_search_index() {
    search_index_.clear();
    search_folds_.clear();
    indexing_ = true;
    const auto saved_page = current_page_;
    for (const auto &page : nav::pages) {
      current_page_ = std::string(page.id);
      tab_index_.clear();
      index_tabs_.clear();
      group_index_.clear();
      StackPanel scratch;
      build_page(page.id, scratch);
    }
    current_page_ = saved_page;
    tab_index_.clear();
    index_tabs_.clear();
    group_index_.clear();
    indexing_ = false;
    index_revision_ = document_.revision();
    index_valid_ = true;
  }

  // 说明不进索引：与 macOS 一样只按行标题匹配（`settings-search-index.ts` 不读说明、表单值和历史），调用处照常传行的说明。结果的第二行是「页 › 标签 › 分组」：标签取建索引时正在画的那一块（`in_tab`），分组取这一行之前最近的 `add_group` 标题。
  void record(std::wstring const &title, [[maybe_unused]] std::wstring const &detail) {
    if (!indexing_ || title.empty())
      return;
    if (!fold_index_.empty())
      search_folds_[title] = fold_index_;
    SearchEntry entry;
    entry.title = title;
    entry.page = current_page_;
    entry.tab = tab_index_;
    entry.path = page_label(current_page_).title;
    if (!tab_index_.empty())
      for (const auto &tab : index_tabs_)
        if (tab.value == tab_index_) {
          entry.path += L" › " + tab.label;
          break;
        }
    if (!group_index_.empty())
      entry.path += L" › " + group_index_;
    search_index_.push_back(std::move(entry));
  }

  // 搜索结果落在折叠区里的一行时，那个折叠区要展开才看得见它：索引时 record 记下每行所在的折叠区（`fold_index_`）。
  bool fold_holds_focus(std::wstring const &fold) const {
    const auto found = search_folds_.find(focus_row_);
    return found != search_folds_.end() && found->second == fold;
  }

  // The element a search result lands on: the first one drawn under the chosen title. Every element that records a search entry marks itself here, so a check box, a theme card or a section is scrolled to like a settings card; one that is a Control (a check box) is focused as well.
  void mark_search_target(std::wstring const &title,
                          FrameworkElement const &element) {
    if (!indexing_ && !focus_row_.empty() && title == focus_row_ &&
        !focus_target_)
      focus_target_ = element;
  }

  // 查询按 JavaScript 的 trim 去掉两头空白再转小写，与 macOS 的 `navQuery.trim().toLocaleLowerCase()` 相同；为空表示没在搜索。
  static std::wstring search_needle(std::wstring const &query) {
    return lowercase(nav::text::trim(query));
  }

  // 结果照 macOS 的顺序：先是名字匹配的页面（侧栏顺序），再是标题匹配的设置行（索引顺序），不设条数上限。同一页同一位置的同名行只列一次。
  std::vector<SearchEntry> search(std::wstring const &query) {
    std::vector<SearchEntry> matches;
    const auto needle = search_needle(query);
    if (needle.empty())
      return matches;
    if (!index_valid_ || index_revision_ != document_.revision())
      rebuild_search_index();
    for (const auto &page : nav::pages) {
      const auto names = page_search_names(page.id);
      if (std::none_of(names.begin(), names.end(), [&needle](std::wstring_view name) {
            return lowercase(name).find(needle) != std::wstring::npos;
          }))
        continue;
      SearchEntry entry;
      entry.title = page_label(page.id).title;
      entry.page = std::string(page.id);
      entry.whole_page = true;
      matches.push_back(std::move(entry));
    }
    for (const auto &entry : search_index_) {
      if (lowercase(entry.title).find(needle) == std::wstring::npos)
        continue;
      const bool repeated = std::any_of(matches.begin(), matches.end(), [&entry](SearchEntry const &shown) {
        return !shown.whole_page && shown.title == entry.title && shown.page == entry.page &&
               shown.path == entry.path;
      });
      if (!repeated)
        matches.push_back(entry);
    }
    return matches;
  }

  // 按查询重建结果列表：有查询时收起页面列表、显示结果，第一条高亮；查询清空时换回页面列表。
  void update_search_results(std::wstring const &query) {
    search_query_ = query;
    const bool searching = !search_needle(query).empty();
    search_hits_ = search(query);
    search_active_ = 0;
    search_buttons_.clear();
    if (!search_list_ || !navigation_)
      return;
    // 上一次已经是「没有匹配的设置」时，接着输入仍查不到就不再播报。
    const bool was_empty = search_list_.Children().Size() == 1 &&
                           static_cast<bool>(search_list_.Children().GetAt(0).try_as<TextBlock>());
    search_list_.Children().Clear();
    for (auto const &value : navigation_.MenuItems())
      if (const auto item = value.try_as<NavigationViewItem>())
        item.Visibility(searching ? Visibility::Collapsed : Visibility::Visible);
    search_scroll_.Visibility(searching ? Visibility::Visible : Visibility::Collapsed);
    if (!searching)
      return;
    fit_search_results();
    search_scroll_.ChangeView(nullptr, 0.0, nullptr, true);
    if (search_hits_.empty()) {
      // 照 macOS 的 `role="status"`：14 DIP、60% 不透明度，读屏礼貌播报。新建的文字块只设 LiveSetting 不会被念出来，进了可视树后要自己发 LiveRegionChanged，与 show_warning 相同；接着输入仍查不到时不再发，与 macOS 那个文字不变的状态元素一样只念一次。
      auto empty = make_text(no_search_results, 14, palette_.text);
      empty.Opacity(0.6);
      empty.Margin(Thickness{12, 8, 12, 8});
      A11y::SetLiveSetting(empty, Automation::Peers::AutomationLiveSetting::Polite);
      if (!was_empty)
        empty.Loaded([](Inspectable const &sender, RoutedEventArgs const &) {
          if (auto peer = Automation::Peers::FrameworkElementAutomationPeer::CreatePeerForElement(
                  sender.as<UIElement>()))
            peer.RaiseAutomationEvent(Automation::Peers::AutomationEvents::LiveRegionChanged);
        });
      search_list_.Children().Append(empty);
      return;
    }
    for (size_t i = 0; i < search_hits_.size(); ++i) {
      auto button = make_search_result(search_hits_[i], i);
      search_buttons_.push_back(button);
      search_list_.Children().Append(button);
    }
    paint_search_results();
  }

  // 一条结果照 macOS 的结果按钮：内边距左右 12、上下 8，圆角 8，条与条相隔 4；13 DIP 标题，下面相隔 4 是 12 DIP、60% 不透明度的位置。悬停铺 10% 的强调色，高亮的那条是 15%（`paint_search_results`）。结果不进 Tab 顺序，键盘在搜索框里用上下键选，换到的那条由搜索框发通知念出；读屏名是标题，帮助文本是位置。
  Button make_search_result(SearchEntry const &hit, size_t index) {
    StackPanel lines;
    lines.Spacing(4);
    lines.Children().Append(make_text(hit.title, 13, palette_.text));
    if (!hit.path.empty()) {
      auto path = make_text(hit.path, 12, palette_.text);
      path.Opacity(0.6);
      lines.Children().Append(path);
    }
    Button button;
    button.Content(lines);
    button.HorizontalAlignment(HorizontalAlignment::Stretch);
    button.HorizontalContentAlignment(HorizontalAlignment::Left);
    button.Padding(Thickness{12, 8, 12, 8});
    button.Margin(Thickness{0, 0, 0, 4});
    button.CornerRadius(CornerRadius{8, 8, 8, 8});
    button.BorderThickness(Thickness{0, 0, 0, 0});
    button.IsTabStop(false);
    auto resources = button.Resources();
    auto put = [&resources](const wchar_t *key, Color const &color) {
      resources.Insert(box_value(hstring(key)), brush(color));
    };
    put(L"ButtonBackground", transparent_color());
    put(L"ButtonBackgroundPointerOver", faded(palette_.accent, .1));
    put(L"ButtonBackgroundPressed", faded(palette_.accent, .15));
    put(L"ButtonForeground", palette_.text);
    put(L"ButtonForegroundPointerOver", palette_.text);
    put(L"ButtonForegroundPressed", palette_.text);
    A11y::SetName(button, hstring(hit.title));
    if (!hit.path.empty())
      A11y::SetHelpText(button, hstring(hit.path));
    A11y::SetAutomationId(button, hstring(L"settings-search-result-" + std::to_wstring(index)));
    button.Click([this, hit](Inspectable const &, RoutedEventArgs const &) {
      // 打开结果会清空查询、卸下这个按钮，放到下一轮再做。
      DispatcherQueue().TryEnqueue([weak = get_weak(), hit] {
        if (auto self = weak.get())
          self->open_search_result(hit);
      });
    });
    return button;
  }

  // 高亮的那条铺 15% 的强调色，ItemStatus 是「已选择」，与页内标签的写法相同。
  void paint_search_results() {
    for (size_t i = 0; i < search_buttons_.size(); ++i) {
      const bool active = i == search_active_;
      search_buttons_[i].Background(brush(active ? faded(palette_.accent, .15) : transparent_color()));
      A11y::SetItemStatus(search_buttons_[i], active ? L"已选择" : L"");
    }
  }

  // PaneCustomContent 所在的模板行按内容定高，结果多了会伸出侧栏，所以给列表定一个最高值：从搜索框底下（加上搜索框外框的 8 DIP 下边距）到侧栏底部，再留 16。搜索框还没进可视树时（换明暗重建外壳后写回查询）先不算，导航视图第一次排版的 SizeChanged 会再算。
  void fit_search_results() {
    if (!search_scroll_ || !navigation_ || !search_ || !search_.IsLoaded())
      return;
    const auto bottom = search_.TransformToVisual(navigation_)
                            .TransformPoint(point_at(0, search_.ActualHeight()))
                            .Y;
    search_scroll_.MaxHeight(std::max(0.0, navigation_.ActualHeight() - bottom - 8 - 16));
  }

  // 打开一条结果，照 macOS 的 `openResult`：先清空查询、换回页面列表，整页结果只切到那一页；设置行切到它所在的页和标签，画好后滚到那一行，是控件时把焦点交给它（`render_page_at`），所在的折叠区由 `fold_holds_focus` 展开。
  void open_search_result(SearchEntry hit) {
    clear_search();
    if (hit.whole_page) {
      navigate(hit.page);
      return;
    }
    focus_row_ = hit.title;
    if (!hit.tab.empty())
      page_tabs_[hit.page] = hit.tab;
    if (hit.page == current_page_) {
      render_page(true);
      return;
    }
    navigate(hit.page);
  }

  // 清空查询：TextBox 的 TextChanged 是异步的，这里直接换回页面列表，之后到的 TextChanged 再算一遍也是同样的结果。
  void clear_search() {
    if (search_)
      search_.Text(L"");
    update_search_results(L"");
  }

  // ---- Page scaffolding ----

  void render_page(bool keep_offset) {
    if (!scroll_)
      return;
    render_page_at(keep_offset ? scroll_.VerticalOffset() : 0.0);
  }

  // Draws the current page and scrolls to `offset`, or to the element a search result named.
  void render_page_at(double offset) {
    if (!scroll_)
      return;
    toolbar_title_.Text(page_label(current_page_).title);
    preview_host_ = nullptr;
    preview_frame_ = nullptr;
    toolbar_preview_host_ = nullptr;
    toolbar_preview_label_ = nullptr;
    font_preset_host_ = nullptr;
    font_catalog_text_ = nullptr;
    font_catalog_button_ = nullptr;
    font_menu_updaters_.clear();
    focus_target_ = nullptr;

    StackPanel page;
    page.Padding(content_padding(scroll_.ActualWidth()));
    page.SizeChanged([](Inspectable const &sender, SizeChangedEventArgs const &args) {
      sender.as<StackPanel>().Padding(content_padding(args.NewSize().Width));
    });
    // 正文列不跟着窗口无限变宽。共享样式给 Windows 和 macOS 的是 `max-w-none`（`settings-style.ts` 的 `contentColumn`），那是照 macOS 定的，而 Windows 上最大化是常态：在 1700 宽的窗口里一行的标签贴着最左、控件贴着最右，中间一千多 DIP 全是空白，单选项也被拉成几百 DIP 宽的大块。上限取共享样式自己的基准 900（Linux 680、HarmonyOS 760、iPad 720 也都限宽），初始的 1000 × 780 下正文只有 730 DIP，这个值不改变它的样子，只在窗口拉宽后把正文居中留白。
    page.Spacing(4);
    if (input_method_ == InputMethodState::missing ||
        input_method_ == InputMethodState::not_default)
      page.Children().Append(make_input_method_banner());
    page.Children().Append(make_page_header());
    if (!loaded_)
      page.Children().Append(make_load_failure());
    if (!notice_.empty())
      page.Children().Append(make_notice());
    build_page(current_page_, page);
    scroll_.Content(page);

    // 页面持有回调，弱引用避免页面与控件互相持有。
    const bool focus = static_cast<bool>(focus_target_);
    weak_ref<FrameworkElement> weak_target;
    if (focus)
      weak_target = make_weak(focus_target_);
    focus_row_.clear();
    page.Loaded([weak_scroll = make_weak(scroll_), offset, focus,
                 weak_target](Inspectable const &, RoutedEventArgs const &) {
      if (focus) {
        if (auto target = weak_target.get()) {
          target.StartBringIntoView();
          // A control (a check box) also takes keyboard focus, so its focus rectangle shows which one the result named; a card, including a theme card, is only scrolled to.
          if (const auto control = target.try_as<Control>())
            control.Focus(FocusState::Keyboard);
        }
        return;
      }
      if (auto scroll = weak_scroll.get()) {
        scroll.UpdateLayout();
        scroll.ChangeView(nullptr, offset, nullptr, true);
      }
    });
  }

  // 正文列最宽 1400 DIP。再宽下去，一行里的标签贴着最左、控件贴着最右，中间拉开上千 DIP 的空白，读起来要来回扫；而限得太窄（曾经按共享样式的 900）在 2560 的屏上最大化后正文只占三分之一，缩成窄窄一条浮在中间。1400 在 2560 上占六成左右，两侧留白不喧宾夺主。共享样式给 Windows 和 macOS 的是 `max-w-none`，那份规格按 macOS 定，而 macOS 很少有人最大化设置窗口。
  static constexpr double content_maximum_width = 1800;

  // 正文边距照 macOS 的 `contentColumn`（`settings-style.ts` 的 `mac:px-12 mac:pt-6 mac:pb-11`）：上 24、左右 48、下 44。macOS 窄于 601 时收起侧栏，这个窗口的侧栏常驻，窗口拉到最窄（800）时正文只剩 540 宽，左右退到 32，给卡片多留些宽度。
  // 超过上限的部分摊进左右边距，正文因此居中。**不要**改用 `HorizontalAlignment::Center` 加 `MaxWidth`：居中会让面板按自身内容的自然宽度排版，不再拉伸，`MaxWidth` 于是永远碰不到，正文宽度变成卡片内容撑出来的那个值，改上限毫无效果。
  static Thickness content_padding(double width) {
    const double side = width > 0 && width < 640 ? 32 : 48;
    const double overflow = width > content_maximum_width
                                ? (width - content_maximum_width) / 2
                                : 0;
    return Thickness{side + overflow, 24, side + overflow, 44};
  }

  // 页名只在标题栏里（与 macOS 的页面工具栏相同），正文不再画大标题。正文顶部照 macOS 的页首引导（`settings-mascot-intro.tsx` 的 `SettingsMascotIntro`）：左边一句 13 DIP、1.625 倍行高的次要色说明，右边一张 80 DIP 的水杉娘插画，两者竖直居中、相隔 20，整行左右各留 4；窗口矮于 700 DIP 时插画缩到 56、间隔收成 12（`update_intro_size`）。说明和插画按页面与当前标签取自 `SettingsMascotIntro.h`，「关于」没有这一行。「候选窗口」页在它下面右侧放 Windows 自己的预览工具（「界面预览」开关）；说明带 `settings-page-<页面 id>` 的自动化 ID，界面验收靠它确认正文已换成这一页；「关于」页由品牌卡上的产品名带这个 ID。
  FrameworkElement make_page_header() {
    Grid row;
    ColumnDefinition title_column;
    title_column.Width(GridLength{1, GridUnitType::Star});
    ColumnDefinition tools_column;
    tools_column.Width(GridLength{1, GridUnitType::Auto});
    row.ColumnDefinitions().Append(title_column);
    row.ColumnDefinitions().Append(tools_column);

    intro_row_ = nullptr;
    intro_text_ = nullptr;
    intro_mascot_ = nullptr;
    // 「关于」没有页首引导（`settings-mascot-intro.tsx` 里没有 about）。
    if (nav::mascot_intro(current_page_)) {
      Grid intro;
      intro.Padding(Thickness{4, 0, 4, 0});
      ColumnDefinition description_column;
      description_column.Width(GridLength{1, GridUnitType::Star});
      ColumnDefinition mascot_column;
      mascot_column.Width(GridLength{1, GridUnitType::Auto});
      intro.ColumnDefinitions().Append(description_column);
      intro.ColumnDefinitions().Append(mascot_column);
      auto description = make_text(L"", 13, palette_.sub);
      description.LineHeight(13 * 1.625);
      description.VerticalAlignment(VerticalAlignment::Center);
      A11y::SetAutomationId(description, text("settings-page-" + current_page_));
      intro.Children().Append(description);
      Image mascot;
      mascot.Stretch(Stretch::Uniform);
      mascot.VerticalAlignment(VerticalAlignment::Center);
      // 纯装饰：旁边的说明已经讲清这一页，读屏不读它，与共享 UI 插画的 aria-hidden 相同。
      A11y::SetAccessibilityView(mascot, Microsoft::UI::Xaml::Automation::Peers::AccessibilityView::Raw);
      Grid::SetColumn(mascot, 1);
      intro.Children().Append(mascot);
      Grid::SetColumnSpan(intro, 2);
      row.Children().Append(intro);
      intro_row_ = intro;
      intro_text_ = description;
      intro_mascot_ = mascot;
      show_page_intro_tab(utf8(hstring(page_tab(current_page_))));
      update_intro_size();
    }
    RowDefinition heading_row;
    heading_row.Height(GridLength{1, GridUnitType::Auto});
    RowDefinition actions_row;
    actions_row.Height(GridLength{1, GridUnitType::Auto});
    row.RowDefinitions().Append(heading_row);
    row.RowDefinitions().Append(actions_row);

    if (has_preview(current_page_)) {
      StackPanel tools;
      tools.Orientation(Orientation::Horizontal);
      tools.Spacing(20);
      tools.VerticalAlignment(VerticalAlignment::Center);
      StackPanel switcher;
      switcher.Orientation(Orientation::Horizontal);
      switcher.Spacing(12);
      auto caption = make_text(L"界面预览", 14, palette_.sub);
      caption.VerticalAlignment(VerticalAlignment::Center);
      switcher.Children().Append(caption);
      ToggleSwitch toggle;
      toggle.OnContent(box_value(L""));
      toggle.OffContent(box_value(L""));
      toggle.MinWidth(0);
      toggle.IsOn(preview_visible_);
      A11y::SetName(toggle, L"界面预览");
      toggle.Toggled([this](Inspectable const &sender, RoutedEventArgs const &) {
        preview_visible_ = sender.as<ToggleSwitch>().IsOn();
        queue_refresh();
      });
      switcher.Children().Append(toggle);
      tools.Children().Append(switcher);
      Grid::SetRow(tools, 1);
      Grid::SetColumnSpan(tools, 2);
      tools.HorizontalAlignment(HorizontalAlignment::Right);
      tools.Margin(Thickness{0, 8, 0, 0});
      row.Children().Append(tools);
    }
    return row;
  }

  // 把页首引导换成这一页 `tab` 标签的说明和插画，不重画整页；`tab` 为空或这一页没有这个标签时是第一个标签的那句。`make_page_header` 用这一页现在的标签（`page_tabs_`）调用，切标签时整页重画，引导随之换掉。这一页没有引导时什么也不做。
  void show_page_intro_tab(std::string_view tab) {
    if (!intro_text_ || !intro_mascot_)
      return;
    const auto intro = nav::mascot_intro(current_page_, tab);
    if (!intro)
      return;
    intro_text_.Text(hstring(intro->description));
    auto bitmap = mascot_bitmaps_.find(intro->scene);
    if (bitmap == mascot_bitmaps_.end())
      bitmap = mascot_bitmaps_.emplace(intro->scene, load_mascot(intro->scene)).first;
    intro_mascot_.Source(bitmap->second);
  }

  // macOS 的引导行在视口矮于 700 时把插画缩到 56、间隔收成 12（`[@media(max-height:700px)]`）。这里按窗口内容区的高度判断，窗口拉高拉矮时由 `build_shell` 里根元素的 SizeChanged 跟着改。
  void update_intro_size() {
    if (!intro_row_ || !intro_mascot_)
      return;
    const double height = root_ ? root_.ActualHeight() : 0;
    const bool short_window = height > 0 && height < 700;
    const double size = short_window ? 56 : 80;
    intro_mascot_.Width(size);
    intro_mascot_.Height(size);
    intro_row_.ColumnSpacing(short_window ? 12 : 20);
  }

  static bool has_preview(std::string_view page) {
    return page == "candidate";
  }

  FrameworkElement make_load_failure() {
    InfoBar bar;
    bar.IsOpen(true);
    bar.IsClosable(false);
    bar.Severity(InfoBarSeverity::Error);
    bar.Title(L"无法读取设置");
    A11y::SetAutomationId(bar, L"settings-load-error");
    bar.Message(hstring(load_error_.empty() ? L"读取设置失败，请稍后重试。"
                                            : load_error_));
    bar.Margin(Thickness{0, 0, 0, 12});
    Button retry;
    retry.Content(box_value(L"重试"));
    retry.Click([this](Inspectable const &, RoutedEventArgs const &) {
      reload_document();
      queue_refresh();
    });
    bar.ActionButton(retry);
    return bar;
  }

  // The design's deskStrip: an orange notice above the page title while the input method is not in the keyboard list, or is not the default one, with the button that fixes it.
  FrameworkElement make_input_method_banner() {
    const bool missing = input_method_ == InputMethodState::missing;
    const std::wstring message = missing ? L"水杉输入法尚未在「语言和区域」中添加"
                                         : L"水杉输入法还不是默认输入法";
    const std::wstring action_label = missing ? L"去添加" : L"设为默认";

    Border strip;
    strip.Padding(Thickness{16, 10, 16, 10});
    strip.Margin(Thickness{0, 0, 0, 16});
    strip.CornerRadius(CornerRadius{4, 4, 4, 4});
    strip.BorderThickness(Thickness{1, 1, 1, 1});
    strip.Background(brush(palette_.dark ? rgba(255, 149, 0, .12)
                                         : rgba(0xFF, 0xF4, 0xE5, 1)));
    strip.BorderBrush(brush(palette_.dark ? rgba(255, 149, 0, .3)
                                          : rgba(0xFF, 0xD8, 0xA8, 1)));

    Grid row;
    row.ColumnSpacing(12);
    ColumnDefinition mark_column;
    mark_column.Width(GridLength{1, GridUnitType::Auto});
    ColumnDefinition text_column;
    text_column.Width(GridLength{1, GridUnitType::Star});
    ColumnDefinition action_column;
    action_column.Width(GridLength{1, GridUnitType::Auto});
    row.ColumnDefinitions().Append(mark_column);
    row.ColumnDefinitions().Append(text_column);
    row.ColumnDefinitions().Append(action_column);

    Border mark;
    mark.Width(18);
    mark.Height(18);
    mark.CornerRadius(CornerRadius{9, 9, 9, 9});
    mark.Background(brush(rgba(0xFF, 0x95, 0x00, 1)));
    mark.VerticalAlignment(VerticalAlignment::Center);
    auto bang = make_text(L"!", 12, rgba(255, 255, 255, 1));
    bang.FontWeight(Windows::UI::Text::FontWeight{700});
    bang.HorizontalAlignment(HorizontalAlignment::Center);
    bang.VerticalAlignment(VerticalAlignment::Center);
    mark.Child(bang);
    A11y::SetAccessibilityView(mark,
                               Microsoft::UI::Xaml::Automation::Peers::AccessibilityView::Raw);
    Grid::SetColumn(mark, 0);
    row.Children().Append(mark);

    auto text_block = make_text(message, 14, palette_.text);
    text_block.VerticalAlignment(VerticalAlignment::Center);
    Grid::SetColumn(text_block, 1);
    row.Children().Append(text_block);

    // The fixed accent fill of the design, kept through hover and press rather than the system accent the default button styles use. It carries white text, so it is the strong accent in both appearances.
    Button action;
    action.Content(box_value(hstring(action_label)));
    action.FontSize(13);
    action.Padding(Thickness{12, 4, 12, 4});
    action.CornerRadius(CornerRadius{4, 4, 4, 4});
    action.BorderThickness(Thickness{0, 0, 0, 0});
    action.VerticalAlignment(VerticalAlignment::Center);
    auto resources = action.Resources();
    auto put = [&resources](const wchar_t *key, Color const &color) {
      resources.Insert(box_value(hstring(key)), brush(color));
    };
    const auto accent = palette_.accent_strong;
    auto faded = [accent](double alpha) {
      return Color{static_cast<uint8_t>(std::lround(alpha * 255)), accent.R,
                   accent.G, accent.B};
    };
    put(L"ButtonBackground", accent);
    put(L"ButtonBackgroundPointerOver", faded(.9));
    put(L"ButtonBackgroundPressed", faded(.8));
    put(L"ButtonForeground", palette_.on_accent);
    put(L"ButtonForegroundPointerOver", palette_.on_accent);
    put(L"ButtonForegroundPressed", palette_.on_accent);
    A11y::SetName(action, hstring(action_label));
    A11y::SetHelpText(action, hstring(message));
    action.Click([this, missing](Inspectable const &, RoutedEventArgs const &) {
      if (missing) {
        if (!add_input_method()) {
          // Windows refused (a policy, or a profile that is not registered): the user adds it by hand in 语言和区域.
          const auto result = reinterpret_cast<INT_PTR>(
              ShellExecuteW(nullptr, L"open", L"ms-settings:regionlanguage",
                            nullptr, nullptr, SW_SHOWNORMAL));
          if (result <= 32)
            show_notice(L"无法添加水杉输入法。请在「设置 → 时间和语言 → 语言和区域」中为中文添加水杉输入法。");
        }
      } else if (!make_input_method_default()) {
        show_notice(L"无法设为默认输入法。请在「设置 → 时间和语言 → 输入 → 高级键盘设置」中选择水杉输入法。");
      }
      input_method_ = input_method_state();
      queue_refresh();
    });
    Grid::SetColumn(action, 2);
    row.Children().Append(action);

    strip.Child(row);
    A11y::SetName(strip, hstring(message));
    return strip;
  }

  FrameworkElement make_notice() {
    InfoBar bar;
    bar.IsOpen(true);
    bar.IsClosable(true);
    A11y::SetAutomationId(bar, L"settings-notice");
    bar.Severity(notice_severity_);
    bar.Message(hstring(notice_));
    bar.Margin(Thickness{0, 0, 0, 12});
    bar.Closed([this](InfoBar const &, InfoBarClosedEventArgs const &) {
      notice_.clear();
      notice_severity_ = InfoBarSeverity::Error;
    });
    return bar;
  }

  void show_notice(std::wstring message,
                   InfoBarSeverity severity = InfoBarSeverity::Error) {
    notice_ = std::move(message);
    notice_severity_ = severity;
    queue_refresh();
  }

  // 标题位于卡片之外；同一功能组的行共用一张圆角卡片。密度照 macOS 的平台 token（`platform-tokens.ts` 的 `groupTitle`、`groupsGap`）：组名 14 DIP 半粗体、左右缩进 8，与卡片相隔 8；组与组相隔 24（这里的上边距 20 加页面的 Spacing 4）。卡片里的行面板左右留 16 DIP：不是行的内容（说明、表单、子卡片）照这个缩进，`add_row` 等行再用负边距伸到卡片两边，分隔线和悬停底色因此贯穿整行。
  StackPanel add_group(StackPanel const &page, std::wstring const &title) {
    // 建索引时记下组名，之后 record 的行在搜索结果里写成「页 › 分组」。
    if (indexing_)
      group_index_ = title;
    StackPanel section;
    section.Spacing(8);
    section.Margin(Thickness{0, 20, 0, 0});
    if (!title.empty()) {
      // 组名和它旁边的「?」（explain_group 收进来的组说明）排在一行，登记在 group_headers_ 里供 explain_group 找到。
      StackPanel header;
      header.Orientation(Orientation::Horizontal);
      header.Spacing(4);
      header.Margin(Thickness{8, 0, 8, 0});
      auto heading = make_text(title, 14, palette_.text);
      heading.FontWeight(Windows::UI::Text::FontWeight{600});
      heading.VerticalAlignment(VerticalAlignment::Center);
      header.Children().Append(heading);
      section.Children().Append(header);
    }
    StackPanel rows;
    rows.Padding(Thickness{row_inset, 0, row_inset, 0});
    auto card = make_card();
    card.Child(rows);
    section.Children().Append(card);
    page.Children().Append(section);
    if (!title.empty())
      group_headers_.push_back({rows, section.Children().GetAt(0).as<StackPanel>(), card});
    return rows;
  }

  // 设置项说明的「?」，对应共享页的 InfoTip：解释性说明不常驻页面，收进标题旁 14 DIP 的圆圈问号（Segoe Fluent Icons 的 Unknown），鼠标悬停或键盘聚焦时用系统 ToolTip 显示，点按（触屏、鼠标）时也打开或收起。问号是次要文字色，悬停、按下时变成正文色。读屏名称见 info_tip_name，正文就在名称里。`passive` 用在本身就是按钮的整行（跳转行、插件导航卡片）：按钮里不再嵌按钮，问号只是一个悬停出 ToolTip 的图标，不进无障碍树，说明由行按钮的 HelpText 读出。
  FrameworkElement info_tip(std::wstring const &title, std::wstring const &text, bool passive = false) {
    FontIcon mark;
    mark.FontFamily(Media::FontFamily(L"Segoe Fluent Icons,Segoe MDL2 Assets"));
    mark.Glyph(glyph_text(0xE9CE));
    mark.FontSize(14);
    ToolTip tip;
    tip.Content(box_value(hstring(text)));
    if (passive) {
      // 透明底让整个方块都能悬停，而不只是字形的笔画。
      Border icon;
      icon.Background(brush(transparent_color()));
      icon.Padding(Thickness{3, 3, 3, 3});
      icon.VerticalAlignment(VerticalAlignment::Center);
      mark.Foreground(brush(palette_.sub));
      icon.Child(mark);
      ToolTipService::SetToolTip(icon, tip);
      A11y::SetAccessibilityView(icon, Microsoft::UI::Xaml::Automation::Peers::AccessibilityView::Raw);
      return icon;
    }
    Button button;
    button.Content(mark);
    button.Padding(Thickness{3, 3, 3, 3});
    button.MinWidth(0);
    button.MinHeight(0);
    button.BorderThickness(Thickness{0, 0, 0, 0});
    button.CornerRadius(CornerRadius{10, 10, 10, 10});
    button.VerticalAlignment(VerticalAlignment::Center);
    button.Background(brush(transparent_color()));
    button.Foreground(brush(palette_.sub));
    auto resources = button.Resources();
    auto put = [&resources](const wchar_t *key, Color const &color) {
      resources.Insert(box_value(hstring(key)), brush(color));
    };
    put(L"ButtonBackground", transparent_color());
    put(L"ButtonBackgroundPointerOver", transparent_color());
    put(L"ButtonBackgroundPressed", transparent_color());
    put(L"ButtonBorderBrush", transparent_color());
    put(L"ButtonBorderBrushPointerOver", transparent_color());
    put(L"ButtonBorderBrushPressed", transparent_color());
    put(L"ButtonForeground", palette_.sub);
    put(L"ButtonForegroundPointerOver", palette_.text);
    put(L"ButtonForegroundPressed", palette_.text);
    ToolTipService::SetToolTip(button, tip);
    A11y::SetName(button, hstring(info_tip_name(title, text)));
    // 从按钮上取回 ToolTip，不在处理函数里捕获它：按钮经 ToolTipService 持有它，再反过来捕获会成环。
    button.Click([](Inspectable const &sender, RoutedEventArgs const &) {
      if (auto shown = ToolTipService::GetToolTip(sender.as<DependencyObject>()).try_as<ToolTip>())
        shown.IsOpen(!shown.IsOpen());
    });
    return button;
  }

  // 组里一块内容（提示词编辑框、模型列表）的标题：14 DIP 正文色，说明收进旁边的「?」。
  StackPanel block_heading(std::wstring const &title, std::wstring const &description) {
    StackPanel heading;
    heading.Orientation(Orientation::Horizontal);
    heading.Spacing(4);
    auto name = make_text(title, 14, palette_.text);
    name.VerticalAlignment(VerticalAlignment::Center);
    heading.Children().Append(name);
    if (!description.empty())
      heading.Children().Append(info_tip(title, description));
    return heading;
  }

  // 「?」的读屏名称，四端一致：有标题时是「<标题>的说明：<正文>」，没有标题时是「说明：<正文>」。名称里已经带着正文，不再另设 HelpText，读屏不会把说明读两遍。
  static std::wstring info_tip_name(std::wstring const &title, std::wstring const &text) {
    return (title.empty() ? std::wstring(L"说明：") : title + L"的说明：") + text;
  }

  // 「?」里的说明正文，取自它的 ToolTip（info_tip 把正文放在那里）。
  static std::wstring info_tip_text(FrameworkElement const &tip) {
    if (auto shown = ToolTipService::GetToolTip(tip).try_as<ToolTip>())
      return std::wstring(unbox_value_or<hstring>(shown.Content(), L""));
    return {};
  }

  // 分组的解释性说明收进组名旁的「?」，对应共享页有组名的 GroupList 收集 SettingsGroupNote。同一组说了几段时接在同一个「?」里，段与段之间空一行。`group` 不是 add_group 给的有组名的行面板时，照旧把说明画在原处。状态、校验和错误反馈、空列表和截断提示、风险警告、紧挨按钮讲后果的话不走这里，仍用 group_note 画在原处。组里除了说明没有别的内容时（语音页的「本地识别」「Windows 系统语音」），说明移走后卡片是空的，组建完（Loaded）时收起卡片，只留组名和「?」。
  void explain_group(StackPanel const &group, std::wstring const &text) {
    // 按 add_group 登记的组头找，不沿 Parent() 往上找：在 CI 的真实窗口里那条父链取不到组头，说明全都退回了组内。
    const auto entry = std::find_if(group_headers_.begin(), group_headers_.end(),
                                    [&group](GroupHeader const &known) { return known.rows == group; });
    if (entry == group_headers_.end()) {
      group.Children().Append(group_note(text));
      return;
    }
    const auto header = entry->header;
    const auto card = entry->card;
    const std::wstring title(header.Children().GetAt(0).as<TextBlock>().Text());
    if (header.Children().Size() > 1) {
      auto tip = header.Children().GetAt(1).as<FrameworkElement>();
      const auto combined = info_tip_text(tip) + L"\n\n" + text;
      A11y::SetName(tip, hstring(info_tip_name(title, combined)));
      if (auto shown = ToolTipService::GetToolTip(tip).try_as<ToolTip>())
        shown.Content(box_value(hstring(combined)));
      return;
    }
    header.Children().Append(info_tip(title, text));
    card.Loaded([](Inspectable const &sender, RoutedEventArgs const &) {
      const auto box = sender.as<Border>();
      if (auto rows = box.Child().try_as<Panel>(); rows && rows.Children().Size() == 0)
        box.Visibility(Visibility::Collapsed);
    });
  }

  // 分组卡片照 macOS 的 `--p-group-*`：10 DIP 圆角，外围一圈 1 DIP 描边（浅色 rgba(0,0,0,.06)、深色 rgba(255,255,255,.07)，即 `palette_.stroke`）。
  Border make_card() {
    Border card;
    card.Background(brush(palette_.card));
    card.BorderBrush(brush(palette_.stroke));
    card.BorderThickness(Thickness{1, 1, 1, 1});
    card.CornerRadius(CornerRadius{10, 10, 10, 10});
    return card;
  }

  // 放进分组卡片里的子卡片（说明文字、表单、列表）：上下各留 10 DIP，与 macOS 组内非行内容的 `groupBlock` 内边距相同，排在组首或组末时不和外层卡片的边框贴在一起。
  Border make_inner_card() {
    auto card = make_card();
    card.Margin(Thickness{0, 10, 0, 10});
    return card;
  }

  // 卡片行面板左右各留的宽度，也是每行左右的内边距，与 macOS 的 `rowPad`（10px 16px）相同。
  static constexpr double row_inset = 16;

  // 组内一行的外框：左右用负边距伸到卡片边缘，不是第一项时顶上一条贯穿整行的 1 DIP 分隔线，与 macOS 的 `groupRows` 相同。
  Border append_row_frame(StackPanel const &group, UIElement const &child) {
    Border frame;
    frame.Margin(Thickness{-row_inset, 0, -row_inset, 0});
    if (group.Children().Size() > 0) {
      frame.BorderBrush(brush(palette_.divider));
      frame.BorderThickness(Thickness{0, 1, 0, 0});
    }
    frame.Child(child);
    group.Children().Append(frame);
    return frame;
  }

  // 整行可点的透明按钮，跳转行和折叠区的标题行共用，对应共享页的 `linkRow` 和 `moreOptionsSummary`：内边距与普通行相同，悬停和按下铺 `palette_.hover`。底色走模板资源，模板的视觉状态才不会把它盖回系统色。
  Button make_row_button(bool described) {
    Button button;
    button.HorizontalAlignment(HorizontalAlignment::Stretch);
    button.HorizontalContentAlignment(HorizontalAlignment::Stretch);
    button.VerticalContentAlignment(VerticalAlignment::Center);
    button.MinHeight(described ? 56 : 44);
    button.Padding(Thickness{row_inset, 10, row_inset, 10});
    button.CornerRadius(CornerRadius{0, 0, 0, 0});
    button.BorderThickness(Thickness{0, 0, 0, 0});
    button.Background(brush(transparent_color()));
    auto resources = button.Resources();
    auto put = [&resources](const wchar_t *key, Color const &color) {
      resources.Insert(box_value(hstring(key)), brush(color));
    };
    put(L"ButtonBackground", transparent_color());
    put(L"ButtonBackgroundPointerOver", palette_.hover);
    put(L"ButtonBackgroundPressed", palette_.hover);
    put(L"ButtonBackgroundDisabled", transparent_color());
    put(L"ButtonForeground", palette_.text);
    put(L"ButtonForegroundPointerOver", palette_.text);
    put(L"ButtonForegroundPressed", palette_.text);
    return button;
  }

  // 跳转行，对应共享页的 LinkRow：没有单独的按钮，整行就是按钮，行尾是 12 DIP 的灰色「↗」（`external`，打开网页、文件这类窗口外的东西）或「›」（本窗口的另一页）。`value` 是 LinkRow 的 value：箭头左边 6 DIP 处一段同样 12 DIP 灰字的状态（语音页「服务配置」的「已填写」）。按钮以标题命名、以状态和说明描述，箭头不进无障碍树。
  Button link_row(StackPanel const &group, std::wstring const &title,
                  std::wstring const &subtitle, bool external,
                  std::function<void()> click, std::wstring const &value = L"") {
    record(title, subtitle);
    Grid content;
    content.ColumnSpacing(12);
    ColumnDefinition text_column;
    text_column.Width(GridLength{1, GridUnitType::Star});
    ColumnDefinition arrow_column;
    arrow_column.Width(GridLength{1, GridUnitType::Auto});
    content.ColumnDefinitions().Append(text_column);
    content.ColumnDefinitions().Append(arrow_column);
    // 说明收进标题旁悬停出 ToolTip 的「?」（info_tip 的 passive）：整行就是按钮，里面不再嵌按钮，读屏照旧从行按钮的 HelpText 读到说明。
    StackPanel texts;
    texts.Orientation(Orientation::Horizontal);
    texts.Spacing(4);
    texts.VerticalAlignment(VerticalAlignment::Center);
    auto name = make_text(title, 14, palette_.text);
    name.VerticalAlignment(VerticalAlignment::Center);
    texts.Children().Append(name);
    if (!subtitle.empty())
      texts.Children().Append(info_tip(title, subtitle, true));
    content.Children().Append(texts);
    auto arrow = make_text(external ? L"\u2197" : L"\u203A", 12, palette_.sub);
    arrow.VerticalAlignment(VerticalAlignment::Center);
    A11y::SetAccessibilityView(arrow, Microsoft::UI::Xaml::Automation::Peers::AccessibilityView::Raw);
    StackPanel trailing;
    trailing.Orientation(Orientation::Horizontal);
    trailing.Spacing(6);
    trailing.VerticalAlignment(VerticalAlignment::Center);
    if (!value.empty()) {
      auto state = make_text(value, 12, palette_.sub);
      state.TextWrapping(TextWrapping::NoWrap);
      state.VerticalAlignment(VerticalAlignment::Center);
      trailing.Children().Append(state);
    }
    trailing.Children().Append(arrow);
    Grid::SetColumn(trailing, 1);
    content.Children().Append(trailing);

    auto button = make_row_button(false);
    button.Content(content);
    A11y::SetName(button, hstring(title));
    // 与 LinkRow 的 aria-describedby 同序：先状态，再说明。
    std::wstring help = value;
    if (!subtitle.empty())
      help += (help.empty() ? L"" : L"，") + subtitle;
    if (!help.empty())
      A11y::SetHelpText(button, hstring(help));
    button.Click([on_click = std::move(click)](Inspectable const &, RoutedEventArgs const &) {
      on_click();
    });
    append_row_frame(group, button);
    mark_search_target(title, button);
    return button;
  }

  // 组里的折叠区，对应共享页的 MoreOptions：标题行与普通行同高、没有边框，悬停铺中性底色，标题右边一个灰色「›」，展开时转 90°（关掉系统动画效果时直接转过去）；展开后的各行和组内其他行一样带分隔线，第一行上面那条由折叠体的上边框画出。名字默认是「更多选项」，只有里面是同一类具体东西时才写它是什么（「自定义配色」「更多音效」）。`open` 是画出来时展开与否，点按时经 `remember` 交给调用方记在自己的成员里，跨页面重建保留。标题行是普通按钮，展开与否用 ItemStatus（「已展开」「已收起」）告诉读屏。`search_title` 非空时，搜索落到这个标题的结果把焦点交给标题按钮（「自定义主题」组只有这一个折叠区）。返回放行的面板。
  StackPanel add_fold(StackPanel const &group, std::wstring const &title, bool open,
                      std::function<void(bool)> remember,
                      std::wstring const &automation_id = L"",
                      std::wstring const &search_title = L"") {
    StackPanel fold;
    fold.Margin(Thickness{-row_inset, 0, -row_inset, 0});

    StackPanel label;
    label.Orientation(Orientation::Horizontal);
    label.Spacing(12);
    auto heading = make_text(title, 14, palette_.text);
    heading.VerticalAlignment(VerticalAlignment::Center);
    label.Children().Append(heading);
    auto marker = make_text(L"\u203A", 12, palette_.sub);
    marker.VerticalAlignment(VerticalAlignment::Center);
    marker.RenderTransformOrigin(Windows::Foundation::Point{0.5f, 0.5f});
    RotateTransform turn;
    turn.Angle(open ? 90 : 0);
    marker.RenderTransform(turn);
    A11y::SetAccessibilityView(marker, Microsoft::UI::Xaml::Automation::Peers::AccessibilityView::Raw);
    label.Children().Append(marker);

    auto header = make_row_button(false);
    header.Content(label);
    A11y::SetName(header, hstring(title));
    A11y::SetItemStatus(header, open ? L"已展开" : L"已收起");
    if (!automation_id.empty())
      A11y::SetAutomationId(header, hstring(automation_id));
    if (!search_title.empty())
      mark_search_target(search_title, header);
    Border head;
    if (group.Children().Size() > 0) {
      head.BorderBrush(brush(palette_.divider));
      head.BorderThickness(Thickness{0, 1, 0, 0});
    }
    head.Child(header);
    fold.Children().Append(head);

    StackPanel rows;
    Border body;
    body.BorderBrush(brush(palette_.divider));
    body.BorderThickness(Thickness{0, 1, 0, 0});
    body.Padding(Thickness{row_inset, 0, row_inset, 0});
    body.Visibility(open ? Visibility::Visible : Visibility::Collapsed);
    body.Child(rows);
    fold.Children().Append(body);

    // 折叠体和「›」的旋转按值捕获：CI 上对折叠体取的弱引用在点按时解析为空（元素明明还在树上），点了没有反应。两者都不引用标题按钮，按值持有不成环，折叠区整体丢弃时一起释放。
    header.Click([this, body, turn, on_toggle = std::move(remember)](Inspectable const &sender,
                                                                     RoutedEventArgs const &) {
      const bool expanded = body.Visibility() == Visibility::Collapsed;
      body.Visibility(expanded ? Visibility::Visible : Visibility::Collapsed);
      A11y::SetItemStatus(sender.as<Button>(), expanded ? L"已展开" : L"已收起");
      turn_fold_marker(turn, expanded);
      on_toggle(expanded);
    });
    group.Children().Append(fold);
    return rows;
  }

  // 折叠区的「›」转到展开（90°）或收起（0°），与共享页 `moreOptionsMarker` 的 transition 一样用 150 ms；系统关掉了动画效果时直接转过去，对应 `motion-reduce:transition-none`。
  void turn_fold_marker(RotateTransform const &turn, bool expanded) {
    const double angle = expanded ? 90 : 0;
    if (!ui_settings_.AnimationsEnabled()) {
      turn.Angle(angle);
      return;
    }
    Media::Animation::DoubleAnimation spin;
    spin.To(angle);
    spin.Duration(DurationHelper::FromTimeSpan(std::chrono::milliseconds(150)));
    Media::Animation::Storyboard::SetTarget(spin, turn);
    Media::Animation::Storyboard::SetTargetProperty(spin, L"Angle");
    Media::Animation::Storyboard board;
    board.Children().Append(spin);
    board.Begin();
  }

  // 组末的「更多选项」折叠区：展开与否记在 `open` 这个成员里，跨页面重建保留；`focused` 为真（搜索落到折叠区里的某一行）时直接展开。返回放行的面板。
  StackPanel add_more_options(StackPanel const &group, bool &open, bool focused) {
    // `open` 是窗口自己的成员，与这些控件同寿。
    bool *const state = &open;
    return add_fold(group, L"更多选项", open || focused, [state](bool expanded) { *state = expanded; });
  }

  TextBlock group_note(std::wstring_view value) {
    auto note = make_text(value, 12, palette_.sub);
    // 上下 10 与 macOS `groupNote` 的行内边距相同；左右由卡片行面板的 16 DIP 缩进给出。
    note.Margin(Thickness{0, 10, 0, 10});
    return note;
  }

  // 行内不重复画图标；标签和控件分别贴住卡片两边，窄窗口将控件放在下一行。尺寸照 macOS 的 `group` token：左右 16、上下 10 的内边距，行高至少 44；标题下面画出说明的行（没有控件的纯文字行、经 inline_subtitle 留下状态或后果的行）至少 56。标题 14 DIP、说明 12 DIP。
  Border add_row(StackPanel const &group, wchar_t,
                 std::wstring const &title, std::wstring const &subtitle,
                 FrameworkElement const &control,
                 FrameworkElement const &below = nullptr) {
    record(title, subtitle);
    // 有控件的行，说明是解释，收进标题旁的「?」（info_tip）；没有控件的纯文字行（隐私页「发送哪些内容」这类），说明就是这一行的内容，照旧画在标题下面。说明其实是当前状态或按钮后果的行，由调用处用 inline_subtitle 画回原处。
    const bool tip = control && !subtitle.empty();
    const bool shown = !control && !subtitle.empty();
    Grid header;
    // 行的上下内边距各 10，标题区至少 36（或 24），合起来就是 56（或 44）。
    header.MinHeight(shown ? 36 : 24);
    header.ColumnSpacing(12);
    ColumnDefinition text_column;
    text_column.Width(GridLength{1, GridUnitType::Star});
    ColumnDefinition control_column;
    control_column.Width(GridLength{1, GridUnitType::Auto});
    header.ColumnDefinitions().Append(text_column);
    header.ColumnDefinitions().Append(control_column);
    RowDefinition label_row;
    label_row.Height(GridLength{1, GridUnitType::Auto});
    RowDefinition field_row;
    field_row.Height(GridLength{1, GridUnitType::Auto});
    header.RowDefinitions().Append(label_row);
    header.RowDefinitions().Append(field_row);

    StackPanel texts;
    texts.Spacing(2);
    texts.VerticalAlignment(VerticalAlignment::Center);
    // 第一项是标题；带「?」时是标题和「?」排成的一行（横排面板），row_subtitle 按这个结构找到「?」和第二项的说明。
    auto name = make_text(title, 14, palette_.text);
    if (tip) {
      StackPanel heading;
      heading.Orientation(Orientation::Horizontal);
      heading.Spacing(4);
      name.VerticalAlignment(VerticalAlignment::Center);
      heading.Children().Append(name);
      heading.Children().Append(info_tip(title, subtitle));
      texts.Children().Append(heading);
    } else {
      texts.Children().Append(name);
    }
    if (shown)
      texts.Children().Append(make_text(subtitle, 12, palette_.sub));
    header.Children().Append(texts);
    if (control) {
      control.VerticalAlignment(VerticalAlignment::Center);
      control.HorizontalAlignment(HorizontalAlignment::Right);
      Grid::SetColumn(control, 1);
      header.Children().Append(control);
      // 用控件自身的期望宽度判断，长按钮、字体组合框和滑块都不挤掉标签。
      header.SizeChanged([weak_control = make_weak(control), weak_texts = make_weak(texts),
                          weak_name = make_weak(name), tip](Inspectable const &sender,
                                                            SizeChangedEventArgs const &) {
        const auto row = sender.as<Grid>();
        const auto field = weak_control.get();
        const auto labels = weak_texts.get();
        if (!field || !labels)
          return;
        const bool stacked = row.ActualWidth() < field.DesiredSize().Width + 192;
        // 横排面板不限制宽度，标题不会自己换行：按标签列的宽度减去「?」（约 24 DIP）给标题定最大宽度，长标题照旧折行。
        if (tip)
          if (auto title_block = weak_name.get())
            title_block.MaxWidth(std::max(
                48.0, (stacked ? row.ActualWidth() : row.ActualWidth() - field.DesiredSize().Width - 12) - 24));
        Grid::SetColumnSpan(labels, stacked ? 2 : 1);
        Grid::SetColumn(field, stacked ? 0 : 1);
        Grid::SetRow(field, stacked ? 1 : 0);
        Grid::SetColumnSpan(field, stacked ? 2 : 1);
        field.Margin(Thickness{0, stacked ? 6.0 : 0.0, 0, 0});
      });
    } else {
      Grid::SetColumnSpan(texts, 2);
    }

    StackPanel body;
    body.Children().Append(header);
    if (below)
      body.Children().Append(below);
    auto row = append_row_frame(group, body);
    row.Padding(Thickness{row_inset, 10, row_inset, 10});
    mark_search_target(title, row);
    return row;
  }

  // add_row 画出的卡片标题下的那行说明；没有说明时补一个收起的，供就地更新。
  TextBlock row_subtitle(Border const &card) {
    auto texts = card.Child()
                     .as<StackPanel>()
                     .Children()
                     .GetAt(0)
                     .as<Grid>()
                     .Children()
                     .GetAt(0)
                     .as<StackPanel>();
    if (texts.Children().Size() < 2) {
      // 要就地改写说明的行，说明其实是随数值变化的状态（圆角、不透明度的读数、接口地址的校验）：add_row 收进「?」的说明拿出来，画回标题下面。
      std::wstring moved;
      if (auto heading = texts.Children().GetAt(0).try_as<StackPanel>(); heading && heading.Children().Size() > 1) {
        moved = info_tip_text(heading.Children().GetAt(1).as<FrameworkElement>());
        const auto name = heading.Children().GetAt(0);
        heading.Children().Clear();
        texts.Children().SetAt(0, name);
        texts.Children().GetAt(0).as<FrameworkElement>().MaxWidth(std::numeric_limits<double>::infinity());
      }
      auto line = make_text(moved, moved.empty() ? 11 : 12, palette_.sub);
      line.Visibility(moved.empty() ? Visibility::Collapsed : Visibility::Visible);
      texts.Children().Append(line);
    }
    return texts.Children().GetAt(1).as<TextBlock>();
  }

  // 说明是当前状态（版本号、安装进度、设备详情）或紧挨着按钮讲后果（注销、清除、删除）的行：不收进「?」，照旧画在标题下面。返回同一行，方便接在 add_row 外面。
  Border inline_subtitle(Border const &row) {
    row_subtitle(row);
    return row;
  }

  static void set_subtitle(TextBlock const &line, std::wstring const &value) {
    if (std::wstring_view(line.Text()) != value)
      line.Text(hstring(value));
    line.Visibility(value.empty() ? Visibility::Collapsed : Visibility::Visible);
  }

  // ---- Controls ----

  TextBlock make_text(std::wstring_view value, double size, Color const &color) {
    TextBlock block;
    block.Text(hstring(value));
    block.FontSize(size);
    block.Foreground(brush(color));
    block.TextWrapping(TextWrapping::Wrap);
    return block;
  }

  // 侧栏的产品图标（`SettingsNavIcons.h`）。路径坐标在 24 × 24 的方框里，按 `size` 缩放；图标本身定成 size × size，保留图源四周的留白，导航项模板里装图标的 Viewbox 因此不再按路径的外框放大。不设前景，由所在的模板或父元素决定。
  static PathIcon make_nav_icon(std::string_view page, double size) {
    const std::wstring markup =
        L"<PathIcon xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" Data=\"" +
        std::wstring(nav::nav_icon_path(page)) + L"\"/>";
    auto icon = Markup::XamlReader::Load(hstring(markup)).as<PathIcon>();
    ScaleTransform scale;
    scale.ScaleX(size / nav::nav_icon_box);
    scale.ScaleY(size / nav::nav_icon_box);
    icon.Data().Transform(scale);
    icon.Width(size);
    icon.Height(size);
    return icon;
  }

  FontIcon make_icon(wchar_t glyph, double size, Color const &color) {
    FontIcon icon;
    icon.FontFamily(Media::FontFamily(L"Segoe Fluent Icons,Segoe MDL2 Assets"));
    icon.Glyph(glyph_text(glyph));
    icon.FontSize(size);
    icon.Foreground(brush(color));
    return icon;
  }

  // 开关、下拉框和分段控件写的是偏好，偏好没读到时不可用；`document` 为假的那些（打字统计页的开关、保留期和标签）不碰偏好，照常可用。
  FrameworkElement toggle_control(std::wstring const &name, bool on,
                                  std::function<void(bool)> apply,
                                  bool enabled = true, bool document = true) {
    ToggleSwitch toggle;
    toggle.OnContent(box_value(L""));
    toggle.OffContent(box_value(L""));
    toggle.MinWidth(0);
    toggle.IsOn(on);
    toggle.IsEnabled(enabled && (loaded_ || !document));
    A11y::SetName(toggle, hstring(name));
    toggle.Toggled([on_toggle = std::move(apply)](
                       Inspectable const &sender, RoutedEventArgs const &) {
      on_toggle(sender.as<ToggleSwitch>().IsOn());
    });
    return toggle;
  }

  FrameworkElement select_control(std::wstring const &name,
                                  std::vector<Option> options,
                                  std::wstring const &current,
                                  std::function<void(std::wstring const &)> apply,
                                  bool enabled = true, bool document = true) {
    ComboBox combo;
    combo.Width(168);
    combo.MinHeight(28);
    combo.FontSize(13);
    int32_t selected = -1;
    for (size_t i = 0; i < options.size(); ++i) {
      if (options[i].enabled) {
        combo.Items().Append(box_value(hstring(options[i].label)));
      } else {
        ComboBoxItem item;
        item.Content(box_value(hstring(options[i].label)));
        item.IsEnabled(false);
        combo.Items().Append(item);
      }
      if (options[i].value == current)
        selected = static_cast<int32_t>(i);
    }
    combo.SelectedIndex(selected);
    combo.IsEnabled(enabled && (loaded_ || !document));
    A11y::SetName(combo, hstring(name));
    combo.SelectionChanged(
        [choices = std::move(options), on_select = std::move(apply)](
            Inspectable const &sender, SelectionChangedEventArgs const &) {
          const auto index = sender.as<ComboBox>().SelectedIndex();
          if (index < 0 || static_cast<size_t>(index) >= choices.size())
            return;
          on_select(choices[static_cast<size_t>(index)].value);
        });
    return combo;
  }

  // 分段控件用浮起的选中底；默认超过三项用下拉框，紧凑页内标签可显式允许四项。`full_width` 是页内顶部标签的样式（macOS 的 `candidateTabs`，见 `page_tab_bar`）：铺满正文宽度、各段等宽，段高 36、14 DIP 字，选中与否都用正文色，外框圆角 6、段圆角 5；这种样式不按 `maximum_segments` 退成下拉框。
  FrameworkElement segmented_control(std::wstring const &name,
                                     std::vector<Option> const &options,
                                     std::wstring const &current,
                                     std::function<void(std::wstring const &)> apply,
                                     bool enabled = true, bool document = true,
                                     size_t maximum_segments = 3,
                                     std::wstring const &automation_prefix = L"",
                                     bool full_width = false) {
    if (!full_width && options.size() > maximum_segments)
      return select_control(name, options, current, std::move(apply), enabled, document);
    Border well;
    if (full_width) {
      // 标签条的底照 macOS `candidateTabs` 的 `--p-seg-bg`、`--p-seg-border`：浅色 rgba(0,0,0,.055)、深色 rgba(255,255,255,.08)，没有描边。
      well.Background(brush(palette_.dark ? rgba(255, 255, 255, .08) : rgba(0, 0, 0, .055)));
    } else {
      well.Background(brush(palette_.seg_well));
      well.BorderBrush(brush(palette_.stroke));
      well.BorderThickness(Thickness{1, 1, 1, 1});
    }
    well.CornerRadius(full_width ? CornerRadius{6, 6, 6, 6} : CornerRadius{5, 5, 5, 5});
    well.Padding(Thickness{2, 2, 2, 2});
    // 紧凑样式横排、按内容定宽；整宽样式每段一个等宽的 * 列。
    StackPanel items;
    items.Orientation(Orientation::Horizontal);
    items.Spacing(2);
    Grid columns;
    columns.ColumnSpacing(2);
    if (full_width)
      well.HorizontalAlignment(HorizontalAlignment::Stretch);
    int32_t column = 0;
    for (const auto &option : options) {
      const bool on = option.value == current;
      Button button;
      // 整宽样式的段至少 36 高；最窄的窗口里放不下一行字时（「输入」页有六个标签）字折成两行，同一排的段一起变高，与共享页按钮里的字折行相同。
      if (full_width) {
        button.MinHeight(36);
        button.VerticalAlignment(VerticalAlignment::Stretch);
      } else {
        button.Height(26);
      }
      button.MinWidth(0);
      button.Padding(full_width ? Thickness{8, 0, 8, 0} : Thickness{10, 0, 10, 0});
      button.CornerRadius(full_width ? CornerRadius{5, 5, 5, 5} : CornerRadius{4, 4, 4, 4});
      button.BorderThickness(Thickness{1, 1, 1, 1});
      button.Background(brush(on ? (palette_.dark ? rgba(0x5A, 0x5A, 0x5E, 1) : rgba(255, 255, 255, 1)) : transparent_color()));
      button.BorderBrush(brush(on ? palette_.stroke : transparent_color()));
      button.HorizontalContentAlignment(HorizontalAlignment::Stretch);
      button.VerticalContentAlignment(VerticalAlignment::Stretch);
      Grid content;
      auto label = make_text(option.label, full_width ? 14 : 13,
                             (on || full_width) ? palette_.text : palette_.sub);
      label.TextWrapping(full_width ? TextWrapping::Wrap : TextWrapping::NoWrap);
      if (full_width)
        label.TextAlignment(TextAlignment::Center);
      label.HorizontalAlignment(HorizontalAlignment::Center);
      label.VerticalAlignment(VerticalAlignment::Center);
      content.Children().Append(label);
      button.Content(content);
      A11y::SetName(button, hstring(name + L" " + option.label));
      if (!automation_prefix.empty())
        A11y::SetAutomationId(button, hstring(automation_prefix + L"-" + option.value));
      if (on)
        A11y::SetItemStatus(button, L"已选择");
      button.IsEnabled(enabled && (loaded_ || !document) && option.enabled);
      button.Click([apply, value = option.value, on](Inspectable const &,
                                                     RoutedEventArgs const &) {
        if (!on)
          apply(value);
      });
      if (full_width) {
        ColumnDefinition segment;
        segment.Width(GridLength{1, GridUnitType::Star});
        columns.ColumnDefinitions().Append(segment);
        button.HorizontalAlignment(HorizontalAlignment::Stretch);
        Grid::SetColumn(button, column++);
        columns.Children().Append(button);
      } else {
        items.Children().Append(button);
      }
    }
    A11y::SetName(well, hstring(name));
    if (full_width)
      well.Child(columns);
    else
      well.Child(items);
    return well;
  }

  // ---- 页内标签 ----

  // 页内顶部标签，照共享设置 UI 的 `useSettingsTabs`（桌面用 `candidateTabs` 样式，即 segmented_control 的 `full_width`）：一页的内容分成几块，一次只画选中的那一块。选中哪个标签记在窗口成员 `page_tabs_` 里，按页面 id 存，跨页面保留、不写偏好（与 macOS 的 `useSettingsIntroTabs` 相同）；还没选过、或记下的标签这一页没有时，落到第一个。切标签时整页重画，页首引导随之换成这个标签的说明和插画（`SettingsMascotIntro.h`）。用法是先 `const auto tab = page_tab_bar(page, 名称, 标签, 自动化前缀);`，再把每块内容包在 `if (in_tab(tab, L"<标签 id>"))` 里。标签按钮的自动化 ID 是 `<自动化前缀>-<标签 id>`。建搜索索引时不画标签条、返回空，只记下各标签的名字（`index_tabs_`，结果第二行的位置要写标签名），`in_tab` 对每块都返回真，并把之后 record 的条目记在那个标签下（`tab_index_`）；选中搜索结果时 open_search_result 先把它所在的标签设成当前标签，再画页面，结果因此总在画出来的那块里。
  std::wstring page_tab_bar(StackPanel const &page, std::wstring const &name,
                            std::vector<Option> const &tabs,
                            std::wstring const &automation_prefix) {
    // 建索引时只记下各标签的名字，搜索结果的位置要写标签名。
    if (indexing_) {
      index_tabs_ = tabs;
      return {};
    }
    if (tabs.empty())
      return {};
    auto current = page_tab(current_page_);
    if (std::none_of(tabs.begin(), tabs.end(),
                     [&current](Option const &tab) { return tab.value == current; }))
      current = tabs.front().value;
    auto bar = segmented_control(
        name, tabs, current,
        [this, id = current_page_](std::wstring const &value) {
          page_tabs_[id] = value;
          queue_refresh();
        },
        true, false, tabs.size(), automation_prefix, true);
    // 与页首引导、下面的第一组都相隔 24（页面的 Spacing 4 加这里的 20，组自己的上边距 20 加 Spacing 4）。
    bar.Margin(Thickness{0, 20, 0, 0});
    page.Children().Append(bar);
    return current;
  }

  // 标签 `tab` 那一块现在画不画：平时只画选中的（`current` 是 page_tab_bar 的返回值），建索引时每块都画，并把之后的条目记在这个标签下。
  bool in_tab(std::wstring const &current, std::wstring const &tab) {
    if (indexing_) {
      tab_index_ = tab;
      group_index_.clear();
      return true;
    }
    return current == tab;
  }

  // 某一页记下的标签，没选过时为空。
  std::wstring page_tab(std::string const &page) const {
    const auto found = page_tabs_.find(page);
    return found == page_tabs_.end() ? std::wstring() : found->second;
  }

  FrameworkElement slider_control(std::wstring const &name, double minimum,
                                  double maximum, double value,
                                  std::function<void(double)> apply,
                                  bool enabled = true, double step = 1,
                                  std::wstring const &suffix = L"") {
    return formatted_slider_control(
        name, minimum, maximum, value, std::move(apply),
        [suffix](double shown) { return number_text(shown) + suffix; }, enabled, step);
  }

  // 数值由 format 格式化；两列布局给滑块与数值各自留出空间。
  Grid formatted_slider_control(std::wstring const &name, double minimum,
                                            double maximum, double value,
                                            std::function<void(double)> apply,
                                            std::function<std::wstring(double)> format,
                                            bool enabled = true, double step = 1) {
    Grid box;
    box.Width(218);
    box.ColumnSpacing(8);
    ColumnDefinition track_column;
    track_column.Width(GridLength{1, GridUnitType::Star});
    ColumnDefinition value_column;
    value_column.Width(GridLength{1, GridUnitType::Auto});
    box.ColumnDefinitions().Append(track_column);
    box.ColumnDefinitions().Append(value_column);
    Slider slider;
    slider.MinWidth(100);
    slider.Minimum(minimum);
    slider.Maximum(maximum);
    slider.StepFrequency(step);
    slider.SmallChange(step);
    slider.LargeChange(step);
    slider.Value(std::clamp(value, minimum, maximum));
    slider.VerticalAlignment(VerticalAlignment::Center);
    slider.IsEnabled(enabled && loaded_);
    A11y::SetName(slider, hstring(name));
    auto label = make_text(format(std::clamp(value, minimum, maximum)), 12,
                           palette_.text);
    label.MinWidth(48);
    label.TextWrapping(TextWrapping::NoWrap);
    label.TextAlignment(TextAlignment::Right);
    label.VerticalAlignment(VerticalAlignment::Center);
    Grid::SetColumn(label, 1);
    slider.ValueChanged(
        [on_slide = std::move(apply), step,
         format](Inspectable const &sender,
                 Controls::Primitives::RangeBaseValueChangedEventArgs const &args) {
          const double rounded = std::round(args.NewValue() / step) * step;
          // XAML 视觉树保留控件后，建行时的弱引用仍可能失效；从当前滑块取同一容器里的可见数值。
          if (auto parent = sender.as<Slider>().Parent().try_as<Grid>()) {
            auto children = parent.Children();
            if (children.Size() == 2) {
              if (auto block = children.GetAt(1).try_as<TextBlock>())
                block.Text(hstring(format(rounded)));
            }
          }
          on_slide(rounded);
        });
    box.Children().Append(slider);
    box.Children().Append(label);
    return box;
  }

  FrameworkElement button_control(std::wstring const &label,
                                  std::function<void()> click,
                                  bool enabled = true) {
    Button button;
    button.Content(box_value(hstring(label)));
    button.Height(32);
    button.Padding(Thickness{16, 0, 16, 0});
    button.IsEnabled(enabled);
    button.Click([on_click = std::move(click)](Inspectable const &,
                                               RoutedEventArgs const &) { on_click(); });
    return button;
  }

  // 按可用宽度等分列，重排已有子项，保留按钮与复选框的焦点和选择状态。`natural_columns` 时列宽取本列最宽子项的宽度（Auto 列）、从左排起，不铺满整行，`minimum_width` 只用来估算一行放几列。
  Grid responsive_grid(double minimum_width, double gap, int maximum_columns = 0,
                       bool natural_columns = false) {
    Grid grid;
    grid.ColumnSpacing(gap);
    grid.RowSpacing(gap);
    grid.SizeChanged([minimum_width, gap, maximum_columns, natural_columns](
                         Inspectable const &sender, SizeChangedEventArgs const &) {
      const auto panel = sender.as<Grid>();
      auto columns = std::max(
          1, static_cast<int>((panel.ActualWidth() + gap) / (minimum_width + gap)));
      if (maximum_columns > 0)
        columns = std::min(columns, maximum_columns);
      if (static_cast<int>(panel.ColumnDefinitions().Size()) == columns)
        return;
      panel.ColumnDefinitions().Clear();
      panel.RowDefinitions().Clear();
      for (int column = 0; column < columns; ++column) {
        ColumnDefinition definition;
        definition.Width(GridLength{1, natural_columns ? GridUnitType::Auto : GridUnitType::Star});
        panel.ColumnDefinitions().Append(definition);
      }
      const auto count = static_cast<int>(panel.Children().Size());
      for (int index = 0; index < count; ++index) {
        if (index % columns == 0) {
          RowDefinition definition;
          definition.Height(GridLength{1, GridUnitType::Auto});
          panel.RowDefinitions().Append(definition);
        }
        const auto child = panel.Children()
                               .GetAt(static_cast<uint32_t>(index))
                               .as<FrameworkElement>();
        Grid::SetRow(child, index / columns);
        Grid::SetColumn(child, index % columns);
      }
    });
    return grid;
  }

  // 一组复选框，按 `item_width` 排成等宽的列；`maximum_columns` 为 1 时排成纵向一列（悬浮工具栏页的「按钮」）。`text_size` 是复选框文字的字号。
  FrameworkElement checks_panel(std::vector<Check> checks,
                                double item_width = 174, int maximum_columns = 0,
                                double text_size = 12) {
    auto grid = responsive_grid(item_width, 8, maximum_columns);
    grid.Margin(Thickness{0, 4, 0, 8});
    for (auto &check : checks) {
      record(check.label, L"");
      CheckBox box;
      box.Content(make_text(check.label, text_size, palette_.text));
      box.IsChecked(check.on);
      box.MinWidth(0);
      box.MinHeight(28);
      box.HorizontalContentAlignment(HorizontalAlignment::Stretch);
      box.HorizontalAlignment(HorizontalAlignment::Stretch);
      A11y::SetName(box, hstring(check.label));
      box.IsEnabled(check.enabled && loaded_);
      box.Click([apply = std::move(check.apply)](Inspectable const &sender,
                                                 RoutedEventArgs const &) {
        const auto state = sender.as<CheckBox>().IsChecked();
        apply(state && state.Value());
      });
      mark_search_target(check.label, box);
      grid.Children().Append(box);
    }
    return grid;
  }

  // ---- Preference rows ----

  void bool_row(StackPanel const &group, wchar_t glyph, std::wstring const &title,
                std::wstring const &subtitle, std::wstring key, bool fallback,
                bool redraw = false, bool enabled = true) {
    const bool on = document_.Boolean(key, fallback);
    add_row(group, glyph, title, subtitle,
            toggle_control(title, on,
                           [this, key, redraw](bool value) {
                             change([&](PreferencesDocument &doc) {
                               doc.SetBoolean(key, value);
                             }, redraw);
                           },
                           enabled));
  }

  void select_row(StackPanel const &group, wchar_t glyph,
                  std::wstring const &title, std::wstring const &subtitle,
                  std::wstring key, std::vector<Option> options,
                  std::wstring const &fallback, bool redraw = false,
                  bool enabled = true) {
    const auto current = document_.String(key, fallback);
    add_row(group, glyph, title, subtitle,
            select_control(title, std::move(options), current,
                           [this, key, redraw](std::wstring const &value) {
                             change([&](PreferencesDocument &doc) {
                               doc.SetString(key, value);
                             }, redraw);
                           },
                           enabled));
  }

  void segment_row(StackPanel const &group, wchar_t glyph,
                   std::wstring const &title, std::wstring const &subtitle,
                   std::wstring key, std::vector<Option> const &options,
                   std::wstring const &fallback) {
    const auto current = document_.String(key, fallback);
    add_row(group, glyph, title, subtitle,
            segmented_control(title, options, current,
                              [this, key](std::wstring const &value) {
                                change([&](PreferencesDocument &doc) {
                                  doc.SetString(key, value);
                                }, true);
                              }));
  }

  // A numeric preference offered as a list; a stored value outside the list is kept as its own entry rather than silently shown as another.
  void number_select_row(StackPanel const &group, wchar_t glyph,
                         std::wstring const &title, std::wstring const &subtitle,
                         std::wstring key, std::vector<int> values,
                         double fallback, std::wstring const &suffix,
                         bool enabled = true) {
    const auto current = static_cast<int>(std::lround(document_.Number(key, fallback)));
    if (std::find(values.begin(), values.end(), current) == values.end()) {
      values.push_back(current);
      std::sort(values.begin(), values.end());
    }
    std::vector<Option> options;
    options.reserve(values.size());
    for (const int value : values)
      options.push_back({std::to_wstring(value), std::to_wstring(value) + suffix});
    add_row(group, glyph, title, subtitle,
            select_control(title, std::move(options), std::to_wstring(current),
                           [this, key](std::wstring const &value) {
                             change([&](PreferencesDocument &doc) {
                               doc.SetNumber(key, std::stod(value));
                             }, false);
                           }, enabled));
  }

  void slider_row(StackPanel const &group, wchar_t glyph,
                  std::wstring const &title, std::wstring const &subtitle,
                  std::wstring key, double minimum, double maximum,
                  double fallback, double step = 1,
                  std::wstring const &suffix = L"") {
    add_row(group, glyph, title, subtitle,
            slider_control(title, minimum, maximum,
                           document_.Number(key, fallback),
                           [this, key](double value) {
                             document_.SetNumber(key, value);
                             schedule_save();
                             update_preview();
                           },
                           true, step, suffix));
  }

  // A surface of the shared desktop app this window does not draw itself.

  // 打开网页的跳转行，行尾是「↗」。
  void url_row(StackPanel const &group, std::wstring const &title,
               std::wstring const &subtitle, const wchar_t *url) {
    link_row(group, title, subtitle, true, [this, url] { open_url(url); });
  }

  // ---- Launching ----

  // 本程序自己的路径。面板宿主（MSIME.exe 或 msime-client-panels.exe）和随包资源都装在它旁边。

  // The shared app must read the same store as this window; the context is passed only when both paths are known.

  // Starting the shared app waits for it to become idle, so it runs off the UI thread; only a failure comes back.

  void open_url(const wchar_t *url) {
    const auto result = reinterpret_cast<INT_PTR>(
        ShellExecuteW(nullptr, L"open", url, nullptr, nullptr, SW_SHOWNORMAL));
    if (result <= 32)
      show_notice(std::wstring(L"无法打开浏览器。地址：") + url);
  }

  void open_folder(std::filesystem::path const &folder) {
    std::error_code error;
    if (folder.empty() || !std::filesystem::is_directory(folder, error)) {
      show_notice(L"数据目录还不存在。请先完成输入法安装。");
      return;
    }
    const auto result = reinterpret_cast<INT_PTR>(ShellExecuteW(
        nullptr, L"open", folder.c_str(), nullptr, nullptr, SW_SHOWNORMAL));
    if (result <= 32)
      show_notice(L"无法打开数据目录：" + folder.wstring());
  }

  // 「打开日志目录」：Server 的日志在数据目录下的 logs（server.log，TSF 端日志也汇总进去，见 src/system/DiagnosticLog.h）。还没写过日志、这个目录还不存在时打开数据目录本身，与共享页「还没有写入时打开它所在的目录」一样。打不开时用共享页 Windows 一支的说法。

  // 「立即重启输入法服务」，与共享应用 Windows 上的 `restart_input_method`（`apps/desktop/src-tauri/src/lib.rs`）走同一条路：往本版本 Server 的 Aux 管道（`FANY_IME_AUX_NAMED_PIPE`，带版本后缀）写 UTF-16LE 的 `RestartServer`，不带结尾的 0、不等回答。Server 只收本用户完整桌面进程发来的这条消息（`src/ipc/AuxListener.cpp`），收到后以重启退出码结束，由看门狗重新拉起。管道在短暂重试后仍连不上（Server 没在运行）时报失败。管道的写法复用 TSF 的 `AuxNamedpipeSender.h`；连接最多重试约 100 ms，所以放到工作线程，结果照共享页的 HostActionButton 写在页面顶上。
  fire_and_forget restart_input_method() {
    if (input_method_restarting_)
      co_return;
    input_method_restarting_ = true;
    auto weak = get_weak();
    auto queue = DispatcherQueue();
    co_await resume_background();
    const bool sent = msime::windows::detail::SendToAuxNamedpipe(FANY_IME_AUX_NAMED_PIPE,
                                                                  L"RestartServer", false);
    queue.TryEnqueue([weak, sent] {
      if (auto self = weak.get()) {
        self->input_method_restarting_ = false;
        if (sent)
          self->show_notice(L"已发送重启请求。", InfoBarSeverity::Success);
        else
          self->show_notice(L"重启输入法服务失败，请稍后重试。");
      }
    });
  }

  // 安装器随包装入的第三方组件许可声明，用系统关联的程序（通常是记事本）打开。
  void open_third_party_notices() {
    const auto notices = msime::settings::third_party_notices_path(settings_executable());
    std::error_code error;
    if (notices.empty() || !std::filesystem::is_regular_file(notices, error)) {
      show_notice(L"找不到第三方组件许可声明。请重新安装输入法后再试。");
      return;
    }
    const auto result = reinterpret_cast<INT_PTR>(ShellExecuteW(
        nullptr, L"open", notices.c_str(), nullptr, nullptr, SW_SHOWNORMAL));
    if (result <= 32)
      show_notice(L"无法打开第三方组件许可声明：" + notices.wstring());
  }

  // 「导出设置」「导入设置」的文件对话框：Windows 自己的保存或打开对话框，只列 .json。用户取消或对话框打不开时返回空。
  std::optional<std::filesystem::path> pick_settings_file(bool save) {
    com_ptr<IFileDialog> dialog;
    if (FAILED(CoCreateInstance(save ? CLSID_FileSaveDialog : CLSID_FileOpenDialog,
                                nullptr, CLSCTX_INPROC_SERVER,
                                IID_PPV_ARGS(dialog.put()))))
      return std::nullopt;
    const COMDLG_FILTERSPEC filter{L"设置文件 (*.json)", L"*.json"};
    dialog->SetFileTypes(1, &filter);
    dialog->SetDefaultExtension(L"json");
    dialog->SetTitle(save ? L"导出设置" : L"导入设置");
    dialog->SetOkButtonLabel(save ? L"导出" : L"导入");
    if (save)
      dialog->SetFileName(MSIME_EDITION_DISPLAY_NAME L"设置.json");
    FILEOPENDIALOGOPTIONS options = 0;
    if (SUCCEEDED(dialog->GetOptions(&options)))
      dialog->SetOptions(options | FOS_FORCEFILESYSTEM |
                         (save ? FOS_OVERWRITEPROMPT : FOS_FILEMUSTEXIST));
    // 点击按钮时本窗口就是这个线程的活动窗口，对话框以它为所有者，模态地挡住设置窗口。
    if (dialog->Show(GetActiveWindow()) != S_OK)
      return std::nullopt;
    com_ptr<IShellItem> item;
    if (FAILED(dialog->GetResult(item.put())))
      return std::nullopt;
    PWSTR raw = nullptr;
    if (FAILED(item->GetDisplayName(SIGDN_FILESYSPATH, &raw)) || !raw)
      return std::nullopt;
    std::filesystem::path chosen(raw);
    CoTaskMemFree(raw);
    return chosen;
  }

  // 导出的是已经保存的设置：先把还在等待的滑块修改存下，再由 host-api 读出并换算成设置文件。
  void export_settings() {
    flush_pending();
    const auto directory = state_directory();
    if (directory.empty()) {
      show_notice(L"找不到水杉输入法的数据目录。请先完成输入法安装。");
      return;
    }
    const auto target = pick_settings_file(true);
    if (!target)
      return;
    const auto path = path_utf8(directory);
    auto response = take_response(msime_client_export_settings(
        reinterpret_cast<const uint8_t *>(path.data()), path.size()));
    std::string contents;
    if (response.ok) {
      try {
        contents = utf8(
            JsonObject::Parse(text(response.text)).GetNamedString(L"value"));
      } catch (...) {
        response.ok = false;
      }
    }
    if (!response.ok) {
      show_notice(L"导出设置未能完成：无法读取当前设置。");
      return;
    }
    if (!msime::settings::write_settings_document(*target, contents)) {
      show_notice(L"导出设置未能完成：无法写入所选位置。");
      return;
    }
    show_notice(L"设置已导出到 " + target->wstring() + L"。", InfoBarSeverity::Success);
  }

  // 导入按本窗口读到的修订号比较并交换写回，与每个控件的保存一样；别处刚改过设置时拒绝导入，什么也不写。
  void import_settings() {
    flush_pending();
    const auto directory = state_directory();
    if (directory.empty()) {
      show_notice(L"找不到水杉输入法的数据目录。请先完成输入法安装。");
      return;
    }
    const auto source = pick_settings_file(false);
    if (!source)
      return;
    const auto contents = msime::settings::read_settings_document(*source);
    if (!contents) {
      show_notice(L"无法导入这个文件：读取失败，或者文件太大，不是水杉输入法导出的设置文件。");
      return;
    }
    const auto path = path_utf8(directory);
    auto response = take_response(msime_client_import_settings(
        reinterpret_cast<const uint8_t *>(path.data()), path.size(),
        document_.revision(), reinterpret_cast<const uint8_t *>(contents->data()),
        contents->size()));
    // 成功时控件要显示导入后的设置，修订号冲突时要换成别处刚存下的设置，所以两种情况都重新读取。
    reload_document();
    if (!response.ok) {
      std::string code;
      try {
        code = utf8(JsonObject::Parse(text(response.text)).GetNamedString(L"error", L""));
      } catch (...) {
      }
      show_notice(msime::settings::settings_import_error_message(code));
      return;
    }
    show_notice(L"设置已导入并立即生效。本机的服务配置与密钥、诊断日志、使用统计和剪贴板历史开关保持不变。",
                InfoBarSeverity::Success);
  }

  // 把文字放进剪贴板，成功时返回 true。别的程序正占着剪贴板时 SetContent 会抛 hresult_error，按钮的点击回调里不能让它漏出去；Flush 让内容在本窗口关闭后仍留在剪贴板里（否则 DataPackage 是延迟提供的，进程退出就没了）。
  static bool copy_text(std::wstring const &value) {
    try {
      Windows::ApplicationModel::DataTransfer::DataPackage package;
      package.SetText(hstring(value));
      Windows::ApplicationModel::DataTransfer::Clipboard::SetContent(package);
      Windows::ApplicationModel::DataTransfer::Clipboard::Flush();
      return true;
    } catch (hresult_error const &) {
      return false;
    }
  }

  // ---- Pages ----

  void build_page(std::string_view id, StackPanel const &page) {
    // 上一次建页登记的组头随旧页面一起作废。
    group_headers_.clear();
    const auto *model = nav::find_page(id);
    if (!model)
      return;
    if (id == "candidate")
      build_candidate_page(page);
    else if (id == "toolbar")
      build_toolbar_page(page);
    else if (id == "typing")
      build_typing_page(page);
    else if (id == "shortcuts")
      build_shortcuts_page(page);
    else if (id == "about")
      build_about_page(page);
  }

  // 交给共享应用的设置页（现在没有这样的页）只有一行，在共享应用里打开它。

  // ---- 候选字体 ----

  // 共享设置「候选窗口」页的字体各行，按 Windows 宿主的样子：「字体与大小」组开头是「候选字体」预设（末尾是「自定义…」）和按需出现的主字体，字号之后是英文字体（candidate_english_font_row）。规则在 CandidateFontPresets.h。与共享页一样没有回退字体列表的编辑器：预设和主字体会写它。返回主字体框画没画出来：读取字体列表的状态和「刷新」按钮跟着当前第一个可见的字体框走，主字体框不在时由英文字体框带着。
  bool candidate_font_rows(StackPanel const &group) {
    StackPanel preset_host;
    group.Children().Append(preset_host);
    candidate_font_preset_row(preset_host);
    if (!indexing_) {
      font_preset_host_ = preset_host;
      font_menu_updaters_.clear();
    }

    // 索引时总要画，搜索才找得到「主字体」；搜索落到它上面时也画出来。
    const bool shown =
        indexing_ || focus_row_ == L"主字体" ||
        nav::candidate_main_font_shown(
            candidate_font_custom_,
            document_.String(L"candidate_font_family", nav::candidate_default_font_family),
            document_.Strings(L"candidate_fallback_fonts"));
    if (!shown)
      return false;
    auto [field, status] = font_field_with_catalog(
        font_input(L"主字体",
                   document_.String(L"candidate_font_family",
                                    nav::candidate_default_font_family),
                   [this](std::wstring const &font) {
                     // Windows 的候选窗从回退列表取汉字，主字体同时排到回退列表最前。
                     const auto patch = nav::candidate_main_font_patch(
                         font,
                         document_.String(L"candidate_font_family",
                                          nav::candidate_default_font_family),
                         document_.Strings(L"candidate_fallback_fonts"));
                     document_.SetString(L"candidate_font_family", patch.family);
                     document_.SetOrderedStrings(L"candidate_fallback_fonts", patch.fallback);
                     schedule_save();
                     update_preview();
                     refresh_font_preset_row();
                   }));
    add_row(group, 0xE8C1, L"主字体", L"", field, status);
    return true;
  }

  // 「字体与大小」组里的英文字体。`catalog` 为真（主字体框收起）时由它带着字体列表的读取状态和「刷新」按钮，读取失败后的重试和装了新字体后的刷新仍然找得到。
  void candidate_english_font_row(StackPanel const &group, bool catalog) {
    auto input = font_input(L"英文字体",
                            document_.String(L"candidate_english_font",
                                             nav::candidate_default_english_font),
                            [this](std::wstring const &font) {
                              document_.SetString(L"candidate_english_font", font);
                              schedule_save();
                              update_preview();
                            });
    const std::wstring subtitle =
        L"优先用于候选和预编辑；缺字时使用主字体，不限英文输入模式。保存后自动应用。";
    if (!catalog) {
      add_row(group, 0xE8D3, L"英文字体", subtitle, input);
      return;
    }
    auto [field, status] = font_field_with_catalog(input);
    add_row(group, 0xE8D3, L"英文字体", subtitle, field, status);
  }

  // 字体框和它旁边的「刷新」按钮（读屏名「刷新字体列表」），以及放在这一行下面的读取状态：共享页把原来单独的「系统字体列表」一行并进了第一个字体框。状态和按钮绑到这一行，show_font_catalog_status 在原处更新它们。Windows 只有一份字体目录状态，两个字体框共用，不会读两次。
  std::pair<FrameworkElement, TextBlock> font_field_with_catalog(FrameworkElement const &input) {
    StackPanel field;
    field.Orientation(Orientation::Horizontal);
    field.Spacing(8);
    field.Children().Append(input);
    auto refresh_button = button_control(
        L"刷新", [this] { load_font_catalog(); },
        loaded_ && font_catalog_status_ != FontCatalogStatus::loading);
    refresh_button.VerticalAlignment(VerticalAlignment::Top);
    A11y::SetName(refresh_button, hstring(L"刷新字体列表"));
    field.Children().Append(refresh_button);
    // 状态是读屏会播报的区域。
    auto status = make_text(font_catalog_message(), 12, palette_.sub);
    status.Margin(Thickness{0, 0, 0, 8});
    A11y::SetLiveSetting(status, Automation::Peers::AutomationLiveSetting::Polite);
    status.Visibility(status.Text().empty() ? Visibility::Collapsed : Visibility::Visible);
    if (!indexing_) {
      font_catalog_text_ = status;
      font_catalog_button_ = refresh_button.as<Button>();
    }
    return {field, status};
  }

  // 「候选字体」预设一行。它画在自己的容器里，主字体每输入一个字就在原处重画这一行，不必重建整页而丢掉输入框的焦点。末尾的「自定义…」只亮出主字体框、不改字体（共享页的 onCustomChange）；字体不是任何预设时选中的就是它。六项照共享页的 SegmentedRow 画成分段控件，不按三段退成下拉框；Windows 没有自带圆体，那一段按不下去。
  void candidate_font_preset_row(StackPanel const &host) {
    const auto family =
        document_.String(L"candidate_font_family", nav::candidate_default_font_family);
    const auto preset = nav::current_candidate_font_preset(
        family, document_.Strings(L"candidate_fallback_fonts"));
    std::vector<Option> options;
    for (auto const &entry : nav::candidate_font_presets)
      options.push_back({std::wstring(entry.id), std::wstring(entry.label),
                         nav::candidate_font_preset_available(entry.id)});
    options.push_back({L"custom", L"自定义…", true});
    inline_subtitle(add_row(host, 0xE8D2, L"候选字体",
            preset ? std::wstring(L"Windows 没有自带圆体")
                   : L"当前为自定义字体 " + family + L"，可在下方修改",
            segmented_control(L"候选字体", options,
                              preset && !candidate_font_custom_ ? std::wstring(*preset)
                                                                : std::wstring(L"custom"),
                              [this](std::wstring const &id) {
                                if (id == L"custom") {
                                  candidate_font_custom_ = true;
                                  queue_refresh();
                                  return;
                                }
                                candidate_font_custom_ = false;
                                change([&](PreferencesDocument &doc) {
                                  const auto patch = nav::candidate_font_preset_patch(
                                      id, doc.Strings(L"candidate_fallback_fonts"));
                                  doc.SetString(L"candidate_font_family", patch.family);
                                  doc.SetOrderedStrings(L"candidate_fallback_fonts",
                                                        patch.fallback);
                                }, true);
                              },
                              true, true, options.size())));
  }

  void refresh_font_preset_row() {
    if (!font_preset_host_)
      return;
    font_preset_host_.Children().Clear();
    candidate_font_preset_row(font_preset_host_);
  }

  // 字体名输入框（共享页的 FontFamilyInput）：可以直接输入，也可以从系统字体列表里选。第一次获得焦点时才读取字体列表；还没打字时列出全部字体，打字后只列名字里含有输入内容的，最多 100 项。每输入一个字就写进设置，停下后保存；不合法的名字（空、超过 128 字节或含控制字符）不写入，输入框标红，与共享页不保存不合法字体的做法相同。
  FrameworkElement font_input(std::wstring const &label, std::wstring const &value,
                              std::function<void(std::wstring const &)> apply) {
    StackPanel box;
    box.Spacing(4);
    box.Width(240);
    AutoSuggestBox input;
    input.Text(hstring(value));
    input.UpdateTextOnSelect(false);
    input.IsEnabled(loaded_);
    A11y::SetName(input, hstring(label));
    auto note = make_text(L"", 12, palette_.sub);
    note.Visibility(Visibility::Collapsed);
    // 名字不合法时的说明，用共享页底部那条错误提示的规则部分（「请在候选窗口页修正字体」那半句指的是页面，这里就在输入框下面）；读屏从输入框的帮助文本读到它，对应共享输入框的 aria-invalid。
    const auto danger = palette_.dark ? rgba(0xFF, 0x99, 0xA4, 1) : rgba(0xC4, 0x2B, 0x1C, 1);
    auto invalid = make_text(L"名称不能为空、不能含控制字符或超过 128 个 UTF-8 字节。", 12, danger);
    invalid.TextWrapping(TextWrapping::Wrap);
    invalid.Visibility(Visibility::Collapsed);
    A11y::SetLiveSetting(invalid, Automation::Peers::AutomationLiveSetting::Assertive);
    box.Children().Append(input);
    box.Children().Append(invalid);
    box.Children().Append(note);

    // 用户打过字以后才按输入内容筛选，选中一项后又列出全部，与共享页的 search 状态相同。
    auto search = std::make_shared<bool>(false);
    auto focused = std::make_shared<bool>(false);
    std::function<void()> update_menu = [this, weak_input = make_weak(input),
                                         weak_note = make_weak(note), search, focused] {
      auto field = weak_input.get();
      auto hint = weak_note.get();
      if (!field || !hint || !*focused)
        return;
      const auto menu =
          nav::font_menu(font_catalog_, std::wstring(field.Text().c_str()), *search,
                         font_name_lower);
      std::vector<Inspectable> items;
      items.reserve(menu.options.size());
      for (auto const &font : menu.options)
        items.push_back(box_value(hstring(font)));
      field.ItemsSource(single_threaded_vector<Inspectable>(std::move(items)));
      std::wstring message;
      if (font_catalog_status_ == FontCatalogStatus::ready && menu.options.empty())
        message = L"没有匹配的字体，可继续手动输入。";
      else if (menu.matches > nav::font_menu_limit)
        message = L"显示前 100 项，请输入名称缩小范围。";
      hint.Text(hstring(message));
      hint.Visibility(message.empty() ? Visibility::Collapsed : Visibility::Visible);
      field.IsSuggestionListOpen(!menu.options.empty());
    };
    if (!indexing_)
      font_menu_updaters_.push_back(update_menu);

    auto write = [this, apply = std::move(apply), weak_input = make_weak(input),
                  weak_invalid = make_weak(invalid), danger](std::wstring const &font) {
      auto field = weak_input.get();
      auto message = weak_invalid.get();
      if (!field || !message)
        return;
      const bool valid = nav::valid_font_family(font);
      if (valid) {
        field.ClearValue(Control::BorderBrushProperty());
        field.ClearValue(A11y::HelpTextProperty());
      } else {
        field.BorderBrush(brush(danger));
        A11y::SetHelpText(field, message.Text());
      }
      // 只在显隐变化时通知读屏，免得每敲一个字都播报一遍。
      const auto shown = valid ? Visibility::Collapsed : Visibility::Visible;
      if (message.Visibility() != shown) {
        message.Visibility(shown);
        if (!valid)
          if (auto peer =
                  Automation::Peers::FrameworkElementAutomationPeer::FromElement(message))
            peer.RaiseAutomationEvent(Automation::Peers::AutomationEvents::LiveRegionChanged);
      }
      if (valid)
        apply(font);
    };

    input.GotFocus([this, focused, update_menu](Inspectable const &, RoutedEventArgs const &) {
      *focused = true;
      request_font_catalog();
      update_menu();
    });
    input.LostFocus([this, focused, weak_note = make_weak(note)](Inspectable const &,
                                                                 RoutedEventArgs const &) {
      *focused = false;
      if (auto hint = weak_note.get())
        hint.Visibility(Visibility::Collapsed);
      flush_pending();
    });
    input.TextChanged([search, update_menu, write](AutoSuggestBox const &sender,
                                                   AutoSuggestBoxTextChangedEventArgs const &args) {
      if (args.Reason() != AutoSuggestionBoxTextChangeReason::UserInput)
        return;
      *search = true;
      write(std::wstring(sender.Text().c_str()));
      update_menu();
    });
    input.QuerySubmitted([search, write](AutoSuggestBox const &sender,
                                         AutoSuggestBoxQuerySubmittedEventArgs const &args) {
      const auto item = args.ChosenSuggestion();
      if (!item)
        return;
      const std::wstring font = unbox_value_or<hstring>(item, L"").c_str();
      sender.Text(hstring(font));
      *search = false;
      sender.IsSuggestionListOpen(false);
      write(font);
    });
    return box;
  }

  std::wstring font_catalog_message() const {
    switch (font_catalog_status_) {
    case FontCatalogStatus::loading:
      return L"正在读取字体列表。";
    case FontCatalogStatus::failed:
      return L"读取字体列表失败，可重试或手动输入。";
    case FontCatalogStatus::ready:
      return font_catalog_.empty() ? L"系统字体列表为空，可手动输入。" : L"";
    case FontCatalogStatus::idle:
      break;
    }
    return L"";
  }

  // Runs `call` off the UI thread and gives its response to `done` back on it, unless the window has closed.
  fire_and_forget run_host(std::function<Response()> call,
                           std::function<void(Response const &)> done) {
    auto weak = get_weak();
    auto queue = DispatcherQueue();
    co_await resume_background();
    auto response = call();
    queue.TryEnqueue([weak, done = std::move(done), response = std::move(response)] {
      if (auto self = weak.get(); self && !self->closed_)
        done(response);
    });
  }

  // 第一次点开字体输入框时读取；读过以后只有「刷新字体列表」重读。
  void request_font_catalog() {
    if (font_catalog_status_ == FontCatalogStatus::idle)
      load_font_catalog();
  }

  // 在 UI 线程之外读取字体列表。读取期间列表是空的；新的一次读取开始后，较早那次的结果到了也丢弃。
  void load_font_catalog() {
    const auto generation = ++font_catalog_generation_;
    font_catalog_status_ = FontCatalogStatus::loading;
    font_catalog_.clear();
    show_font_catalog_status();
    auto fonts = std::make_shared<std::optional<std::vector<std::wstring>>>();
    run_host(
        [fonts] {
          *fonts = read_font_catalog();
          return Response{};
        },
        [this, generation, fonts](Response const &) {
          if (generation != font_catalog_generation_)
            return;
          font_catalog_status_ = *fonts ? FontCatalogStatus::ready : FontCatalogStatus::failed;
          font_catalog_ = *fonts ? std::move(**fonts) : std::vector<std::wstring>();
          show_font_catalog_status();
        });
  }

  // 在原处更新状态行、刷新按钮和正打开的下拉，不重建页面，免得输入框丢掉焦点。
  void show_font_catalog_status() {
    if (font_catalog_text_) {
      const auto message = font_catalog_message();
      font_catalog_text_.Text(hstring(message));
      font_catalog_text_.Visibility(message.empty() ? Visibility::Collapsed
                                                    : Visibility::Visible);
      if (auto peer = Automation::Peers::FrameworkElementAutomationPeer::FromElement(
              font_catalog_text_))
        peer.RaiseAutomationEvent(Automation::Peers::AutomationEvents::LiveRegionChanged);
    }
    if (font_catalog_button_)
      font_catalog_button_.IsEnabled(loaded_ &&
                                     font_catalog_status_ != FontCatalogStatus::loading);
    for (auto const &update : font_menu_updaters_)
      update();
  }

  // ---- 界面预览 ----

  // 候选窗口预览，照共享页的 `AppearanceCandidatePreview`：顶上一行 12 DIP 次要色的说明，隔 18 DIP 是预览面。下面的示例文字框、预览明暗下拉和提示是 Windows 自己的比较工具，有意保留。
  FrameworkElement make_preview() {
    StackPanel holder;
    holder.Spacing(8);
    holder.Margin(Thickness{0, 8, 0, 12});
    auto label = make_text(L"预览：固定样例随当前设置草稿变化，不代表实际输入候选。", 12, palette_.sub);
    // 与预览面相隔 18（面板的 Spacing 8 加这里的 10），即共享页 `panelPreviewLabel` 的 mb-[18px]。
    label.Margin(Thickness{0, 0, 0, 10});
    holder.Children().Append(label);
    // 卡片外面套一层 Grid：皮肤包的装饰图画在卡片上方的装饰带里，压住卡片顶边，与卡片同在这一格。
    preview_frame_ = Grid();
    preview_frame_.HorizontalAlignment(HorizontalAlignment::Center);
    preview_host_ = Border();
    preview_host_.VerticalAlignment(VerticalAlignment::Top);
    preview_frame_.Children().Append(preview_host_);
    ScrollViewer viewport;
    viewport.MaxHeight(current_page_ == "candidate" ? 160 : 248);
    viewport.HorizontalScrollBarVisibility(ScrollBarVisibility::Auto);
    viewport.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
    viewport.HorizontalContentAlignment(HorizontalAlignment::Center);
    viewport.Content(preview_frame_);
    Border stage;
    stage.Background(brush(palette_.seg_well));
    stage.CornerRadius(CornerRadius{10, 10, 10, 10});
    stage.Padding(Thickness{16, 16, 16, 16});
    stage.Child(viewport);
    holder.Children().Append(stage);

    Grid controls;
    controls.ColumnSpacing(10);
    ColumnDefinition sample_column;
    sample_column.Width(GridLength{1, GridUnitType::Star});
    ColumnDefinition theme_column;
    theme_column.Width(GridLength{1, GridUnitType::Auto});
    controls.ColumnDefinitions().Append(sample_column);
    controls.ColumnDefinitions().Append(theme_column);
    TextBox sample;
    sample.FontSize(13);
    sample.MaxLength(256);
    sample.PlaceholderText(L"预览示例文字，以空格分隔");
    sample.Text(hstring(preview_sample_));
    A11y::SetName(sample, L"预览示例文字");
    A11y::SetAutomationId(sample, L"candidate-preview-sample");
    sample.TextChanged([this](Inspectable const &sender, TextChangedEventArgs const &) {
      preview_sample_ = sender.as<TextBox>().Text().c_str();
      update_preview();
    });
    controls.Children().Append(sample);
    auto theme = select_control(
        L"预览颜色模式",
        {{L"follow", L"跟随设置"}, {L"light", L"预览浅色"}, {L"dark", L"预览深色"}},
        preview_dark_ ? (*preview_dark_ ? L"dark" : L"light") : L"follow",
        [this](std::wstring const &value) {
          // 只改变预览，本机输入法和其他窗口继续使用保存的主题。
          preview_dark_ = value == L"follow" ? std::nullopt
                                            : std::optional<bool>(value == L"dark");
          update_preview();
        }, true, false);
    A11y::SetAutomationId(theme, L"candidate-preview-appearance");
    Grid::SetColumn(theme, 1);
    controls.Children().Append(theme);
    holder.Children().Append(controls);
    holder.Children().Append(make_text(
        L"输入示例文字比较字体，调整下方设置查看效果。", 11, palette_.sub));
    update_preview();
    return holder;
  }

  void update_preview() {
    if (!preview_host_ || !preview_frame_)
      return;
    // 使用候选窗自身的调色板；预览可独立比较浅深色，不改变输入法的主题。
    const auto theme = candidate_theme(document_.String(L"global_theme", L"system"),
                                       true, preview_dark_);
    const bool vertical =
        document_.String(L"candidate_layout", L"vertical") == L"vertical";
    const auto count = std::clamp<int>(
        static_cast<int>(std::lround(document_.Number(L"candidate_page_size", 6))),
        1, 9);
    const double font = std::clamp(document_.Number(L"candidate_font_size", 18),
                                   12.0, 32.0);
    const bool preedit =
        document_.String(L"candidate_preedit_style", L"pinyin") != L"empty";
    // 首行的水杉 logo 随 `show_app_logo`（缺值读成关，与「布局」里那一行相同）。
    const bool brand = document_.Boolean(L"show_app_logo", false);
    // 辅助码照共享页：方案是全拼或双拼、这一族的辅助码打开且「在候选窗口中显示」时，样例候选后面带上它的辅助码。缺值取 client-core 的出厂值，与「输入」页的 helpcode_rows 相同（全拼不显示、双拼显示）。
    const auto scheme = document_.String(L"scheme", MSIME_EDITION_DEFAULT_SCHEME_W);
    const bool shuangpin = scheme == L"shuangpin";
    const std::wstring helpcode_key = shuangpin ? L"shuangpin_helpcode" : L"quanpin_helpcode";
    const bool helpcode = (shuangpin || scheme == L"quanpin") &&
                          document_.Boolean(helpcode_key + L".enabled", true) &&
                          document_.Boolean(helpcode_key + L".show_in_candidate_window", shuangpin);
    auto main_family = document_.String(L"candidate_font_family", nav::candidate_default_font_family);
    if (main_family.empty())
      main_family = std::wstring(nav::candidate_default_font_family);
    auto english_family = document_.String(L"candidate_english_font", nav::candidate_default_english_font);
    if (english_family.empty())
      english_family = main_family;
    // 原生候选先用英文字体，再按保存的列表逐字回退；示例也沿用这个顺序。
    auto family_chain = english_family;
    for (const auto &family : document_.Strings(L"candidate_fallback_fonts"))
      if (!family.empty())
        family_chain += L"," + family;
    const auto preview_font = FontFamily(hstring(family_chain));
    // 字重与候选窗一致：候选文字按 `candidate_font_weight`，拼音行至少半粗。
    const auto weight_id = document_.String(L"candidate_font_weight", L"regular");
    unsigned body_weight = msime::windows::candidate_regular_font_weight;
    for (const auto &option : msime::windows::candidate_font_weights)
      if (std::wstring(option.id.begin(), option.id.end()) == weight_id)
        body_weight = option.weight;
    const auto preview_weight = Windows::UI::Text::FontWeight{static_cast<uint16_t>(body_weight)};
    // Scale, opacity and radius through the rules the card itself uses (CandidateWindowStyle.h). 皮肤已移除，圆角只取用户设置或原生主题值。
    const auto style = candidate_window_style();
    const double scale = style.scale();
    const double radius = msime::windows::candidate_card_radius(
        style, std::nullopt, theme.radius);
    const double row_radius =
        msime::windows::candidate_row_radius(style, theme.item_radius,
                                             static_cast<float>(radius));
    // 卡片的内边距。
    const double pad_x = 12 * scale, pad_y = 8 * scale;

    preview_host_.Background(
        brush(to_color(msime::windows::candidate_faded(theme.surface, style))));
    preview_host_.BorderBrush(
        brush(to_color(msime::windows::candidate_faded(theme.border, style))));
    constexpr double edge = 1;
    preview_host_.BorderThickness(Thickness{edge, edge, edge, edge});
    preview_host_.CornerRadius(CornerRadius{radius, radius, radius, radius});
    preview_host_.Padding(Thickness{0, 0, 0, 0});
    preview_host_.Margin(Thickness{0, 0, 0, 0});
    preview_host_.MinWidth(0.0);
    A11y::SetName(preview_host_, L"候选窗口预览");

    Grid layers;

    StackPanel body;
    body.Margin(Thickness{pad_x, pad_y, pad_x, 10 * scale});
    body.Spacing(6 * scale);
    // 首行照共享页的样例：打头是水杉 logo（候选窗的 16 DIP 见方、隔 6 DIP，CandidateCardSize.h 的 logo_side、logo_gap），再是拼音 ni'mf 和光标；拼音隐藏时首行只剩 logo，logo 也不画时整行不画，与候选窗一致。
    if (brand || preedit) {
      StackPanel head;
      head.Orientation(Orientation::Horizontal);
      head.Spacing(6 * scale);
      head.Padding(Thickness{4 * scale, 0, 4 * scale, 0});
      if (brand) {
        Image logo;
        logo.Width(16 * scale);
        logo.Height(16 * scale);
        logo.VerticalAlignment(VerticalAlignment::Center);
        show_logo(logo, static_cast<int32_t>(std::ceil(16 * scale * 2)));
        head.Children().Append(logo);
      }
      // The candidate window draws the preedit in the accent colour at semibold; the caret after it is the shared sample's 1.5 DIP bar, 1.2 em high.
      if (preedit) {
        const double preedit_size =
            std::clamp(document_.Number(L"candidate_preedit_font_size", 15), 12.0, 32.0) *
            scale;
        auto pre = make_text(L"ni'mf", preedit_size, to_color(theme.accent));
        pre.FontFamily(preview_font);
        pre.FontWeight(Windows::UI::Text::FontWeight{
            static_cast<uint16_t>(msime::windows::candidate_preedit_font_weight(body_weight))});
        pre.VerticalAlignment(VerticalAlignment::Center);
        head.Children().Append(pre);
        Border caret;
        caret.Width(1.5 * scale);
        caret.Height(preedit_size * 1.2);
        // 首行各项相隔 6，光标只离拼音 1（共享页 `.cursor` 的 margin-left）。
        caret.Margin(Thickness{-5 * scale, 0, 0, 0});
        caret.VerticalAlignment(VerticalAlignment::Center);
        caret.Background(brush(to_color(theme.accent)));
        head.Children().Append(caret);
      }
      body.Children().Append(head);
    }

    // 短示例循环填满设定的每页数量；清空或只输入空白时恢复内置示例。
    std::vector<std::wstring> samples;
    constexpr std::wstring_view whitespace = L" \t\r\n\v\f\u3000";
    for (size_t start = preview_sample_.find_first_not_of(whitespace);
         start != std::wstring::npos && samples.size() < preview_words.size();) {
      const auto end = preview_sample_.find_first_of(whitespace, start);
      samples.push_back(preview_sample_.substr(start, end == std::wstring::npos
                                                        ? end : end - start));
      start = end == std::wstring::npos ? end
                                        : preview_sample_.find_first_not_of(whitespace, end);
    }
    // 行距、序号、释义字号跟着候选窗的缺省版式（CandidateCardSize.h，和 macOS 一样）：行挨着行，序号 0.8 倍字号、从行左缘 8 DIP 起（主题打开选中竖条时 14 DIP，竖条画在行内 1 DIP 处），候选文字紧跟在序号后 1.5 DIP，行高是文字上下各 6 DIP，释义固定 12。
    const double number_left = theme.show_selected_bar ? 14.0 : 8.0;
    StackPanel list;
    list.Orientation(vertical ? Orientation::Vertical : Orientation::Horizontal);
    list.Spacing(0);
    for (int i = 0; i < count; ++i) {
      const auto &[default_word, code] = preview_words[static_cast<size_t>(i)];
      const std::wstring word = samples.empty() ? default_word
                                                : samples[static_cast<size_t>(i) % samples.size()];
      const bool highlighted = i == 0;
      Grid item;
      item.Padding(Thickness{number_left * scale, 6 * scale, 8 * scale, 6 * scale});
      item.CornerRadius(CornerRadius{row_radius, row_radius, row_radius, row_radius});
      if (highlighted)
        item.Background(brush(to_color(theme.selected)));
      if (highlighted && theme.show_selected_bar) {
        Border bar;
        bar.Width(3 * scale);
        bar.Height(16 * scale);
        bar.CornerRadius(CornerRadius{1.5 * scale, 1.5 * scale, 1.5 * scale,
                                      1.5 * scale});
        bar.Background(brush(to_color(theme.accent)));
        bar.HorizontalAlignment(HorizontalAlignment::Left);
        bar.VerticalAlignment(VerticalAlignment::Center);
        bar.Margin(Thickness{(1.0 - number_left) * scale, 0, 0, 0});
        item.Children().Append(bar);
      }
      StackPanel cell;
      cell.Orientation(Orientation::Horizontal);
      cell.Spacing(1.5 * scale);
      // Alpha 0 in the selected slots means "keep the unselected colour", as the renderer reads it.
      const auto number_color =
          msime::windows::candidate_row_number_color(theme, highlighted);
      const auto row_color = msime::windows::candidate_row_text_color(
          theme, theme.text, highlighted, false);
      auto number =
          make_text(std::to_wstring(i + 1), font * 0.8 * scale, to_color(number_color));
      number.FontWeight(preview_weight);
      number.VerticalAlignment(VerticalAlignment::Center);
      cell.Children().Append(number);
      // 辅助码紧跟在候选后面、同字号同色，写成「你们(rR)」，与共享页的样例相同；自己输入的示例文字没有辅助码。
      StackPanel words;
      words.Orientation(Orientation::Horizontal);
      auto candidate = make_text(word, font * scale, to_color(row_color));
      A11y::SetAutomationId(candidate, hstring(L"candidate-preview-word-" + std::to_wstring(i)));
      candidate.TextWrapping(TextWrapping::NoWrap);
      candidate.FontFamily(preview_font);
      candidate.FontWeight(preview_weight);
      words.Children().Append(candidate);
      if (helpcode && samples.empty()) {
        auto suffix = make_text(std::wstring(L"(") + code + L")", font * scale,
                                to_color(row_color));
        suffix.TextWrapping(TextWrapping::NoWrap);
        suffix.FontFamily(preview_font);
        suffix.FontWeight(preview_weight);
        words.Children().Append(suffix);
      }
      cell.Children().Append(words);
      item.Children().Append(cell);
      list.Children().Append(item);
    }
    body.Children().Append(list);
    layers.Children().Append(body);
    preview_host_.Child(layers);
    while (preview_frame_.Children().Size() > 1)
      preview_frame_.Children().RemoveAtEnd();
  }

  // ---- 候选窗口 ----

  // 共享设置「候选窗口」页的桌面版（appearance-page.tsx）：页首引导下面是四个页内标签（`candidateTabs`：布局、字体与大小、窗口样式、预编辑，page_tab_bar），标签条与标签内容之间是一张无标题卡片里的预览，每个标签里只有一组，组名与标签同名。展开候选和内边距在 Windows 上没有（能力位为假），不画；页码开关在「布局」里，排在每页候选数之后，与共享页相同。「游戏」组管游戏里候选窗能不能出现，是 Windows 独有的功能开关而不是外观，不属于哪个标签，画在标签内容下面、一直显示。
  void build_candidate_page(StackPanel const &page) {
    const auto tab = page_tab_bar(page, L"候选窗口设置",
                                  {{L"layout", L"布局"},
                                   {L"font", L"字体与大小"},
                                   {L"style", L"窗口样式"},
                                   {L"preedit", L"预编辑"}},
                                  L"candidate-tab");
    if (preview_visible_ && !indexing_) {
      auto card = add_group(page, L"");
      auto preview = make_preview();
      // 共享页 `groupPreview` 的内边距：左右 24（卡片行面板的 16 加这里的 8）、上 20、下 30。
      preview.Margin(Thickness{8, 20, 8, 30});
      card.Children().Append(preview);
    }

    if (in_tab(tab, L"layout")) {
      auto layout = add_group(page, L"布局");
      segment_row(layout, 0xE8FD, L"候选项排列方式", L"", L"candidate_layout",
                  {{L"horizontal", L"横向"}, {L"vertical", L"纵向"}},
                  L"vertical");
      candidate_page_size_row(layout);
      // 与共享设置「候选窗口」页的同名开关相同（`HostCapabilities::candidate_page_number`，`candidate-page-number-section.tsx`）：候选窗首行的「1 / 3」和 ‹ › 一起隐藏（`CandidateLayoutSettings.h`），缺值读成开（#10709）。预览只画一页样例，没有页码，不用重画。
      bool_row(layout, 0xE8A9, L"显示页码和翻页按钮",
               L"关闭后候选窗不再显示「1 / 3」和 ‹ ›，仍可用键盘翻页；鼠标滚轮翻页默认关闭，需要时在「输入 › 选词与翻页」里打开。",
               L"show_candidate_page_number", true);
      // 与共享设置「候选窗口」页的同名开关相同（`HostCapabilities::app_logo`）：新装默认关，缺值读成关。
      bool_row(layout, 0xE790, L"显示水杉 logo",
               L"在候选窗和悬浮工具栏左端显示水杉图标。", L"show_app_logo",
               false);
      bool_row(layout, 0xE718, L"跟随光标",
               L"关闭后保持首次出现的位置，直到候选窗口消失。",
               L"candidate_follow_cursor", true);
    }

    if (in_tab(tab, L"font")) {
      auto font = add_group(page, L"字体与大小");
      const bool main_font = candidate_font_rows(font);
      number_select_row(font, 0xE8E9, L"字号", L"", L"candidate_font_size",
                        candidate_font_sizes(), 18, L"");
      // 共享偏好 `candidate_font_weight`（#10748），四档和字重在 CandidateFontWeight.h；缺值是常规，等于这一项出现之前的画法。
      std::vector<Option> weights;
      for (const auto &option : msime::windows::candidate_font_weights)
        weights.push_back({std::wstring(option.id.begin(), option.id.end()), std::wstring(option.label)});
      select_row(font, 0xE8DD, L"字重", L"作用于候选、注释、序号和释义，拼音行至少半粗。字体没有这一档字面时按最接近的字面显示。",
                 L"candidate_font_weight", std::move(weights), L"regular");
      candidate_english_font_row(font, !main_font);
      candidate_percent_row(font, L"整体大小", L"candidate_scale_percent", 75, 150,
                            L"字号与窗口尺寸一起缩放");
    }

    if (in_tab(tab, L"style")) {
      auto style = add_group(page, L"窗口样式");
      candidate_percent_row(style, L"不透明度", L"candidate_opacity_percent", 50, 100,
                            L"文字和焦点高亮保持不透明");
      corner_radius_row(style);
    }

    if (in_tab(tab, L"preedit")) {
      auto preedit = add_group(page, L"预编辑");
      number_select_row(preedit, 0xE8E9, L"预编辑字号", L"",
                        L"candidate_preedit_font_size", candidate_font_sizes(), 15, L"");
      select_row(
          preedit, 0xE70F, L"候选窗口预编辑", L"", L"candidate_preedit_style",
          {{L"pinyin", L"拼音分词"}, {L"empty", L"不显示"}}, L"pinyin", true);
      select_row(preedit, 0xE8D2, L"行内预编辑", L"", L"tsf_preedit_style",
                 {{L"raw", L"原始按键"},
                  {L"pinyin", L"拼音分词"},
                  {L"empty", L"不显示"}},
                 L"raw");
    }

    // 「游戏」组不在任何标签里：建索引时之后的条目不再记在最后一个标签下，搜索结果的位置只写「候选窗口 › 游戏」，打开它时也不切标签。
    if (indexing_)
      tab_index_.clear();
    auto games = add_group(page, L"游戏");
    bool_row(
        games, 0xE7FC, L"在游戏中显示候选窗口",
        L"游戏声明自己画候选却不画时，由水杉显示候选窗口。CS2、Dota 2 和使用 "
        L"SDL2 "
        L"的游戏会自动识别。候选窗口不出现或跑到左上角的游戏，可以加进「总是"
        L"显示」；游戏自带的候选被替换时，加进「从不显示」。对已打开的游戏，"
        L"切换一次输入法后生效。独占全屏下不显示，请把游戏改为无边框窗口。",
        L"game_compatibility.candidate_overlay", true);
    const bool gameProfile = msime::windows::game_profile::enabled();
    add_row(games, 0xE7FC, L"CS2 / Dota 2 原生候选兼容",
            L"将本版本的系统登记名称改为「拼音输入法」，让游戏自行绘制候选。"
            L"Windows 输入法列表也会显示这个名称，需要管理员授权并重启游戏；"
            L"关闭后恢复本版本名称。用户「总是显示」列表仍优先。",
            button_control(gameProfile ? L"恢复产品名称" : L"启用兼容名称", [this, gameProfile] {
              const auto error = msime::windows::game_profile::apply(GetActiveWindow(), !gameProfile);
              if (!error.empty()) show_notice(error);
              else show_notice(L"登记名称已更新，请重启游戏并重新切换输入法。");
              render_page(true);
            }));
    game_process_row(
        games, 0xE890, L"总是显示候选窗口的程序",
        L"候选窗口不出现或跑到左上角的游戏。填程序文件名，例如 game.exe。",
        L"game_compatibility.overlay_processes");
    game_process_row(
        games, 0xED1A, L"从不显示候选窗口的程序",
        L"自带候选被水杉的候选窗口替换的游戏。填程序文件名，例如 game.exe。",
        L"game_compatibility.excluded_processes");
  }

  // 「字号」「预编辑字号」下拉的选项：共享页的 `candidateFontSizes`，12 到 32。
  static std::vector<int> candidate_font_sizes() {
    std::vector<int> sizes;
    for (int size = 12; size <= 32; ++size)
      sizes.push_back(size);
    return sizes;
  }

  // 「每页候选项数量」照共享页的 CandidatePageSizeSection：带刻度的滑块，数值在右边，没有说明。范围是 3 到 Windows 的上限 9（client-core 的 max_candidate_page_size）；存着的值不在这个范围里时把范围放宽到它，而不是改写它（`offeredCandidatePageSizes`）。
  void candidate_page_size_row(StackPanel const &group) {
    const auto current =
        static_cast<int>(std::lround(document_.Number(L"candidate_page_size", 6)));
    auto control = slider_control(
        L"每页候选项数量", std::min(current, 3), std::max(current, 9), current,
        [this](double value) {
          document_.SetNumber(L"candidate_page_size", value);
          schedule_save();
          update_preview();
        });
    // 每一档一个刻度，画在轨道上，与共享页 `platform-slider-ticks` 相同。
    auto slider = control.as<Grid>().Children().GetAt(0).as<Slider>();
    slider.TickFrequency(1);
    slider.TickPlacement(Primitives::TickPlacement::Inline);
    add_row(group, 0xE8CB, L"每页候选项数量", L"", control);
  }

  // 「整体大小」「不透明度」两个百分比滑块（共享页的 CandidateScaleRow 和 CandidateWindowStyleRows）：每档 5%，说明随数值写成「N%，<detail>」，拖动时就地改写说明，不重画整页。共享页这两行只传 valueText、不传 displayValue，滑块旁不画数值（valueText 只是读屏用的 aria-valuetext），所以这里收起滑块旁的数值文字，数值只写在说明里。
  void candidate_percent_row(StackPanel const &group, std::wstring const &title,
                             std::wstring const &key, double minimum, double maximum,
                             std::wstring const &detail) {
    const double value = std::clamp(document_.Number(key, 100), minimum, maximum);
    auto line = std::make_shared<weak_ref<TextBlock>>();
    auto control = slider_control(title, minimum, maximum, value,
                                  [this, key, detail, line](double next) {
                                    document_.SetNumber(key, next);
                                    schedule_save();
                                    update_preview();
                                    if (auto block = line->get())
                                      block.Text(hstring(number_text(next) + L"%，" + detail));
                                  },
                                  true, 5, L"%");
    control.as<Grid>().Children().GetAt(1).Visibility(Visibility::Collapsed);
    auto row = add_row(group, 0xE740, title, number_text(value) + L"%，" + detail, control);
    *line = make_weak(row_subtitle(row));
  }

  // 「圆角大小」照共享页 CandidateWindowStyleRows：滑块写的值覆盖皮肤包的圆角；旁边的「跟随皮肤」清掉它，卡片又跟随皮肤包、没有包时跟随主题。跟随时这个按钮是按下的（选中的 ToggleButton，与「自定义配色」里的「跟随主题」相同），再点一次不起作用。说明随数值变化：跟随时是「跟随皮肤；设置后覆盖皮肤自带的圆角」，覆盖时是「Npt，覆盖皮肤自带的圆角」；拖动时就地改写说明、抬起按钮，不重画整页。共享页的圆角滑块只传 valueText（读屏用的 aria-valuetext）、不传 displayValue，滑块旁不画数值，所以这里收起滑块旁的数值文字，跟随与否只看说明和按钮。没设圆角时滑块停在主题的圆角上。
  void corner_radius_row(StackPanel const &group) {
    const auto stored = document_.Value(L"candidate_corner_radius");
    const bool overridden = stored && stored.ValueType() == JsonValueType::Number;
    const double fallback =
        msime::windows::candidate_native_palette(candidate_dark()).radius;
    Primitives::ToggleButton follow;
    follow.Content(box_value(L"跟随主题"));
    follow.Height(32);
    follow.IsChecked(!overridden);
    follow.IsEnabled(loaded_);
    A11y::SetName(follow, L"跟随主题");
    // 按文档里现在的值判断：拖过滑块以后页面没有重画，画出来时的状态已经过时。
    follow.Click([this](Inspectable const &sender, RoutedEventArgs const &) {
      const auto current = document_.Value(L"candidate_corner_radius");
      if (current && current.ValueType() == JsonValueType::Number)
        change([](PreferencesDocument &doc) {
          doc.SetValue(L"candidate_corner_radius", JsonValue::CreateNullValue());
        }, true);
      else
        sender.as<Primitives::ToggleButton>().IsChecked(true);
    });
    auto line = std::make_shared<weak_ref<TextBlock>>();
    StackPanel box;
    box.Orientation(Orientation::Horizontal);
    box.Spacing(8);
    auto slider = formatted_slider_control(
        L"圆角大小", 0, 16, overridden ? stored.GetNumber() : fallback,
        [this, line, weak_follow = make_weak(follow)](double value) {
          document_.SetNumber(L"candidate_corner_radius", value);
          schedule_save();
          update_preview();
          if (auto block = line->get())
            block.Text(hstring(number_text(value) + L"pt，覆盖主题自带的圆角"));
          if (auto button = weak_follow.get())
            button.IsChecked(false);
        },
        [](double shown) { return number_text(shown) + L"pt"; });
    slider.Children().GetAt(1).Visibility(Visibility::Collapsed);
    box.Children().Append(slider);
    box.Children().Append(follow);
    auto row = add_row(group, 0xE739, L"圆角大小",
                       overridden ? number_text(stored.GetNumber()) + L"pt，覆盖主题自带的圆角"
                                  : std::wstring(L"跟随主题；设置后覆盖主题自带的圆角"),
                       box);
    *line = make_weak(row_subtitle(row));
  }

  static const wchar_t *app_input_mode_rule_problem(nav::AppInputModeRuleError error) {
    switch (error) {
    case nav::AppInputModeRuleError::Empty:
      return L"请填写程序文件名，例如 code.exe。";
    case nav::AppInputModeRuleError::TooLong:
      return L"程序文件名不能超过 64 个字节（一个汉字算 3 个字节）。";
    case nav::AppInputModeRuleError::InvalidCharacter:
      return L"程序文件名不能包含控制字符。";
    case nav::AppInputModeRuleError::NotExe:
      return L"程序文件名要以 .exe 结尾，例如 code.exe。";
    case nav::AppInputModeRuleError::Duplicate:
      return L"这个程序已经有例外了，在下面的列表里改它的模式即可。";
    case nav::AppInputModeRuleError::TooMany:
      return L"最多 32 个应用例外，请先移除不再需要的。";
    }
    return L"";
  }

  // 文档里的应用例外，复制成一份可以改的对象。共享偏好里同一张表也装着 macOS 的 bundle id，它们原样带过去，不在这里丢掉。
  static JsonObject app_input_mode_rules_of(PreferencesDocument const &doc) {
    JsonObject rules;
    const auto stored = doc.Value(L"app_input_mode_rules");
    if (stored && stored.ValueType() == JsonValueType::Object)
      for (auto const &entry : stored.GetObject())
        rules.SetNamedValue(entry.Key(), JsonValue::Parse(entry.Value().Stringify()));
    return rules;
  }

  // 应用例外（`app_input_mode_rules`）：输入框加「添加」按钮，每条规则一行，可以选中文或英文、可以移除。新加的规则从中文开始，和 macOS 与共享设置页相同。写入前先按偏好库的规则规范化和校验（AppInputModeRuleList.h），不合法时在本行下面说明原因，不写入。
  void app_input_mode_rule_row(StackPanel const &group) {
    std::vector<std::pair<std::wstring, std::wstring>> rules;
    for (auto const &entry : app_input_mode_rules_of(document_))
      if (entry.Value().ValueType() == JsonValueType::String)
        rules.emplace_back(std::wstring(entry.Key().c_str()),
                           std::wstring(entry.Value().GetString().c_str()));
    std::sort(rules.begin(), rules.end());
    const auto set_rule = [this](std::wstring const &name,
                                 std::optional<std::wstring> const &mode) {
      change([&](PreferencesDocument &doc) {
        auto current = app_input_mode_rules_of(doc);
        if (mode)
          current.SetNamedValue(hstring(name), JsonValue::CreateStringValue(hstring(*mode)));
        else if (current.HasKey(hstring(name)))
          current.Remove(hstring(name));
        doc.SetValue(L"app_input_mode_rules", current);
      }, true);
    };
    InfoBar problem;
    problem.IsOpen(false);
    problem.IsClosable(true);
    problem.Severity(InfoBarSeverity::Error);
    problem.Margin(Thickness{0, 0, 0, 16});
    problem.Visibility(Visibility::Collapsed);
    problem.Closed([](InfoBar const &sender, InfoBarClosedEventArgs const &) {
      sender.Visibility(Visibility::Collapsed);
    });
    TextBox input;
    input.Width(180);
    input.PlaceholderText(L"例如 code.exe");
    input.IsEnabled(loaded_);
    A11y::SetName(input, L"应用例外的程序文件名");
    StackPanel box;
    box.Orientation(Orientation::Horizontal);
    box.Spacing(8);
    box.Children().Append(input);
    box.Children().Append(button_control(
        L"添加",
        [this, set_rule, weak_input = make_weak(input), weak_problem = make_weak(problem)] {
          auto field = weak_input.get();
          if (!field)
            return;
          const auto name = nav::normalize_app_input_mode_rule(std::wstring_view(field.Text()));
          std::vector<std::wstring> existing;
          for (auto const &entry : app_input_mode_rules_of(document_))
            existing.emplace_back(entry.Key().c_str());
          if (const auto error = nav::validate_app_input_mode_rule(name, existing)) {
            if (auto bar = weak_problem.get()) {
              bar.Message(hstring(app_input_mode_rule_problem(*error)));
              bar.Visibility(Visibility::Visible);
              bar.IsOpen(true);
            }
            return;
          }
          set_rule(name, std::wstring(L"chinese"));
        },
        loaded_));
    StackPanel below;
    if (!rules.empty()) {
      StackPanel list;
      list.Spacing(4);
      list.Margin(Thickness{0, 0, 0, 16});
      for (auto const &rule : rules) {
        const std::wstring name = rule.first;
        const std::wstring mode = rule.second;
        Grid item;
        item.ColumnSpacing(16);
        ColumnDefinition name_column;
        name_column.Width(GridLength{1, GridUnitType::Star});
        ColumnDefinition mode_column;
        mode_column.Width(GridLength{1, GridUnitType::Auto});
        ColumnDefinition action_column;
        action_column.Width(GridLength{1, GridUnitType::Auto});
        item.ColumnDefinitions().Append(name_column);
        item.ColumnDefinitions().Append(mode_column);
        item.ColumnDefinitions().Append(action_column);
        auto label = make_text(name, 14, palette_.text);
        label.VerticalAlignment(VerticalAlignment::Center);
        Grid::SetColumn(label, 0);
        item.Children().Append(label);
        auto mode_select = select_control(
            name + L" 的输入模式", {{L"chinese", L"中文"}, {L"english", L"英文"}}, mode,
            [set_rule, name](std::wstring const &value) { set_rule(name, value); });
        Grid::SetColumn(mode_select, 1);
        item.Children().Append(mode_select);
        auto remove_button = button_control(L"移除", [set_rule, name] {
          set_rule(name, std::nullopt);
        }, loaded_);
        A11y::SetName(remove_button, hstring(L"移除 " + name));
        Grid::SetColumn(remove_button, 2);
        item.Children().Append(remove_button);
        list.Children().Append(item);
      }
      below.Children().Append(list);
    }
    below.Children().Append(problem);
    add_row(group, 0xE71D, L"应用例外",
            L"切到这些程序时从指定的中文或英文开始，优先于上面的默认状态和记忆，「全局统一」下也生效；在程序里手动切换后，到下次切回这个程序之前不再套用。",
            box, below);
  }

  static const wchar_t *game_process_problem(nav::GameProcessError error) {
    switch (error) {
    case nav::GameProcessError::Empty:
      return L"请填写程序文件名，例如 game.exe。";
    case nav::GameProcessError::TooLong:
      return L"程序文件名不能超过 64 个字符。";
    case nav::GameProcessError::InvalidCharacter:
      return L"只填程序文件名，不要带文件夹路径，也不能包含 \\ / : * ? \" < > | 和控制字符。";
    case nav::GameProcessError::NotExe:
      return L"程序文件名要以 .exe 结尾，例如 game.exe。";
    case nav::GameProcessError::Duplicate:
      return L"这个程序已经在「总是显示」或「从不显示」列表中，同一个程序只能放在一张列表里。";
    case nav::GameProcessError::TooMany:
      return L"两张列表合计最多 32 个程序，请先移除不再需要的。";
    }
    return L"";
  }

  // 游戏组的程序列表：输入框加「添加」按钮，每个程序一行带「移除」。写入前先按偏好库的规则规范化和校验（GameProcessList.h），不合法时在本行下面说明原因，不写入。
  void game_process_row(StackPanel const &group, wchar_t glyph,
                        std::wstring const &title, std::wstring const &subtitle,
                        std::wstring key) {
    const auto names = document_.Strings(key);
    // 没有问题时收起，不占行下的空白；关掉提示后同样收起。
    InfoBar problem;
    problem.IsOpen(false);
    problem.IsClosable(true);
    problem.Severity(InfoBarSeverity::Error);
    problem.Margin(Thickness{0, 0, 0, 16});
    problem.Visibility(Visibility::Collapsed);
    problem.Closed([](InfoBar const &sender, InfoBarClosedEventArgs const &) {
      sender.Visibility(Visibility::Collapsed);
    });
    TextBox input;
    input.Width(180);
    input.PlaceholderText(L"例如 game.exe");
    input.IsEnabled(loaded_);
    A11y::SetName(input, hstring(title));
    StackPanel box;
    box.Orientation(Orientation::Horizontal);
    box.Spacing(8);
    box.Children().Append(input);
    box.Children().Append(button_control(
        L"添加",
        [this, key, weak_input = make_weak(input), weak_problem = make_weak(problem)] {
          auto field = weak_input.get();
          if (!field)
            return;
          const auto name = nav::normalize_game_process(std::wstring_view(field.Text()));
          if (const auto error = nav::validate_game_process(
                  name, document_.Strings(L"game_compatibility.overlay_processes"),
                  document_.Strings(L"game_compatibility.excluded_processes"))) {
            if (auto bar = weak_problem.get()) {
              bar.Message(hstring(game_process_problem(*error)));
              bar.Visibility(Visibility::Visible);
              bar.IsOpen(true);
            }
            return;
          }
          change([&](PreferencesDocument &doc) {
            auto current = doc.Strings(key);
            current.push_back(name);
            doc.SetStrings(key, std::move(current));
          }, true);
        },
        loaded_));
    StackPanel below;
    if (!names.empty()) {
      StackPanel list;
      list.Spacing(4);
      list.Margin(Thickness{0, 0, 0, 16});
      for (auto const &name : names) {
        Grid item;
        item.ColumnSpacing(16);
        ColumnDefinition name_column;
        name_column.Width(GridLength{1, GridUnitType::Star});
        ColumnDefinition action_column;
        action_column.Width(GridLength{1, GridUnitType::Auto});
        item.ColumnDefinitions().Append(name_column);
        item.ColumnDefinitions().Append(action_column);
        auto label = make_text(name, 14, palette_.text);
        label.VerticalAlignment(VerticalAlignment::Center);
        Grid::SetColumn(label, 0);
        item.Children().Append(label);
        auto remove_button = button_control(L"移除", [this, key, name] {
          change([&](PreferencesDocument &doc) {
            auto current = doc.Strings(key);
            current.erase(std::remove(current.begin(), current.end(), name),
                          current.end());
            doc.SetStrings(key, std::move(current));
          }, true);
        }, loaded_);
        A11y::SetName(remove_button, hstring(L"移除 " + name));
        Grid::SetColumn(remove_button, 1);
        item.Children().Append(remove_button);
        list.Children().Append(item);
      }
      below.Children().Append(list);
    }
    below.Children().Append(problem);
    add_row(group, glyph, title, subtitle, box, below);
  }

  // The style fields as the preview draws them. A document out of range previews the defaults; the card itself refuses such a document and keeps its previous style.
  msime::windows::CandidateWindowStyle candidate_window_style() const {
    msime::windows::CandidateWindowStyle style;
    style.scale_percent = static_cast<unsigned>(
        std::lround(document_.Number(L"candidate_scale_percent", 100)));
    style.opacity_percent = static_cast<unsigned>(
        std::lround(document_.Number(L"candidate_opacity_percent", 100)));
    const auto radius = document_.Value(L"candidate_corner_radius");
    if (radius && radius.ValueType() == JsonValueType::Number)
      style.corner_radius =
          static_cast<unsigned>(std::lround((std::max)(0.0, radius.GetNumber())));
    return style.valid() ? style : msime::windows::CandidateWindowStyle{};
  }

  // ---- 悬浮工具栏 ----

  struct ToolbarComponent {
    const wchar_t *id;
    const wchar_t *label;
    bool fallback;
    int button;
  };

  // 复选框按这张表的顺序排，与共享设置页 floating-toolbar-options.ts 相同；预览用同一张表，但按原生 floating_toolbar_slots 的顺序画（标点在全/半角之前，与 macOS 工具栏一样），见 update_toolbar_preview。表情、手写、屏幕键盘和语音按钮已随对应功能移除。
  static constexpr std::array<ToolbarComponent, 6> toolbar_components_{{
      {L"english_mode", L"中英文切换", true, msime::windows::kToolbarLanguage},
      {L"input_scheme", L"切换输入方案", true,
       msime::windows::kToolbarInputScheme},
      {L"fullwidth", L"全角 / 半角", true, msime::windows::kToolbarFullwidth},
      {L"punctuation", L"中英文标点", true,
       msime::windows::kToolbarPunctuation},
      {L"character_set", L"简繁切换", true,
       msime::windows::kToolbarCharacterSet},
      {L"settings", L"设置", true, msime::windows::kToolbarSettings},
  }};

  // 「悬浮工具栏」页首的预览，照共享页的 SettingsPreviewBlock（放在无标题的组里）：12 DIP 次要色的「预览」小字，下面是与主题卡片预览面同样底色的一块，工具栏靠左，太宽时横向滚动。没有尺寸和状态那一行；工具栏画出来的 DIP 尺寸写在「预览」小字的帮助文本里（自动化 ID `toolbar-preview-label`，读屏名「悬浮工具栏预览」，与共享页的 aria-label 相同），界面验收靠它确认调整按钮后预览跟着变。
  FrameworkElement make_toolbar_preview() {
    StackPanel block;
    block.Spacing(8);
    block.Padding(Thickness{25, 14, 25, 18});
    toolbar_preview_label_ = make_text(L"预览", 12, palette_.sub);
    A11y::SetName(toolbar_preview_label_, L"悬浮工具栏预览");
    A11y::SetAutomationId(toolbar_preview_label_, L"toolbar-preview-label");
    block.Children().Append(toolbar_preview_label_);
    toolbar_preview_host_ = Border();
    toolbar_preview_host_.HorizontalAlignment(HorizontalAlignment::Left);
    A11y::SetAutomationId(toolbar_preview_host_, L"toolbar-preview");
    ScrollViewer viewport;
    viewport.HorizontalScrollBarVisibility(ScrollBarVisibility::Auto);
    viewport.VerticalScrollBarVisibility(ScrollBarVisibility::Disabled);
    viewport.Content(toolbar_preview_host_);
    Border stage;
    // 皮肤已移除：预览面用原生预览底色。
    stage.Background(brush(palette_.preview_bg));
    stage.Padding(Thickness{24, 18, 24, 18});
    stage.Child(viewport);
    block.Children().Append(stage);
    update_toolbar_preview();
    return block;
  }

  void update_toolbar_preview() {
    if (!toolbar_preview_host_)
      return;
    // 工具栏跟随颜色模式（Server 不读 toolbar_theme），预览也只看 `theme`。
    const auto common_mode = document_.String(L"theme", L"system");
    const bool dark = common_mode == L"dark" || (common_mode == L"system" && system_dark_);
    const auto sample = make_toolbar_sample(msime::windows::toolbar_palette(candidate_theme(
        document_.String(L"global_theme", L"system"), true, dark)), true);
    toolbar_preview_host_.Child(sample);
    if (toolbar_preview_label_)
      A11y::SetHelpText(toolbar_preview_label_,
                        hstring(number_text(sample.Width()) + L" × " +
                                number_text(sample.Height()) + L" DIP"));
  }

  // 一条悬浮工具栏，照原生工具栏的度量（toolbar_metrics、toolbar_cell）和 `colors` 画，按钮状态按默认输入状态。`current_settings` 为真时（「悬浮工具栏」页的预览，共享页传 `preferences={floatingToolbar}`）按钮、大小和 logo 取当前设置；为假时（「主题」页每张卡片，共享页的 `<SkinToolbarPreview />` 不带设置）画全部按钮、100% 缩放、默认图标尺寸、不画 logo。返回的块定了宽高，就是工具栏画出来的 DIP 尺寸。
  Border make_toolbar_sample(msime::windows::CandidatePalette const &colors,
                             bool current_settings) {
    const double scale =
        current_settings
            ? document_.Number(L"floating_toolbar.scale_percent", 100) / 100.0
            : 1.0;
    const double font_size =
        current_settings ? document_.Number(L"floating_toolbar.font_size", 24)
                         : 24.0;
    const bool logo =
        current_settings && document_.Boolean(L"show_app_logo", false);
    const auto metrics =
        msime::windows::toolbar_metrics(font_size, false, logo);
    std::vector<ToolbarComponent> components;
    for (const auto &item : toolbar_components_) {
      if (!current_settings ||
          document_.Boolean(L"floating_toolbar." + std::wstring(item.id),
                            item.fallback))
        components.push_back(item);
    }
    // 工具栏上标点排在全/半角之前（floating_toolbar_slots），表里两项相邻、顺序相反，两项都在时对调。
    const auto fullwidth = std::find_if(components.begin(), components.end(), [](ToolbarComponent const &item) {
      return item.button == msime::windows::kToolbarFullwidth;
    });
    const auto punctuation = std::find_if(components.begin(), components.end(), [](ToolbarComponent const &item) {
      return item.button == msime::windows::kToolbarPunctuation;
    });
    if (fullwidth != components.end() && punctuation != components.end() && fullwidth < punctuation)
      std::iter_swap(fullwidth, punctuation);
    const double width =
        msime::windows::toolbar_content_width(components.size(), metrics) *
        scale;
    const double height = metrics.height * scale;
    Canvas canvas;
    canvas.Width(width);
    canvas.Height(height);
    if (logo) {
      Image image;
      const auto rect = msime::windows::toolbar_logo(metrics);
      image.Width((rect.right - rect.left) * scale);
      image.Height((rect.bottom - rect.top) * scale);
      Canvas::SetLeft(image, rect.left * scale);
      Canvas::SetTop(image, rect.top * scale);
      show_logo(image,
                static_cast<int32_t>(std::ceil(metrics.icon * scale * 2)));
      canvas.Children().Append(image);
    } else {
      auto grip = make_text(L"⠿", 12 * scale, to_color(colors.number));
      Canvas::SetLeft(grip, 1 * scale);
      Canvas::SetTop(grip, (metrics.height - 16) * scale / 2);
      canvas.Children().Append(grip);
    }
    // logo 和按钮之间的分隔线，与原生工具栏一样宽 1.2、和按钮一样高；没有强调色的拖动条。没有按钮时不画。
    if (!components.empty()) {
      const auto line = msime::windows::toolbar_divider(metrics);
      Border divider;
      divider.Width((line.right - line.left) * scale);
      divider.Height((line.bottom - line.top) * scale);
      divider.Background(brush(to_color(colors.divider.value_or(colors.border))));
      Canvas::SetLeft(divider, line.left * scale);
      Canvas::SetTop(divider, line.top * scale);
      canvas.Children().Append(divider);
    }
    msime::windows::ToolbarLanguageState language;
    language.scheme = msime::windows::scheme::scheme_from_name(utf8(
        hstring(document_.String(L"scheme", MSIME_EDITION_DEFAULT_SCHEME_W))));
    language.mode = msime::windows::scheme::input_mode(language.scheme);
    for (size_t index = 0; index < components.size(); ++index) {
      const auto &component = components[index];
      const auto rect = msime::windows::toolbar_cell(index, metrics);
      Border cell;
      cell.Width((rect.right - rect.left) * scale);
      cell.Height((rect.bottom - rect.top) * scale);
      Canvas::SetLeft(cell, rect.left * scale);
      Canvas::SetTop(cell, rect.top * scale);
      const bool chinese_mode = document_.String(L"default_ime_mode", L"chinese") != L"english";
      // 标点按钮照 TIP 激活时定下的标点画：锁定时取锁定值，跟随时英文模式是英文标点、中文模式是已存的起始标点（PunctuationSettings.h 的 toolbar_chinese_punctuation）。
      const bool state =
          component.button == msime::windows::kToolbarLanguage
              ? chinese_mode
          : component.button == msime::windows::kToolbarFullwidth
              ? document_.String(L"character_width", L"halfwidth") ==
                    L"fullwidth"
          : component.button == msime::windows::kToolbarPunctuation
              ? ps::toolbar_chinese_punctuation(
                    chinese_mode, document_.Boolean(L"chinese_punctuation", true),
                    document_.String(L"punctuation_lock", L"follow"))
          : component.button == msime::windows::kToolbarCharacterSet
              ? document_.Boolean(L"traditional_chinese_output", false)
              : true;
      const auto icon =
          msime::windows::toolbar_icon(component.button, state, language);
      // 模式按钮是文字，用工具栏的中文界面字体、字号的 0.95 画，与原生工具栏相同；其余按钮是图标字体的字形。
      FrameworkElement glyph{nullptr};
      if (icon.codepoint) {
        glyph = make_icon(icon.codepoint, metrics.icon * scale, to_color(colors.text));
      } else {
        auto label = make_text(icon.fallback, metrics.icon * msime::windows::toolbar_glyph_scale * scale,
                               to_color(colors.text));
        label.FontFamily(Media::FontFamily(icon.family ? icon.family : msime::windows::toolbar_text_font));
        glyph = label;
      }
      glyph.HorizontalAlignment(HorizontalAlignment::Center);
      glyph.VerticalAlignment(VerticalAlignment::Center);
      cell.Child(glyph);
      A11y::SetName(cell, hstring(component.label));
      ToolTipService::SetToolTip(cell, box_value(hstring(component.label)));
      canvas.Children().Append(cell);
    }
    Border sample;
    sample.Width(width);
    sample.Height(height);
    sample.Background(brush(to_color(colors.surface)));
    const double border = colors.border_width * scale;
    const double radius = colors.radius * scale;
    sample.CornerRadius(CornerRadius{radius, radius, radius, radius});
    // 边框叠在固定尺寸的内容内，避免 XAML Border 把外观撑大并移动图标。
    Grid content;
    content.Width(width);
    content.Height(height);
    content.Children().Append(canvas);
    Border outline;
    outline.BorderBrush(brush(to_color(colors.border)));
    outline.BorderThickness(Thickness{border, border, border, border});
    outline.CornerRadius(CornerRadius{radius, radius, radius, radius});
    outline.IsHitTestVisible(false);
    content.Children().Append(outline);
    sample.Child(content);
    return sample;
  }

  // 照共享页 floating-toolbar-page.tsx 在 Windows 上的样子：页首是无标题组里的预览，然后是「显示」「按钮」「尺寸」三组，「颜色与明暗」是「尺寸」组末尾的跳转行。
  void build_toolbar_page(StackPanel const &page) {
    if (!indexing_) {
      auto preview = add_group(page, L"");
      preview.Children().Append(make_toolbar_preview());
    }
    auto display = add_group(page, L"显示");
    bool_row(display, 0xE7F4, L"在桌面显示悬浮工具栏",
             L"快速访问输入法状态与常用功能", L"floating_toolbar.enabled",
             true);
    std::vector<Check> checks;
    for (const auto &component : toolbar_components_) {
      const std::wstring key =
          L"floating_toolbar." + std::wstring(component.id);
      checks.push_back(
          {component.label, document_.Boolean(key, component.fallback),
           [this, key](bool on) {
             change([&](PreferencesDocument &doc) { doc.SetBoolean(key, on); },
                    false);
           }});
    }
    // 共享页的 Checks：说明之下一列纵向排开的复选框，14 DIP 字、相隔 8，顺序与工具栏上的相同。
    auto buttons = add_group(page, L"按钮");
    add_row(buttons, 0xE71D, L"按钮", L"勾选要显示在悬浮工具栏上的按钮",
            nullptr, checks_panel(std::move(checks), 174, 1, 14));
    // 缩放和图标尺寸相乘才是工具栏的大小，和共享页一样只给一个「工具栏大小」，选一档同时写两个值（见 FloatingToolbarSizeModel.h）。
    auto size = add_group(page, L"尺寸");
    add_row(size, 0xE740, L"工具栏大小", L"按钮和图标一起放大或缩小，不改变系统显示缩放",
            toolbar_size_control());
  }

  // 「工具栏大小」的下拉框，照共享页 floating-toolbar-page.tsx 的 SelectRow：存下的两个值凑不成任何一档时（以前分开调过），最前面多一项灰着的「自定义（缩放 X%，图标 Y）」并选着它，不改写存储；选一档后重画页面，这一项随之消失。
  FrameworkElement toolbar_size_control() {
    const int scale = static_cast<int>(
        std::lround(document_.Number(L"floating_toolbar.scale_percent", 100)));
    const int font = static_cast<int>(
        std::lround(document_.Number(L"floating_toolbar.font_size", 24)));
    const auto current = nav::toolbar_size::size_for(scale, font);
    std::vector<Option> options;
    if (!current)
      options.push_back({L"custom",
                         L"自定义（缩放 " + std::to_wstring(scale) + L"%，图标 " +
                             std::to_wstring(font) + L"）",
                         false});
    for (const auto &size : nav::toolbar_size::sizes)
      options.push_back({std::to_wstring(size.percent), std::to_wstring(size.percent) + L"%"});
    return select_control(
        L"工具栏大小", std::move(options),
        current ? std::to_wstring(current->percent) : std::wstring(L"custom"),
        [this](std::wstring const &value) {
          for (const auto &size : nav::toolbar_size::sizes) {
            if (std::to_wstring(size.percent) != value)
              continue;
            change([&](PreferencesDocument &doc) {
              doc.SetNumber(L"floating_toolbar.scale_percent", size.scale_percent);
              doc.SetNumber(L"floating_toolbar.font_size", size.font_size);
            }, true);
            return;
          }
        });
  }

  void build_typing_page(StackPanel const &page) {
    std::vector<Option> tabs;
    for (auto const &tab : ism::tabs)
      tabs.push_back({std::wstring(tab.id), std::wstring(tab.title)});
    const auto tab = page_tab_bar(page, L"输入设置分类", tabs, L"input-tab");
    if (in_tab(tab, L"scheme"))
      build_scheme_group(page);
    if (in_tab(tab, L"mode")) {
      // 两行与共享页的 DefaultImeModeSection、ImeModeScopeSection 一样是下拉框。
      auto language = add_group(page, L"中英文");
      select_row(language, 0xE774, L"默认中英文", L"新焦点会话开始时使用的中文或英文状态",
                 L"default_ime_mode", {{L"chinese", L"中文"}, {L"english", L"英文"}},
                 L"english");
      select_row(language, 0xE71D, L"中英文状态",
                 L"按应用分别记忆输入状态，或让所有输入上下文保持同一状态",
                 L"ime_mode_scope", {{L"app", L"按应用记忆"}, {L"global", L"全局统一"}},
                 L"app");
      bool_row(language, 0xE946, L"中英文切换提示",
               L"切换输入模式后，在光标附近短暂显示“中”或“英”，不会抢占焦点。",
               L"input_mode_hud", true);
      app_input_mode_rule_row(language);
    }
    if (in_tab(tab, L"selection"))
      build_selection_keys(page);
    build_input_habits(page, tab);
  }

  // 「方案」组：只保留拼音，全拼 / 双拼两张卡片，双拼时下面跟双拼方案和预编辑。
  void build_scheme_group(StackPanel const &page) {
    const auto scheme = document_.String(L"scheme", MSIME_EDITION_DEFAULT_SCHEME_W);
    std::vector<Option> chinese_schemes;
    for (Option const &option : std::vector<Option>{{L"quanpin", L"全拼"}, {L"shuangpin", L"双拼"}})
      if (edition_offers_scheme(option.value))
        chinese_schemes.push_back(option);

    auto group = add_group(page, L"方案");
    if (chinese_schemes.size() > 1)
      scheme_cards(group, chinese_schemes, scheme);

    if (scheme == L"shuangpin") {
      // 选项顺序与共享页 shuangpinProfileOptions 相同。Windows 还没有自定义双拼的编辑页，平时不列「自定义双拼」；文档里已经是它（比如在 macOS 上选的、同步过来）时照共享页列出一项灰的，否则下拉框找不到当前值，显示成空白。
      std::vector<Option> profiles{{L"xiaohe", L"小鹤双拼"}, {L"ziranma", L"自然码双拼"},
                                   {L"shoudao", L"首道双拼"}, {L"microsoft", L"微软双拼"},
                                   {L"smart_abc", L"智能 ABC 双拼"}};
      if (document_.String(L"shuangpin_profile", L"xiaohe") == L"custom")
        profiles.push_back({L"custom", L"自定义双拼", false});
      select_row(group, 0xE8AB, L"双拼方案", L"", L"shuangpin_profile", std::move(profiles),
                 L"xiaohe");
      // 共享偏好 shuangpin_keymap_hint，与共享设置的同名开关相同；没选过时文档里没有这一项，按关显示。
      bool_row(group, 0xE765, L"输入时显示双拼键位提示",
               L"双拼输入时显示当前方案的键位图，完成上屏后自动隐藏。",
               L"shuangpin_keymap_hint", false);
      // 与共享设置「输入 › 方案」双拼详情里的「双拼预编辑」相同，只在双拼方案下出现：原始按键 / 双拼分隔 / 拼音分词三档，存成 shuangpin_preedit_uses_raw 和 shuangpin_preedit_separated 两个布尔（InputSettingsModel.h），缺省是原始按键。
      std::vector<Option> preedit_options;
      std::wstring preedit_value;
      auto const stored_preedit =
          ism::shuangpin_preedit(document_.Boolean(L"shuangpin_preedit_uses_raw", true),
                                   document_.Boolean(L"shuangpin_preedit_separated", false));
      for (auto const &option : ism::shuangpin_preedit_options) {
        preedit_options.push_back({std::wstring(option.value), std::wstring(option.label)});
        if (option.style == stored_preedit)
          preedit_value = option.value;
      }
      add_row(
          group, 0xE8AB, L"双拼预编辑",
          L"保留原始双拼按键、在原始按键之间按音节加分隔符，或显示展开后的拼音分词。",
          select_control(L"双拼预编辑", std::move(preedit_options), preedit_value,
                         [this](std::wstring const &value) {
                           auto style = ism::ShuangpinPreedit::raw;
                           for (auto const &option : ism::shuangpin_preedit_options)
                             if (option.value == value)
                               style = option.style;
                           change(
                               [&](PreferencesDocument &doc) {
                                 doc.SetBoolean(L"shuangpin_preedit_uses_raw",
                                                ism::shuangpin_preedit_uses_raw(style));
                                 doc.SetBoolean(L"shuangpin_preedit_separated",
                                                ism::shuangpin_preedit_separated(style));
                               },
                               false);
                         }));
    }
  }

  // 「中文方案」卡片单选，照共享页 InputSchemeSelectorSection：标题下面三列等宽卡片，相隔 8 DIP，圆角 8、1 DIP 分隔色描边；悬停铺中性底色，选中的那张描边换成强调色、底色是淡强调色（macOS 的 `accent-soft`）。每张卡片里是一个整宽的单选按钮（自动化 ID `scheme-<方案>`），系统单选的 32 DIP 最小高度加上下各 6 的内边距，卡片高度与 macOS 上下各 12 的内边距相当。
  void scheme_cards(StackPanel const &group, std::vector<Option> const &schemes,
                    std::wstring const &scheme) {
    record(L"中文方案", L"");
    StackPanel block;
    block.Spacing(8);
    block.Children().Append(make_text(L"中文方案", 14, palette_.text));
    mark_search_target(L"中文方案", block);
    Grid cards;
    cards.ColumnSpacing(8);
    cards.RowSpacing(8);
    for (int column = 0; column < 3; ++column) {
      ColumnDefinition definition;
      definition.Width(GridLength{1, GridUnitType::Star});
      cards.ColumnDefinitions().Append(definition);
    }
    const auto soft = palette_.dark ? rgba(0x5F, 0xBF, 0x84, .22) : rgba(0x2C, 0x7A, 0x4B, .12);
    int32_t index = 0;
    for (auto const &option : schemes) {
      const bool on = option.value == scheme;
      if (index % 3 == 0) {
        RowDefinition definition;
        definition.Height(GridLength{1, GridUnitType::Auto});
        cards.RowDefinitions().Append(definition);
      }
      RadioButton radio;
      radio.Content(box_value(hstring(option.label)));
      radio.GroupName(L"input-scheme");
      radio.MinWidth(0);
      radio.HorizontalAlignment(HorizontalAlignment::Stretch);
      radio.IsChecked(on);
      radio.IsEnabled(loaded_ && option.enabled);
      radio.Checked([this, next = option.value](Inspectable const &, RoutedEventArgs const &) {
        select_scheme(next);
      });
      A11y::SetName(radio, hstring(option.label));
      A11y::SetAutomationId(radio, hstring(L"scheme-" + option.value));
      record(option.label, L"中文方案");
      mark_search_target(option.label, radio);
      Border card;
      card.CornerRadius(CornerRadius{8, 8, 8, 8});
      card.BorderThickness(Thickness{1, 1, 1, 1});
      card.BorderBrush(brush(on ? palette_.accent : palette_.divider));
      card.Background(brush(on ? soft : transparent_color()));
      card.Padding(Thickness{12, 6, 12, 6});
      card.Opacity(option.enabled ? 1.0 : 0.5);
      card.Child(radio);
      if (!on) {
        card.PointerEntered([hover = palette_.hover](Inspectable const &sender,
                                                     Microsoft::UI::Xaml::Input::PointerRoutedEventArgs const &) {
          sender.as<Border>().Background(brush(hover));
        });
        card.PointerExited([](Inspectable const &sender, Microsoft::UI::Xaml::Input::PointerRoutedEventArgs const &) {
          sender.as<Border>().Background(brush(transparent_color()));
        });
      }
      Grid::SetRow(card, index / 3);
      Grid::SetColumn(card, index % 3);
      cards.Children().Append(card);
      ++index;
    }
    block.Children().Append(cards);
    auto frame = append_row_frame(group, block);
    frame.Padding(Thickness{row_inset, 10, row_inset, 10});
  }


  // 「选词与翻页」组（共享页 InputSharedSettingsSection 的 selection 一组）：以词定字、退格回退和翻页方式。
  void build_selection_keys(StackPanel const &page) {
    auto word = add_group(page, L"选词与翻页");
    const bool word_enabled = document_.Boolean(L"word_character.enabled", false);
    const auto word_keys = document_.String(L"word_character.keys", L"brackets");
    const auto word_current = word_enabled ? word_keys : std::wstring(L"off");
    // 与共享页的 WordCharacterSection 一样是「关闭 / [ / ] / - / =」三段的分段控件。同一键组不能同时用于以词定字和翻页：与共享页的 wordCharacterChoiceChange 相同，已用于翻页的键组那一段按不下去（当前选中的除外），选键组时不去关掉另一项设置。
    std::vector<Option> word_options{Option{L"off", L"关闭"}};
    for (Option option : std::vector<Option>{{L"brackets", L"[ / ]"}, {L"minus_equal", L"- / ="}}) {
      option.enabled = option.value == word_current ||
                       !document_.Boolean(L"navigation." + option.value, false);
      word_options.push_back(std::move(option));
    }
    add_row(word, 0xE8C1, L"以词定字",
            L"按所选键组的左键上屏高亮候选的首个汉字，右键上屏末个汉字；已用于翻页的键组不能选",
            segmented_control(L"以词定字", word_options, word_current,
                              [this](std::wstring const &value) {
                                change([&](PreferencesDocument &doc) {
                                  // 选「关闭」只关开关，记着的键组不动。
                                  if (value == L"off") {
                                    doc.SetBoolean(L"word_character.enabled", false);
                                    return;
                                  }
                                  doc.SetString(L"word_character.keys", value);
                                  doc.SetBoolean(L"word_character.enabled", true);
                                }, true);
                              }));

    bool_row(word, 0xE8AB, L"退格键回退已选候选词",
             L"关闭后，退格只删除剩余拼写，保留已选文字；拼写删空时上屏已选文字。",
             L"backspace_reverts_selection", true);

    // Tab 和 PageUp/PageDown 两组固定开启，不再列出（NavigationPolicy.h 的 preference_navigation，HostCapabilities.tab_page_paging_switches）：关掉只会让它们在组字时被吃掉，并不交还应用。
    const std::array<std::pair<const wchar_t *, const wchar_t *>, 6> paging_keys{{
        {L"minus_equal", L"- / ="},
        {L"comma_period", L", / ."},
        {L"brackets", L"[ / ]"},
        {L"mouse_wheel", L"鼠标滚轮（候选窗口支持时翻页）"},
        {L"arrows", L"上 / 下（移动候选项）"},
        {L"left_right", L"左 / 右（移动候选项）"},
    }};
    std::vector<Check> paging_checks;
    for (const auto &[id, label] : paging_keys) {
      const std::wstring key = std::wstring(L"navigation.") + id;
      const std::wstring pair = id;
      paging_checks.push_back(
          {label, document_.Boolean(key, false),
           [this, key, pair, word_enabled, word_keys](bool on) {
             // 把以词定字占用的键组改为翻页时，同一次保存关闭以词定字。
             const bool conflict = on && word_enabled && pair == word_keys;
             // 以词定字的两个键组跟着翻页勾选变成可选或不可选，改它们时重画这一页。
             const bool word_group = pair == L"brackets" || pair == L"minus_equal";
             change([&](PreferencesDocument &doc) {
               doc.SetBoolean(key, on);
               if (conflict)
                 doc.SetBoolean(L"word_character.enabled", false);
             }, word_group);
           }});
    }
    add_row(word, 0xE8AB, L"翻页方式",
            L"Shift+Tab / Tab 和 PageUp / PageDown 在候选窗口显示时始终翻页", nullptr,
            checks_panel(std::move(paging_checks), 190));
  }

  void build_input_habits(StackPanel const &page, std::wstring const &tab) {
    if (in_tab(tab, L"candidates")) {
      auto candidates = add_group(page, L"候选与联想");
      // 触发字符数固定为默认的 5，不再给出（HostCapabilities.mixed_input_minimum_prefix）：调低会让常见简拼被英文词抢走首选。
      const bool mixed_enabled = document_.Boolean(L"mixed_input.enabled", true);
      bool_row(candidates, 0xE774, L"候选混输", L"关闭后保留各项设置，再开启时恢复",
               L"mixed_input.enabled", true, true);
      bool_row(candidates, 0xE774, L"中英混输", L"中文输入时在候选项中补充英文单词",
               L"mixed_input.english", true, true, mixed_enabled);
      number_select_row(candidates, 0xE774, L"首页英文候选数",
                        L"首页有空间时优先排列的英文候选数",
                        L"mixed_input.first_page_count", {1, 2, 3}, 1, L"",
                        mixed_enabled && document_.Boolean(L"mixed_input.english", true));
      // Windows 的 emoji 混输默认开（client-core 的 source_mixed_emoji_default），回退值与它一致；平时由出厂偏好兜底，读不到出厂偏好时才用到这里。
      bool_row(candidates, 0xE76E, L"emoji 混输",
               L"中文输入时在候选项中加入匹配的 emoji，紧跟在它描绘的那个词后面（如「美国」后面是 🇺🇸），对不上候选词的排在末尾",
               L"mixed_input.emoji", true, false, mixed_enabled);
      // 学习选词习惯决定「高级选项」里「拼音方案调频」组画不画，改它时重画这一页。
      bool_row(candidates, 0xE82D, L"学习选词习惯", L"根据选词调整候选顺序", L"learning",
               true, true);
      // 后加的可选键，默认关，共享序列化关着时不写它，回退值必须是 false（client-core 的 `next_word_prediction`）。位置和共享页桌面一样，紧跟学习开关。
      bool_row(candidates, 0xE8BD, L"上屏后联想",
               L"选词上屏后推荐后续文字，点选、按数字键或空格上屏，继续输入其他内容时消失；使用现有词库，开启学习时也参考个人词序，隐私输入不显示",
               L"next_word_prediction", false);
      // 与共享页一样，少有人改的混输开关收在组末的「更多选项」里。
      auto candidates_more = add_more_options(
          candidates, candidates_more_open_,
          focus_row_ == L"符号混输" || focus_row_ == L"颜文字混输" ||
              focus_row_ == L"只出单字" ||
              focus_row_ == L"移动光标后编辑光标后的部分" ||
              focus_row_ == L"本地整句联想");
      // 符号混输是后加的可选键，默认开，文档里平时没有它，回退值必须是 true。
      bool_row(candidates_more, 0xE8EF, L"符号混输",
               L"打出符号的名称时在候选项中加入对应的符号，紧跟在那个词后面（如「正负」后面是 ±、「平方米」后面是 ㎡）；要打完整的名称才出现",
               L"mixed_input.symbols", true, false, mixed_enabled);
      bool_row(candidates_more, 0xE76E, L"颜文字混输",
               L"中文输入时在候选项中加入匹配的颜文字，紧跟在它描绘的那个词后面、排在同一个词的 emoji 之后，对不上候选词的排在末尾",
               L"mixed_input.kaomoji", false, false, mixed_enabled);
      // 不带临时日语的版本没有这项能力，host-api 也会把偏好固定为关。
      bool_row(candidates_more, 0xE8C1, L"只出单字",
               L"候选只列单个汉字，不出词组和整句；选一个字后接着拼下一个字",
               L"single_character_only", false);
      // 后加的可选键，关着时共享序列化不写它，回退值必须是 false（client-core 的 `caret_edit_suffix`）。共享页把它排在「快捷短语混输」旁边，Windows 没有那一行，排在「只出单字」后面。
      bool_row(candidates_more, 0xE8C1, L"移动光标后编辑光标后的部分",
               L"组字光标移到拼音中间时，光标前的部分固定为当前转换，候选改为编辑光标后的部分；首选是整句",
               L"caret_edit_suffix", false);
      bool_row(candidates_more, 0xE8C1, L"本地整句联想",
               L"把词库组合出的整句加入候选；关闭后仍保留单词候选",
               L"sentence_association.word_lattice", true);
    }

    if (in_tab(tab, L"output")) {
      auto output = add_group(page, L"输出");
      // 「中文输出」与共享页 TraditionalChineseOutputSection 一样是简体 / 繁体两段，说明随选项和方案变（InputSettingsModel.h）。Windows 不认繁体变体，只写 traditional_chinese_output。
      const bool traditional = document_.Boolean(L"traditional_chinese_output", false);
      add_row(output, 0xE8C1, L"中文输出",
              ism::chinese_output_description(traditional),
              segmented_control(L"中文输出", {{L"simplified", L"简体"}, {L"s2t", L"繁体"}},
                                traditional ? L"s2t" : L"simplified",
                                [this](std::wstring const &value) {
                                  change([&](PreferencesDocument &doc) {
                                    doc.SetBoolean(L"traditional_chinese_output", value == L"s2t");
                                  }, true);
                                }));
      bool_row(output, 0xE8D2, L"中英文之间加空格",
               L"一次上屏的文字里，汉字与英文字母、数字相邻处自动加一个空格，如「用Rust写」上屏为「用 Rust 写」。两次上屏之间不加；网址、日期等快捷模式和全角输入不受影响，译文和释义上屏、常用语、剪贴板和语音结果也不加",
               L"chinese_english_spacing", false);
      // 全角输入是会话开始时的状态，与共享页的 CharacterWidthRow 一样直接排在组里。
      add_row(output, 0xE8E9, L"全角输入",
              L"将英文字符和空格提交为全角形式，会话开始时生效；工具栏、键盘的更多工具或快捷键可临时切换",
              toggle_control(L"全角输入",
                             document_.String(L"character_width", L"halfwidth") ==
                                 L"fullwidth",
                             [this](bool on) {
                               change([&](PreferencesDocument &doc) {
                                 doc.SetString(L"character_width",
                                               on ? L"fullwidth" : L"halfwidth");
                               }, false);
                             }));
    }

    if (!in_tab(tab, L"advanced"))
      return;
    auto local = add_group(page, L"快捷模式");
    // 总开关：关掉后没有输入时 Shift+字母照常打出大写字母，「逐个模式」里各模式的开关保留原值、变灰；/ 和 @ 不经过它。
    const bool shift_letter_entry = document_.Boolean(L"local_modes.shift_letter_entry", true);
    bool_row(local, 0xE8D3, L"Shift+字母进入快捷模式",
             L"没有输入时按 Shift 加各模式的字母进入该模式。关掉后 Shift+字母照常打出大写字母，「逐个模式」里的开关保留，/ 和 @ 不受影响",
             L"local_modes.shift_letter_entry", true, true);
    // 与共享页的 LocalModesSection 一样，各模式一行带说明的开关，收进紧跟其后的「逐个模式」组的「更多选项」（InputSettingsModel.h 的 local_modes）。总开关关着时 Shift+字母进入的各行变灰，「@ 地名」在 @ 模式关着时变灰，值都不动；没有选翻译服务时指令模式的说明补一句 fy 不会出结果。搜索落到其中一行时展开。
    auto modes_group = add_group(page, L"逐个模式");
    auto modes_more = add_fold(
        modes_group, L"更多选项", local_modes_more_open_ || fold_holds_focus(L"local-modes-more"),
        [this](bool expanded) { local_modes_more_open_ = expanded; });
    fold_index_ = L"local-modes-more";
    const bool mention = document_.Boolean(L"local_modes.mention", false);
    const bool super_jianpin = document_.Boolean(L"local_modes.super_jianpin", true);
    // 翻译功能已移除，指令模式不再补翻译提示。
    for (auto const &mode : ism::local_modes) {
      const std::wstring key = L"local_modes." + std::wstring(mode.id);
      // @ 模式决定「@ 地名」可不可用、超级简拼决定「组字后按 / 转超级简拼」可不可用，改它们时重画。
      bool_row(modes_more, 0xE945, std::wstring(mode.title),
               ism::local_mode_description(mode), key, mode.default_on,
               ism::local_mode_has_dependents(mode),
               !ism::local_mode_disabled(mode, shift_letter_entry, mention, super_jianpin));
    }
    fold_index_.clear();

    build_fuzzy_group(page);
    build_autocorrect_group(page);
    build_helpcode_group(page);

    // 与共享页的 FrequencySection 相同，只按条件显示，不改存着的值（这几项会上云）：学习选词习惯关闭时整组不画；调频方式为「关闭」时不画触发频次；步长只有线性调频读。
    if (document_.Boolean(L"learning", true)) {
      const auto frequency_mode = document_.String(L"frequency.mode", L"promote");
      auto frequency = add_group(page, L"拼音方案调频");
      select_row(frequency, 0xE8CB, L"调频方式", L"", L"frequency.mode",
                 {{L"disabled", L"关闭"}, {L"pin", L"一次置顶"}, {L"halve", L"折半调频"},
                  {L"linear", L"线性调频"}, {L"promote", L"一次置前"}},
                 L"promote", true);
      if (frequency_mode != L"disabled")
        number_select_row(frequency, 0xE8EF, L"触发频次(第几次上屏触发)", L"",
                          L"frequency.trigger_count", {1, 2, 3, 4, 5, 6}, 1, L"");
      if (frequency_mode == L"linear")
        number_select_row(frequency, 0xE8EF, L"线性调频步长", L"",
                          L"frequency.linear_step", {1, 2, 3, 4, 5, 6}, 1, L"");
    }
  }

  // 只保留拼音：全拼 / 双拼之间切换时记住当前方案。
  void select_scheme(std::wstring const &next) {
    const auto current = document_.String(L"scheme", MSIME_EDITION_DEFAULT_SCHEME_W);
    if (current == next)
      return;
    change([&](PreferencesDocument &doc) {
      doc.SetString(L"last_chinese_scheme", next);
      doc.SetString(L"scheme", next);
    }, true);
  }

  // 一族（全拼或双拼）的辅助码：fallback 是 client-core 的 default_quanpin_helpcode / default_shuangpin_helpcode，只在拿不到出厂偏好时用到。
  // 方案下拉框照共享的 HelpcodeSettingsGroup 排列：内置方案和资源目录里的自定义码表。插件已移除，不再列辅助码表插件。
  void helpcode_rows(StackPanel const &group, std::wstring const &label,
                     std::wstring const &family, bool enabled_fallback,
                     std::wstring const &schema_fallback, bool show_fallback) {
    const std::wstring prefix = family + L"_helpcode";
    const std::wstring schema_key = prefix + L".schema";
    const bool enabled = document_.Boolean(prefix + L".enabled", enabled_fallback);
    // 共享页的 Shift 说明只在 helpcode_shift_entry 的宿主（Android、HarmonyOS、iOS）上显示，Windows 上这一行没有说明。
    bool_row(group, 0xE8CB, label + L"辅助码", L"", prefix + L".enabled", enabled_fallback, true);

    const auto schema = document_.String(schema_key, schema_fallback);
    std::vector<Option> options{
        {L"lantian", L"蓝天小雨点"}, {L"ziranma", L"自然码"},
        {L"shouyou2_0", L"首右2.0"}, {L"shouyouplus", L"首右plus"},
        {L"xiaohe", L"小鹤"}, {L"jiajia", L"加加"}, {L"wubi86", L"五笔 86"}};
    options.insert(options.end(), helpcode_custom_schemas_.begin(),
                   helpcode_custom_schemas_.end());
    const auto listed = [&options](std::wstring const &value) {
      return std::any_of(options.begin(), options.end(),
                         [&value](Option const &option) { return option.value == value; });
    };
    // 资源目录被更换或暂时读不到时，之前选中的自定义码表仍留在列表里。
    if (schema.starts_with(L"custom/") && !listed(schema))
      options.push_back({schema, schema});

    const auto title = label + L"辅助码方案";
    add_row(group, 0xE8D2, title, L"",
            select_control(title, std::move(options), schema,
                           [this, schema_key](std::wstring const &value) {
                             change([&](PreferencesDocument &doc) {
                               doc.SetString(schema_key, value);
                             }, true);
                           },
                           enabled));
    // 共享页的这一行不随辅助码开关变灰。
    bool_row(group, 0xE8FD, L"在候选窗口中显示" + label + L"辅助码", L"",
             prefix + L".show_in_candidate_window", show_fallback);
    // 「一码严格筛选」随辅助码开关变灰，缺字段时为关（client-core `strict_filter` 的默认值）。
    bool_row(group, 0xE71C, label + L"一码严格筛选",
             std::wstring(msime::settings::helpcode::strict_filter_description),
             prefix + L".strict_filter", false, false, enabled);
  }

  // 双拼辅助码的触发方式（`shuangpin_helpcode.modes`），只画在当前方案是双拼时的主流程里，与共享页桌面形态相同：另一族收进「更多选项」时不带这几行。各行与缺值读法见 HelpcodeModesModel.h。互斥由 client-core 在写入端归一，所以每次改动都重画，界面显示归一后的值。
  void helpcode_mode_rows(StackPanel const &group) {
    namespace hm = msime::settings::helpcode;
    const std::wstring prefix = L"shuangpin_helpcode.modes.";
    const auto value = [this, &prefix](hm::Mode const &mode) {
      return document_.Boolean(prefix + std::wstring(mode.field), mode.fallback);
    };
    const auto find = [](std::wstring_view field) -> hm::Mode const & {
      return *std::find_if(hm::shuangpin_modes.begin(), hm::shuangpin_modes.end(),
                           [field](hm::Mode const &mode) { return mode.field == field; });
    };
    const bool helpcode_on = document_.Boolean(L"shuangpin_helpcode.enabled", true);
    const bool direct = value(find(L"direct"));
    const bool mid_sentence = value(find(L"mid_sentence"));
    const bool direct_helpcode = value(find(L"direct_helpcode"));
    for (auto const &mode : hm::shuangpin_modes)
      bool_row(group, 0xE8CB, L"双拼" + std::wstring(mode.title), std::wstring(mode.description),
               prefix + std::wstring(mode.field), mode.fallback, true,
               hm::mode_enabled(mode, helpcode_on, direct, mid_sentence, direct_helpcode));
  }

  // 自定义码表来源。自定义码表只读一次，与共享页一样；在 UI 线程之外读，读完内容有变化才重画。插件已移除，不再读取辅助码表插件。
  void load_helpcode_sources() {
    if (!helpcode_schemas_requested_) {
      helpcode_schemas_requested_ = true;
      run_host(helpcode_schemas_call, [this](Response const &response) {
        auto schemas = custom_helpcode_options(response);
        if (!same_options(schemas, helpcode_custom_schemas_)) {
          helpcode_custom_schemas_ = std::move(schemas);
          redraw_typing();
        }
      });
    }
  }

  static bool same_options(std::vector<Option> const &a, std::vector<Option> const &b) {
    return std::equal(a.begin(), a.end(), b.begin(), b.end(),
                      [](Option const &x, Option const &y) {
                        return x.value == y.value && x.label == y.label;
                      });
  }

  void redraw_typing() {
    if (!closed_ && current_page_ == "typing")
      render_page(true);
  }

  // ---- 标点与翻译 ----

  // 「标点与翻译」页照共享页 `ExpressionSettingsPage` 的桌面分支（`pages/expression-page.tsx`）分三个页内标签（`expression-tabs.ts`）：「标点设置」是「标点」组；「候选释义」是「多语言与释义」「候选词翻译」两组；「翻译服务」单独成组，下面接所选服务的设置组。混输、输出、快捷模式和模糊音在「输入」页。
  void build_fuzzy_group(StackPanel const &page) {
    auto group = add_group(page, L"模糊音");
    const bool enabled = document_.Boolean(L"fuzzy_pinyin.enabled", false);
    add_row(group, 0xE8D2, L"启用模糊音", L"全拼、九键与双拼均支持；更改会在当前输入结束后生效",
            toggle_control(L"启用模糊音", enabled, [this](bool on) {
              change([&](PreferencesDocument &doc) {
                doc.SetBoolean(L"fuzzy_pinyin.enabled", on);
                // The first time it is switched on every rule starts checked; afterwards the user's choice is kept, also while it is off.
                if (on && !doc.Boolean(L"fuzzy_pinyin.seeded", false)) {
                  std::vector<std::wstring> all;
                  for (auto const &rule : ism::fuzzy_rules)
                    all.emplace_back(rule.id);
                  doc.SetStrings(L"fuzzy_pinyin.rules", std::move(all));
                  doc.SetBoolean(L"fuzzy_pinyin.seeded", true);
                }
              }, true);
            }));
    if (!enabled)
      return;
    const auto rules = document_.Strings(L"fuzzy_pinyin.rules");
    explain_group(group, L"勾选容易混淆的读音后，会补充对应候选。关闭总开关会保留已选规则。");
    StackPanel block;
    block.Spacing(8);
    for (size_t first = 0; first < ism::fuzzy_rules.size();) {
      const auto title = ism::fuzzy_rules[first].group;
      auto heading = make_text(title, 13, palette_.text);
      heading.FontWeight(Windows::UI::Text::FontWeight{600});
      heading.Margin(Thickness{0, 4, 0, 0});
      block.Children().Append(heading);
      std::vector<Check> checks;
      for (; first < ism::fuzzy_rules.size() && ism::fuzzy_rules[first].group == title; ++first) {
        const std::wstring id(ism::fuzzy_rules[first].id);
        const bool on = std::find(rules.begin(), rules.end(), id) != rules.end();
        checks.push_back({std::wstring(ism::fuzzy_rules[first].label), on, [this, id](bool checked) {
                            change([&](PreferencesDocument &doc) {
                              auto current = doc.Strings(L"fuzzy_pinyin.rules");
                              current.erase(std::remove(current.begin(), current.end(), id),
                                            current.end());
                              if (checked)
                                current.push_back(id);
                              doc.SetStrings(L"fuzzy_pinyin.rules", std::move(current));
                            }, false);
                          }});
      }
      block.Children().Append(checks_panel(std::move(checks), 120));
    }
    record(L"重置模糊音配置", L"");
    auto reset = button_control(L"重置模糊音配置", [this] { reset_fuzzy_pinyin(); }, loaded_);
    reset.HorizontalAlignment(HorizontalAlignment::Left);
    mark_search_target(L"重置模糊音配置", reset);
    block.Children().Append(reset);
    append_row_frame(group, block).Padding(Thickness{row_inset, 10, row_inset, 10});
  }

  // 不能再撤回的操作前先问一句。
  Windows::Foundation::IAsyncOperation<bool> confirm(std::wstring title, std::wstring message, std::wstring action) {
    ContentDialog dialog;
    dialog.Title(box_value(hstring(title)));
    dialog.Content(box_value(hstring(message)));
    dialog.PrimaryButtonText(hstring(action));
    dialog.CloseButtonText(L"取消");
    dialog.DefaultButton(ContentDialogButton::Close);
    dialog.XamlRoot(root_.XamlRoot());
    dialog.RequestedTheme(palette_.dark ? ElementTheme::Dark : ElementTheme::Light);
    co_return co_await show_dialog(dialog);
  }

  // 用户点了对话框的主按钮时返回 true。WinUI 同一时间只显示一个 ContentDialog，再开第二个会抛异常，在 fire_and_forget 协程里会直接结束进程；已经有一个对话框时再点，比如连按两次删除，按取消处理。
  Windows::Foundation::IAsyncOperation<bool> show_dialog(ContentDialog dialog) {
    if (dialog_open_)
      co_return false;
    dialog_open_ = true;
    bool accepted = false;
    try {
      accepted = co_await dialog.ShowAsync() == ContentDialogResult::Primary;
    } catch (...) {
    }
    dialog_open_ = false;
    co_return accepted;
  }

  // 共享页的重置（resetFuzzyPinyin）：先问一句，确认后关掉模糊音并清空全部规则；seeded 不动，所以再打开时不会重新勾上全部规则。
  fire_and_forget reset_fuzzy_pinyin() {
    auto strong = get_strong();
    if (!loaded_)
      co_return;
    if (!co_await confirm(L"关闭模糊音", L"所有模糊音规则会被清空。", L"关闭并清空"))
      co_return;
    change([](PreferencesDocument &doc) {
      doc.SetBoolean(L"fuzzy_pinyin.enabled", false);
      doc.SetStrings(L"fuzzy_pinyin.rules", std::vector<std::wstring>{});
    }, true);
  }

  // 「全拼纠错」/「双拼纠错」组，照共享页 input-page.tsx 高级选项里模糊音之后的那一组：只在当前方案是全拼或双拼时画，标题和说明按当前方案写，文案与共享页逐字相同。全拼和双拼都读 quanpin 下的 autocorrect_neighbor、autocorrect_transposition（字段名带 quanpin 是历史原因，见 shuangpin::engine::request_autocorrect_types），默认都开。全拼两个都关掉时漏打、多打字母的纠错也一起关闭（request_autocorrect_mask），双拼没有这一类纠错，所以只有全拼的说明写这一句。五笔的混输拼音和 z 键拼音反查也带着这两个字段，但与共享页一样不为它们单列，其他方案不纠错，不画。
  void build_autocorrect_group(StackPanel const &page) {
    const auto scheme = document_.String(L"scheme", MSIME_EDITION_DEFAULT_SCHEME_W);
    if ((scheme != L"quanpin" && scheme != L"shuangpin") || !edition_offers_scheme(scheme))
      return;
    const bool shuangpin = scheme == L"shuangpin";
    auto group = add_group(page, shuangpin ? L"双拼纠错" : L"全拼纠错");
    bool_row(group, 0xE8D2, shuangpin ? L"双拼邻键纠错" : L"全拼邻键纠错",
             shuangpin
                 ? L"把按偏到相邻键的双拼编码纠回来，只用于三个音节以上的整句，纠错整句排在词条和整句后面。关闭后不再按邻键猜测编码。"
                 : L"把按偏到相邻键的字母纠回来，例如 shabg → shang、zhr → zhe。全是辅音的输入（qjn、zhr）先列简拼词，纠错候选排在后面。关闭后不再按邻键猜测拼写。",
             L"quanpin.autocorrect_neighbor", true);
    bool_row(group, 0xE8D2, shuangpin ? L"双拼乱序纠错" : L"全拼乱序纠错",
             shuangpin
                 ? L"把两键编码前后颠倒的音节纠回来，同样只用于三个音节以上的整句。关闭后不再按乱序猜测编码。"
                 : L"把前后颠倒的两个字母纠回来，例如 hsang → shang、gau → gua。关闭后不再按乱序猜测拼写。两个开关都关闭时，漏打、多打字母的纠错（shng → shang）也一起关闭。",
             L"quanpin.autocorrect_transposition", true);
  }

  static bool helpcode_offered() {
    return edition_offers_scheme(L"quanpin") || edition_offers_scheme(L"shuangpin");
  }

  // 「辅助码」组，照共享页 HelpcodeSettingsGroup 的桌面形态：本版本提供全拼或双拼之一就画（共享页的 editionUsesHelpcode），不看当前方案。主流程是当前方案所属那一族的四行（双拼方案用双拼，其他方案用全拼），双拼方案还接着画双拼辅助码的触发方式；另一族的四行收在组末的「更多选项」里，不带触发方式；搜索落到那几行时展开。
  void build_helpcode_group(StackPanel const &page) {
    if (!helpcode_offered())
      return;
    if (!indexing_)
      load_helpcode_sources();
    const bool shuangpin =
        document_.String(L"scheme", MSIME_EDITION_DEFAULT_SCHEME_W) == L"shuangpin";
    const auto family = [this](StackPanel const &rows, bool double_pinyin) {
      if (double_pinyin)
        helpcode_rows(rows, L"双拼", L"shuangpin", true, L"lantian", true);
      else
        helpcode_rows(rows, L"全拼", L"quanpin", true, L"ziranma", false);
    };
    auto helpcode = add_group(page, L"辅助码");
    family(helpcode, shuangpin);
    if (shuangpin)
      helpcode_mode_rows(helpcode);
    auto more = add_fold(
        helpcode, L"更多选项", helpcode_more_open_ || fold_holds_focus(L"helpcode-more"),
        [this](bool expanded) { helpcode_more_open_ = expanded; });
    fold_index_ = L"helpcode-more";
    family(more, !shuangpin);
    fold_index_.clear();
  }

  // ---- 快捷键 ----

  // 「快捷键」页照共享页 ShortcutsSettingsSection 的桌面分支分两个页内标签（InputSettingsModel.h 的 shortcut_tabs）：「按键设置」是页首说明和「输入模式切换」组；「按键速查」是只读的键帽行。翻页方式和以词定字在「输入 › 选词与翻页」。与 macOS 一样，桌面上没有「恢复默认快捷键」。
  void build_shortcuts_page(StackPanel const &page) {
    std::vector<Option> tabs;
    for (auto const &tab : ism::shortcut_tabs)
      tabs.push_back({std::wstring(tab.id), std::wstring(tab.title)});
    const auto tab = page_tab_bar(page, L"按键设置分类", tabs, L"shortcuts-tab");
    if (in_tab(tab, L"settings"))
      build_shortcut_settings(page);
    if (in_tab(tab, L"reference"))
      build_shortcut_reference(page);
  }

  void build_shortcut_settings(StackPanel const &page) {
    // 页首说明（共享页 ShortcutsIntroSection 的 PageIntro）：12 DIP 次要色，左右缩进与组名对齐。
    if (!indexing_) {
      auto intro = make_text(
          L"输入法快捷键仅在对应输入状态或候选窗口显示时生效。翻页方式和以词定字在「输入 › 选词与翻页」中设置。",
          12, palette_.sub);
      intro.Margin(Thickness{8, 20, 8, 0});
      page.Children().Append(intro);
    }
    auto keys = add_group(page, L"输入模式切换");
    explain_group(keys, L"在当前输入上下文中切换中英文模式；未选用或关闭的快捷键会交给应用处理。");
    // 偏好里仍是三个独立的布尔值，这里和共用设置页一样只让选一个：同时开着多个时按 Shift > 单击 Ctrl > Ctrl+Alt+Space 显示第一个，选中一项时一次写全三个，「不使用」三个都关。
    const std::array<std::pair<const wchar_t *, const wchar_t *>, 3> switches{{
        {L"shift", L"keybindings.switch_language_shift"},
        {L"ctrl", L"keybindings.switch_language_ctrl"},
        {L"ctrl_alt_space", L"keybindings.switch_language_ctrl_alt_space"},
    }};
    std::wstring language_switch = L"none";
    for (const auto &[choice, key] : switches) {
      if (document_.Boolean(key, false)) {
        language_switch = choice;
        break;
      }
    }
    add_row(keys, 0xE8AB, L"切换中英文", L"",
            select_control(L"切换中英文",
                           {{L"shift", L"Shift"}, {L"ctrl", L"单击 Ctrl"},
                            {L"ctrl_alt_space", L"Ctrl+Alt+Space"}, {L"none", L"不使用"}},
                           language_switch,
                           [this, switches](std::wstring const &value) {
                             change([&](PreferencesDocument &doc) {
                               for (const auto &[choice, key] : switches)
                                 doc.SetBoolean(key, value == choice);
                             }, false);
                           }));
    // 和上面的切换键互不排斥：Caps Lock 是另加的一个键。说明与共享页 capsLockLanguageSwitchDescription 给 Windows 的相同。
    bool_row(keys, 0xE8AB, L"单按 Caps Lock 切换中英文",
             L"轻点 Caps Lock（300 毫秒内松开）切换中英文，不打开大写锁定；按住更久照常打开大写锁定，大写锁定开着时按一下照常关掉。",
             L"keybindings.switch_language_caps_lock", false);
    // 日语方案已移除，不再有 Ctrl+Shift+J 中英日切换。
    bool_row(keys, 0xE8C1, L"Ctrl+Shift+F 切换繁体输出", L"",
             L"keybindings.toggle_character_set_ctrl_shift_f", true);
    // 说明照共享页，Windows 另有系统的 Ctrl+Shift+Space 全半角切换，也不受这个开关影响，补在后半句里。
    bool_row(keys, 0xE8D3, L"Alt+Shift+H 切换全半角",
             L"关掉后这个组合键交给应用处理；工具栏的全半角开关和 Ctrl+Shift+Space 不受影响。",
             L"keybindings.toggle_fullwidth_option_shift_h", true);
    // Ctrl+Space 是 Windows 自己的快捷键，这里只说明去哪里改，压成组末一行（共享页 InputModeShortcutsSection 的 windows 一支）。
    add_row(keys, 0xE765, L"Ctrl+Space（由 Windows 管理）",
            L"此处不控制。要修改或关闭，打开「设置 › 时间和语言 › 输入 › 高级键盘设置 › 输入语言热键」，选中「中文（简体）输入法 - 输入法 / 非输入法切换」，点「更改按键顺序」后关闭它或改成不常用的组合。不同 Windows 版本的名称可能略有差异；未立即生效时请重新登录或重启电脑。",
            nullptr);
  }

  // 「按键速查」组（共享页 CandidateShortcutReference 加 PanelShortcutRow）：候选窗口显示时各键做什么，只读。翻页的几组按现在生效的列出（Tab 和 PageUp / PageDown 固定算开），只有一组时和其他行一样放在行尾，多组时排到标题下方；鼠标滚轮、上下键、左右键和以词定字只在开着时列出。Windows 没有「数字键选词」开关，数字键总能选词。
  void build_shortcut_reference(StackPanel const &page) {
    auto group = add_group(page, L"按键速查");
    add_row(group, 0xE765, L"选择候选", L"", key_cap(L"Space 或 1–9"));
    std::vector<std::wstring> paging;
    for (auto const &keys : ism::paging_keys) {
      const bool stored = document_.Boolean(L"navigation." + std::wstring(keys.id),
                                            keys.id != L"brackets");
      if (ism::paging_keys_on(keys, stored))
        paging.emplace_back(keys.chord);
    }
    if (paging.size() == 1) {
      add_row(group, 0xE765, L"向前 / 向后翻页", L"", key_cap(paging.front()));
    } else if (paging.size() > 1) {
      // 照共享页 settings.shortcutKeys：键帽按自身宽度（至少 120）靠左排、相隔 6 DIP，一行放不下时折行。WinUI 没有按子项宽度折行的面板，这里用 Auto 列的 responsive_grid，每列按最宽的「Page Up / Page Down」约 140 DIP 估算列数；键帽靠左，不随列宽拉伸。
      auto caps = responsive_grid(140, 6, 0, true);
      caps.Margin(Thickness{0, 6, 0, 0});
      for (auto const &chord : paging) {
        auto cap = key_cap(chord);
        cap.HorizontalAlignment(HorizontalAlignment::Left);
        caps.Children().Append(cap);
      }
      add_row(group, 0xE765, L"向前 / 向后翻页", L"", nullptr, caps);
    }
    if (document_.Boolean(L"navigation.mouse_wheel", false))
      add_row(group, 0xE765, L"候选窗口翻页", L"", key_cap(L"鼠标滚轮"));
    if (document_.Boolean(L"navigation.arrows", true))
      add_row(group, 0xE765, L"移动候选项", L"", key_cap(L"↑ / ↓"));
    // 打开左右键移动候选时，左右键先编辑输入串，光标到了这一端才移高亮。
    if (document_.Boolean(L"navigation.left_right", true))
      add_row(group, 0xE765, L"移动候选项（输入串两端）", L"", key_cap(L"← / →"));
    if (document_.Boolean(L"word_character.enabled", false))
      add_row(group, 0xE765, L"以词定字（上屏首字 / 末字）", L"",
              key_cap(document_.String(L"word_character.keys", L"brackets") == L"minus_equal"
                          ? L"- / ="
                          : L"[ / ]"));
    add_row(group, 0xE765, L"移动到候选列表首项 / 末项（页码随之切换）", L"",
            key_cap(L"Home / End"));
    add_row(group, 0xE765, L"编辑输入串", L"", key_cap(L"← / → / Backspace"));
    add_row(group, 0xE765, L"提交原始输入 / 取消输入", L"", key_cap(L"Enter / Esc"));
    // Server 的维护快捷键（MaintenanceHotkeys.h）在任何输入上下文里都能打开屏幕键盘。
    add_row(group, 0xE765, L"打开屏幕键盘", L"可从当前输入上下文使用 Win 组合键打开面板。",
            key_cap(L"Ctrl+Shift+Win+K"));
  }

  // 按键速查里的一个键帽，照共享页的 SettingsShortcutKey（`settings-style.ts` 的 shortcutKey）：至少 120 宽、圆角 5、1 DIP 描边，次要按钮的底色，12 DIP 正文色的字居中。
  Border key_cap(std::wstring const &chord) {
    Border cap;
    cap.MinWidth(120);
    cap.CornerRadius(CornerRadius{5, 5, 5, 5});
    cap.BorderThickness(Thickness{1, 1, 1, 1});
    cap.BorderBrush(brush(palette_.dark ? rgba(255, 255, 255, .08) : rgba(0, 0, 0, .08)));
    cap.Background(brush(palette_.dark ? rgba(0x3B, 0x3B, 0x3B, 1) : rgba(0xF0, 0xF0, 0xF0, 1)));
    cap.Padding(Thickness{8, 4, 8, 4});
    auto label = make_text(chord, 12, palette_.text);
    label.TextAlignment(TextAlignment::Center);
    label.HorizontalAlignment(HorizontalAlignment::Center);
    cap.Child(label);
    return cap;
  }

  // ---- 关于 ----

  // 按场景缓存的水杉娘插画（load_mascot），与页首共用 `mascot_bitmaps_`。
  Imaging::BitmapImage mascot_bitmap(nav::MascotScene scene) {
    auto bitmap = mascot_bitmaps_.find(scene);
    if (bitmap == mascot_bitmaps_.end())
      bitmap = mascot_bitmaps_.emplace(scene, load_mascot(scene)).first;
    return bitmap->second;
  }

  // 「关于」页：版本、系统信息、许可与隐私三个页内标签（page_tab_bar，「关于设置分类」），一次只画一个。「关于」没有页首引导，标签条就是正文的第一样东西。
  void build_about_page(StackPanel const &page) {
    const auto tab = page_tab_bar(page, L"关于设置分类",
                                  {{L"version", L"版本"},
                                   {L"system", L"系统信息"},
                                   {L"privacy", L"许可与隐私"}},
                                  L"about-tab");
    if (!indexing_ && page.Children().Size() > 0)
      page.Children().GetAt(page.Children().Size() - 1).as<FrameworkElement>().Margin(Thickness{});
    if (in_tab(tab, L"version"))
      build_about_version(page);
    if (in_tab(tab, L"system"))
      build_system_info(page);
    if (in_tab(tab, L"privacy"))
      build_about_privacy(page);
  }

  // 「版本」标签：无标题分组里的品牌卡（共享页的 `AboutHeroSection`），然后是「版本」组。品牌卡左边是 62 DIP、圆角 16、浅底的标志块，里面 48 DIP 的产品图标；中间依次是 12 DIP、字距 0.04em 的「Metasequoia IME」，22 DIP 半粗体的产品名，相隔 7 DIP 的一句 12 DIP 说明；右边是 112 DIP 的 welcome 插画。
  void build_about_version(StackPanel const &page) {
    if (!indexing_) {
      auto card = make_card();
      // 与标签条相隔 24：组自己的上边距是 20，再加页面的 Spacing 4。
      card.Margin(Thickness{0, 20, 0, 0});
      Grid hero;
      hero.Padding(Thickness{20, 20, 20, 20});
      hero.ColumnSpacing(18);
      ColumnDefinition mark_column;
      mark_column.Width(GridLength{1, GridUnitType::Auto});
      ColumnDefinition text_column;
      text_column.Width(GridLength{1, GridUnitType::Star});
      ColumnDefinition mascot_column;
      mascot_column.Width(GridLength{1, GridUnitType::Auto});
      hero.ColumnDefinitions().Append(mark_column);
      hero.ColumnDefinitions().Append(text_column);
      hero.ColumnDefinitions().Append(mascot_column);
      Border mark;
      mark.Width(62);
      mark.Height(62);
      mark.CornerRadius(CornerRadius{16, 16, 16, 16});
      mark.Background(brush(palette_.subtle));
      mark.VerticalAlignment(VerticalAlignment::Center);
      Image logo;
      logo.Width(48);
      logo.Height(48);
      logo.HorizontalAlignment(HorizontalAlignment::Center);
      logo.VerticalAlignment(VerticalAlignment::Center);
      show_logo(logo, 128);
      A11y::SetName(logo, L"水杉 IME");
      mark.Child(logo);
      hero.Children().Append(mark);
      StackPanel names;
      names.VerticalAlignment(VerticalAlignment::Center);
      auto eyebrow = make_text(L"Metasequoia IME", 12, palette_.sub);
      eyebrow.CharacterSpacing(40);
      names.Children().Append(eyebrow);
      auto product = make_text(MSIME_EDITION_DISPLAY_NAME, 22, palette_.text);
      product.FontWeight(Windows::UI::Text::FontWeight{600});
      product.Margin(Thickness{0, 2, 0, 0});
      A11y::SetAutomationId(product, L"settings-page-about");
      names.Children().Append(product);
      auto description = make_text(L"为现代 Windows 桌面体验打造的开放中文输入法。", 12, palette_.sub);
      description.Margin(Thickness{0, 7, 0, 0});
      names.Children().Append(description);
      Grid::SetColumn(names, 1);
      hero.Children().Append(names);
      Image mascot;
      mascot.Width(112);
      mascot.Height(112);
      mascot.Stretch(Stretch::Uniform);
      mascot.VerticalAlignment(VerticalAlignment::Center);
      mascot.Source(mascot_bitmap(nav::MascotScene::welcome));
      // 纯装饰，与共享 UI 插画的 aria-hidden 相同。
      A11y::SetAccessibilityView(mascot, Microsoft::UI::Xaml::Automation::Peers::AccessibilityView::Raw);
      Grid::SetColumn(mascot, 2);
      hero.Children().Append(mascot);
      card.Child(hero);
      page.Children().Append(card);
    }

    auto updates = add_group(page, L"版本");
    add_row(updates, 0xE895, L"当前版本", app_version_label(), nullptr);
    url_row(updates, L"其他平台下载", L"在水杉输入法官网的下载页获取其他平台的版本。",
            download_url);
    url_row(updates, L"历史版本", L"各版本的更新说明与安装包。", releases_url);
  }

  // 「系统信息」各项（AboutPageModel.h）：本版本号，非完整版另写版本名，有发布日期时带上；`RtlGetVersion` 的系统版本；本程序的处理器架构；偏好读到时再加当前的输入方案。
  std::vector<msime::settings::SystemInfoEntry> current_system_info() {
#if MSIME_EDITION_IS_FULL
    constexpr std::wstring_view edition_name;
#else
    constexpr std::wstring_view edition_name = MSIME_EDITION_DISPLAY_NAME;
#endif
    const std::wstring scheme =
        loaded_ ? document_.String(L"scheme", MSIME_EDITION_DEFAULT_SCHEME_W) : std::wstring();
    return msime::settings::system_info_entries(
        app_version, edition_name, app_release_date, windows_os_version(),
        msime::settings::build_architecture(),
        scheme.empty() ? std::wstring_view() : msime::settings::feedback_scheme_title(scheme));
  }

  // 「系统信息」标签（共享页的 `SystemInfoSection`）：逐项一行，值在行尾，次要色、右对齐、可以选中；最后一行「复制系统信息」把同样的内容按「名称：值」每行一项放进剪贴板，复制成功后按钮显示「已复制」1.6 秒（mark_copied）。
  void build_system_info(StackPanel const &page) {
    auto group = add_group(page, L"系统信息");
    for (auto const &entry : current_system_info()) {
      auto value = make_text(entry.value, 12, palette_.sub);
      value.IsTextSelectionEnabled(true);
      value.TextAlignment(TextAlignment::Right);
      value.MaxWidth(280);
      add_row(group, 0xE946, entry.label, L"", value);
    }
    auto copy = button_control(system_info_copied_ ? L"已复制" : L"复制", [this] {
      if (copy_text(msime::settings::system_info_text(current_system_info())))
        mark_copied(system_info_copied_, system_info_timer_);
      else
        show_notice(L"复制失败，请重试");
    });
    A11y::SetName(copy, L"复制系统信息");
    add_row(group, 0xE8C8, L"复制系统信息", L"反馈问题时贴进去，方便排查；不含账号和输入内容", copy);
    if (!indexing_)
      system_info_button_ = copy.as<Button>();
  }

  // 复制后按钮显示「已复制」1.6 秒再复原，与共享页相同；再点一次重新计时。标记跨页面重建保留，新画的按钮照样显示。
  void mark_copied(bool &copied, Microsoft::UI::Dispatching::DispatcherQueueTimer &timer) {
    copied = true;
    relabel_copied_buttons();
    if (!timer) {
      timer = DispatcherQueue().CreateTimer();
      timer.Interval(std::chrono::milliseconds(1600));
      timer.IsRepeating(false);
      timer.Tick([this, &copied](Microsoft::UI::Dispatching::DispatcherQueueTimer const &,
                                 Inspectable const &) {
        copied = false;
        relabel_copied_buttons();
      });
    }
    timer.Stop();
    timer.Start();
  }

  void relabel_copied_buttons() {
    if (system_info_button_)
      system_info_button_.Content(box_value(system_info_copied_ ? L"已复制" : L"复制"));
  }

  // 「许可与隐私」标签：开源许可协议、许可与版权、第三方组件许可、隐私政策。匿名使用统计已随遥测一并移除。
  void build_about_privacy(StackPanel const &page) {
    auto privacy = add_group(page, L"许可与隐私");
    url_row(privacy, L"开源许可协议", L"", license_url);
    auto copyright = make_text(L"© 2026 Metasequoia IME", 12, palette_.sub);
    A11y::SetName(copyright, L"版权");
    add_row(privacy, 0xE8A5, L"许可与版权",
            L"水杉 IME 以 GPL-3.0 发布；第三方组件许可随应用资源提供。", copyright);
    link_row(privacy, L"第三方组件许可", L"", true, [this] { open_third_party_notices(); });
    url_row(privacy, L"隐私政策", L"", privacy_url);
  }

  PreferencesDocument document_;
  bool loaded_ = false;
  std::wstring load_error_;
  Windows::UI::ViewManagement::UISettings ui_settings_;
  bool system_dark_ = false;
  InputMethodState input_method_ = InputMethodState::unknown;
  Palette palette_;

  std::string current_page_;
  bool selecting_ = false;
  bool refresh_queued_ = false;
  bool closed_ = false;
  // 有 ContentDialog 正在显示；见 show_dialog。
  bool dialog_open_ = false;
  bool pending_save_ = false;
  // 写盘连续失败的次数；大于零时文档里有没存下的修改，见 keep_unsaved_edit。
  std::size_t pending_save_failures_ = 0;
  bool preview_visible_ = true;
  std::wstring preview_sample_;
  std::optional<bool> preview_dark_;
  // 正在索引的折叠区，以及索引时记下的每行所在的折叠区，见 fold_holds_focus。
  std::wstring fold_index_;
  std::map<std::wstring, std::wstring> search_folds_;
  // 每页选中的页内标签（页面 id → 标签 id），跨页面保留、不写偏好；以及建索引时正在画的标签。见 page_tab_bar。
  std::map<std::string, std::wstring> page_tabs_;
  std::wstring tab_index_;
  // 建索引时这一页的标签（取标签名）和正在画的分组标题，写进搜索结果的位置，见 record。
  std::vector<Option> index_tabs_;
  std::wstring group_index_;
  // add_group 建的有组名的组：行面板、组名一行和卡片，explain_group 按行面板找到组名一行挂「?」。每次 build_page 清空。
  struct GroupHeader {
    StackPanel rows{nullptr};
    StackPanel header{nullptr};
    Border card{nullptr};
  };
  std::vector<GroupHeader> group_headers_;
  // 「候选字体」的「自定义…」状态，跨重画保留。
  bool candidate_font_custom_ = false;
  std::wstring notice_;
  InfoBarSeverity notice_severity_ = InfoBarSeverity::Error;

  std::vector<SearchEntry> search_index_;
  bool indexing_ = false;
  bool index_valid_ = false;
  uint64_t index_revision_ = 0;
  std::wstring focus_row_;
  // 侧栏结果列表现在对应的查询、结果、高亮的那条和每条的按钮，见 update_search_results。
  std::wstring search_query_;
  std::vector<SearchEntry> search_hits_;
  size_t search_active_ = 0;
  std::vector<Button> search_buttons_;
  bool search_composing_ = false;
  FrameworkElement focus_target_{nullptr};

  // 「输入」页「候选与联想」「逐个模式」「辅助码」的「更多选项」是否展开，跨页面重建保留。
  bool candidates_more_open_ = false;
  bool local_modes_more_open_ = false;
  bool helpcode_more_open_ = false;

  // 辅助码自定义码表：读到过的下拉选项和是否已经请求过。
  std::vector<Option> helpcode_custom_schemas_;
  bool helpcode_schemas_requested_ = false;

  // 「系统信息」的复制按钮和「已复制」计时器。
  bool system_info_copied_ = false;
  Button system_info_button_{nullptr};
  Microsoft::UI::Dispatching::DispatcherQueueTimer system_info_timer_{nullptr};

  // 重启输入法进程时防重入。
  bool input_method_restarting_ = false;

  // 系统字体列表：读取状态和结果跨页面重建保留，与共享页一样第一次点开字体输入框时才读取。控件是主题页当前的那一组。
  enum class FontCatalogStatus { idle, loading, ready, failed };
  FontCatalogStatus font_catalog_status_ = FontCatalogStatus::idle;
  std::vector<std::wstring> font_catalog_;
  uint64_t font_catalog_generation_ = 0;
  TextBlock font_catalog_text_{nullptr};
  Button font_catalog_button_{nullptr};
  StackPanel font_preset_host_{nullptr};
  std::vector<std::function<void()>> font_menu_updaters_;

  Grid root_{nullptr};
  Grid titlebar_{nullptr};
  TextBox search_{nullptr};
  ScrollViewer search_scroll_{nullptr};
  StackPanel search_list_{nullptr};
  TextBlock toolbar_title_{nullptr};
  // 当前页页首引导的那一行、说明和插画；没有引导的页面（「关于」）为空。
  Grid intro_row_{nullptr};
  TextBlock intro_text_{nullptr};
  Image intro_mascot_{nullptr};
  // 页首插画按场景解码一次后留着，每次重画页首（切标签、改设置后的刷新）直接挂上，不重新解码。
  std::map<nav::MascotScene, Imaging::BitmapImage> mascot_bitmaps_;
  NavigationView navigation_{nullptr};
  ScrollViewer scroll_{nullptr};
  Border preview_host_{nullptr};
  Grid preview_frame_{nullptr};
  Border toolbar_preview_host_{nullptr};
  TextBlock toolbar_preview_label_{nullptr};
  Microsoft::UI::Dispatching::DispatcherQueueTimer save_timer_{nullptr};
  Microsoft::UI::Dispatching::DispatcherQueueTimer save_retry_timer_{nullptr};
  // A redraw held back until the click that activated the window has been delivered.
  bool pointer_down_ = false;
  bool refresh_after_pointer_ = false;
  Microsoft::UI::Dispatching::DispatcherQueueTimer pointer_timer_{nullptr};
};

// Without a XAML compiler the application supplies the metadata provider itself; the WinUI controls' own provider answers for every type this window uses.
struct App : ApplicationT<App, Markup::IXamlMetadataProvider> {
  explicit App(msime::settings::SettingsInstance &instance) : instance_(instance) {}
  void OnLaunched(LaunchActivatedEventArgs const &) {
    Resources().MergedDictionaries().Append(XamlControlsResources());
    auto main = make_self<MainWindow>(route_page());
    window_ = main.as<Window>();
    HWND handle = nullptr;
    check_hresult(window_.as<::IWindowNative>()->get_WindowHandle(&handle));
    if (!instance_.listen(handle, [weak = main->get_weak()](std::string_view route) {
          const auto window = weak.get();
          return window && window->open_route(route);
        }))
      throw std::runtime_error("Settings activation unavailable");
    window_.Activate();
  }

  Markup::IXamlType GetXamlType(Windows::UI::Xaml::Interop::TypeName const &type) {
    return provider_.GetXamlType(type);
  }

  Markup::IXamlType GetXamlType(hstring const &name) {
    return provider_.GetXamlType(name);
  }

  com_array<Markup::XmlnsDefinition> GetXmlnsDefinitions() {
    return provider_.GetXmlnsDefinitions();
  }

private:
  msime::settings::SettingsInstance &instance_;
  XamlTypeInfo::XamlControlsXamlMetaDataProvider provider_;
  Window window_{nullptr};
};

} // namespace

int WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR command, int) {
  const int gameProfileResult = msime::windows::game_profile::run_if_requested(command);
  if (gameProfileResult >= 0) return gameProfileResult;
  const auto route = msime::windows::requested_surface_route("settings");
  if (!route) return 2;
  msime::settings::SettingsInstance instance;
  const auto activation = instance.start(*route);
  if (activation != msime::settings::SettingsInstance::Start::primary)
    return activation == msime::settings::SettingsInstance::Start::forwarded ? 0 : 1;
  init_apartment(apartment_type::single_threaded);
  Application::Start([&instance](auto &&) { make<App>(instance); });
  return 0;
}
