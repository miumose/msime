#include "ShellSurfaces.h"
#include <chrono>
#include <cstdio>
#include <fstream>
#include <stdexcept>
#include <string>

using namespace msime::windows;
constexpr wchar_t settings_file[] = L"msime-client-settings.exe";
void require_at(bool value, int line) {
  if (!value)
    throw std::runtime_error("Shell surface test failed at line " +
                             std::to_string(line));
}
#define require(...) require_at((__VA_ARGS__), __LINE__)
namespace {
// The block is double-NUL terminated, so entries are compared one at a time.
std::vector<std::wstring> entries(const std::wstring &block) {
  std::vector<std::wstring> result;
  for (size_t start = 0; start < block.size() && block[start];) {
    const auto end = block.find(L'\0', start);
    result.emplace_back(block, start, end - start);
    start = end + 1;
  }
  return result;
}
bool contains(const std::vector<std::wstring> &values, const std::wstring &entry) {
  for (const auto &value : values)
    if (value == entry)
      return true;
  return false;
}
} // namespace
int main() {
  try {
    // 精简版只保留设置窗口：设置、关于和主题页都落到同一个程序，其余行由 Server 自己处理。
    const auto settings = shell_surface_request(TrayMenuCommand::OpenSettings);
    require(settings && settings->panel.empty() && settings->page.empty());
    const auto about = shell_surface_request(TrayMenuCommand::OpenAbout);
    require(about && about->panel.empty() && about->page == "about");
    const auto theme = shell_surface_request(TrayMenuCommand::OpenTheme);
    require(theme && theme->panel.empty() && theme->page == "candidate");
    require(shell_route_argument(*theme) == L"settings:candidate");
    for (auto command :
         {TrayMenuCommand::ToggleFloatingToolbar, TrayMenuCommand::SelectChinese,
          TrayMenuCommand::SelectEnglish, TrayMenuCommand::ToggleFullwidth,
          TrayMenuCommand::ToggleChinesePunctuation,
          TrayMenuCommand::ToggleDedicatedEnglish,
          TrayMenuCommand::ToggleTraditionalOutput,
          TrayMenuCommand::SelectQuanpin, TrayMenuCommand::SelectShuangpin,
          TrayMenuCommand::OpenSystemEmoji,
          TrayMenuCommand::HideFloatingToolbar, TrayMenuCommand::SelectTheme,
          TrayMenuCommand::ShowSchemes, TrayMenuCommand::ShowThemes,
          TrayMenuCommand::ShowMain})
      require(!shell_surface_request(command));
    // 面板宿主的历史取值仍能解析，但本 fork 不带面板宿主，面板请求没有可执行的程序。
    require(panel_host_from_setting(L"native") == PanelHost::Native);
    require(panel_host_from_setting(L"Native") == PanelHost::Native);
    require(panel_host_from_setting(L"NATIVE") == PanelHost::Native);
    for (const wchar_t *value : {L"", L"shared", L"nativ", L"natives", L" native", L"native ", L"1", L"tauri"})
      require(panel_host_from_setting(value) == PanelHost::Shared);
    require(std::wstring_view(panel_host_variable) == L"MSIME_WINDOWS_PANEL_HOST");
    for (auto host : {PanelHost::Shared, PanelHost::Native})
      require(shell_executable_names(*settings, host) ==
              std::vector<std::wstring>{settings_file});
    require(shell_executable_names(ShellSurfaceRequest{"emoji", {}}, PanelHost::Shared).empty());
    require(shell_route_argument(*settings) == L"settings");
    require(shell_route_argument(*about) == L"settings:about");

    // A route replaces whatever this process inherited and leaves the rest of the environment, including drive current directories, alone.
    const std::wstring existing =
        std::wstring(L"PATH=C:\\Windows") + L'\0' +
        L"msime_client_route=stale" + L'\0' + L"=C:=C:\\work" + L'\0';
    const auto plain = entries(shell_environment_block(existing.c_str(), *settings));
    require(plain.size() == 3 && contains(plain, L"PATH=C:\\Windows"));
    require(contains(plain, L"=C:=C:\\work"));
    require(contains(plain, L"MSIME_CLIENT_ROUTE=settings"));
    require(!contains(plain, L"msime_client_route=stale"));
    const auto about_block = entries(shell_environment_block(existing.c_str(), *about));
    require(contains(about_block, L"MSIME_CLIENT_ROUTE=settings:about"));

    const ShellLaunchContext context{L"C:\\Users\\ime\\state",
                                    L"C:\\Users\\ime\\state\\runtime-options.json"};
    const std::wstring stale_paths =
        std::wstring(L"MSIME_CLIENT_STATE_DIR=C:\\stale") + L'\0' +
        L"MSIME_CLIENT_HOST_OPTIONS=C:\\stale\\runtime-options.json" + L'\0' +
        L"PATH=C:\\Windows" + L'\0';
    const auto configured_entries = entries(
        shell_environment_block(stale_paths.c_str(), *settings, &context));
    require(contains(configured_entries, L"MSIME_CLIENT_STATE_DIR=C:\\Users\\ime\\state"));
    require(contains(configured_entries,
                     L"MSIME_CLIENT_HOST_OPTIONS=C:\\Users\\ime\\state\\runtime-options.json"));
    require(!contains(configured_entries, L"MSIME_CLIENT_STATE_DIR=C:\\stale"));
    require(!contains(configured_entries,
                      L"MSIME_CLIENT_HOST_OPTIONS=C:\\stale\\runtime-options.json"));

    // Nothing but a short lowercase identifier may reach the child.
    for (const char *invalid : {"emoji panel", "Emoji", "emoji=1", "../etc"}) {
      bool rejected = false;
      try {
        (void)shell_environment_block(L"\0", ShellSurfaceRequest{invalid, {}});
      } catch (const std::invalid_argument &) {
        rejected = true;
      }
      require(rejected);
    }

    // 设置窗口只接受实际存在的文件；没有面板宿主时面板请求找不到程序。
    const auto root =
        std::filesystem::temp_directory_path() /
        ("msime-shell-fixture-" +
         std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
    std::filesystem::create_directories(root);
    require(!shell_executable(root, {}, *settings, PanelHost::Shared));
    require(!shell_executable("relative", {}, *settings, PanelHost::Shared));
    std::ofstream(root / settings_file) << "fixture";
    require(shell_executable(
                root, {}, *shell_surface_request(TrayMenuCommand::OpenSettings),
                PanelHost::Shared) == root / settings_file);
    require(!shell_executable(root, {}, ShellSurfaceRequest{"emoji", {}},
                              PanelHost::Shared));
    const auto configured_shell = root / "configured-settings.exe";
    std::ofstream(configured_shell) << "fixture";
    require(shell_executable(root, configured_shell.wstring(), *settings,
                             PanelHost::Shared) == configured_shell);
    require(!shell_executable(root, (root / "elsewhere.exe").wstring(), *settings,
                              PanelHost::Shared));
    require(!shell_executable(root, settings_file, *settings, PanelHost::Shared));

    // Every route also travels as the cross-platform string the shell parses.
    require(shell_surface_route(*settings) == "settings");
    require(shell_surface_route(*about) == "settings:about");
    require(shell_surface_route(*theme) == "settings:candidate");

    // The value filter still refuses anything outside the contract, and allows
    // at most the single separator a route needs.
    bool rejected = false;
    try {
      (void)shell_environment_block(existing.c_str(),
                                    ShellSurfaceRequest{"emoji&calc", {}});
    } catch (const std::invalid_argument &) {
      rejected = true;
    }
    require(rejected);
    rejected = false;
    try {
      (void)shell_environment_block(existing.c_str(),
                                    ShellSurfaceRequest{{}, "a:b"});
    } catch (const std::invalid_argument &) {
      rejected = true;
    }
    require(rejected);

    std::error_code error;
    std::filesystem::remove_all(root, error);
  } catch (const std::exception &error) {
    std::fputs(error.what(), stderr);
    std::fputs("\n", stderr);
    return 1;
  }

  return 0;
}
