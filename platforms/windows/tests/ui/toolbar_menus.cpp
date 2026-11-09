#include "ShellSurfaces.h"
#include "TrayMenuLayout.h"
#include <iostream>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

// 悬浮工具栏弹出的两个菜单：切换输入方案菜单和右键实用菜单，以及它们贴着工具栏打开的位置。
using namespace msime::windows;
namespace {
[[noreturn]] void require_failed(int line) {
  throw std::runtime_error("Toolbar menu check failed at line " + std::to_string(line));
}
#define require(value)                                                         \
  do {                                                                         \
    if (!(value))                                                              \
      require_failed(__LINE__);                                                \
  } while (false)
size_t find(const std::vector<TrayMenuItem> &items, TrayMenuCommand command) {
  for (size_t index = 0; index < items.size(); ++index)
    if (items[index].kind == TrayMenuRowKind::Item && items[index].command == command)
      return index;
  throw std::runtime_error("Toolbar menu row missing");
}
bool rejected(void (*action)()) {
  try {
    action();
  } catch (const std::invalid_argument &) {
    return true;
  }
  return false;
}
} // namespace

int main() {
  try {
    // 精简版的能力：只提供全拼和双拼，产品名写明，测试在哪个版本的构建里都查同一组行。
    TrayMenuCapabilities all;
    all.floating_toolbar = true;
    all.settings = true;
    all.schemes = scheme::OfferedSchemes{};
    all.schemes.offered[scheme::Quanpin] = true;
    all.schemes.offered[scheme::Shuangpin] = true;
    all.product_name = "水杉输入法";
    TrayMenuState state;
    state.scheme = "shuangpin";
    state.shuangpin_profile = "ziranma";

    // 切换输入方案菜单只列出全拼和双拼，名字与托盘一致，正在用的方案打勾，选中的行就是写进偏好的方案。
    {
      const auto schemes = tray_menu_scheme_items(all, state);
      const std::vector<std::pair<TrayMenuCommand, std::string>> expected{
          {TrayMenuCommand::SelectQuanpin, "全拼"},
          {TrayMenuCommand::SelectShuangpin, "双拼（自然码）"}};
      require(schemes.size() == expected.size());
      for (size_t index = 0; index < schemes.size(); ++index) {
        const auto &row = schemes[index];
        require(row.kind == TrayMenuRowKind::Item);
        require(row.command == expected[index].first && row.label == expected[index].second);
        require(row.available);
        require(row.checked == (row.command == TrayMenuCommand::SelectShuangpin));
        require(tray_menu_scheme(row.command) != nullptr);
        require(tray_menu_closes_after(row.command));
      }
      require(tray_menu_geometry(schemes, TrayMenuMetrics{}).rows.size() == schemes.size());
      // 悬浮工具栏的方案菜单像 macOS 一样只列出能运行的方案；两份都提供时与托盘的行相同。
      require(tray_menu_scheme_items(all, state, true).size() == expected.size());
      // 只提供部分方案的版本只列出它提供的。
      auto pinyin = all;
      pinyin.schemes = scheme::OfferedSchemes{};
      pinyin.schemes.offered[scheme::Quanpin] = true;
      pinyin.schemes.offered[scheme::Shuangpin] = true;
      const auto offered = tray_menu_scheme_items(pinyin, state);
      require(offered.size() == 2 && offered[0].command == TrayMenuCommand::SelectQuanpin &&
              offered[1].command == TrayMenuCommand::SelectShuangpin);
      pinyin.schemes = scheme::OfferedSchemes{};
      require(tray_menu_scheme_items(pinyin, state).empty());
    }

    // 右键菜单：表情与符号、打开设置、关于、分隔线、隐藏悬浮状态栏；缺设置窗口时只有依赖它的两行不可用。
    {
      using K = TrayMenuRowKind;
      using C = TrayMenuCommand;
      const auto utility = toolbar_utility_menu_items(all);
      const std::vector<std::pair<K, C>> expected{
          {K::Item, C::OpenSystemEmoji}, {K::Item, C::OpenSettings},
          {K::Item, C::OpenAbout},       {K::Separator, C::OpenSettings},
          {K::Item, C::HideFloatingToolbar}};
      require(utility.size() == expected.size());
      for (size_t index = 0; index < utility.size(); ++index) {
        require(utility[index].kind == expected[index].first);
        if (utility[index].kind != K::Item)
          continue;
        require(utility[index].command == expected[index].second);
        require(utility[index].available && !utility[index].checked);
        require(tray_menu_closes_after(utility[index].command));
        require(!tray_menu_scheme(utility[index].command));
      }
      require(utility[find(utility, C::OpenSystemEmoji)].label == "表情与符号…");
      require(utility[find(utility, C::OpenSettings)].label == "打开设置…");
      require(utility[find(utility, C::OpenAbout)].label == "关于水杉输入法…");
      require(utility[find(utility, C::HideFloatingToolbar)].label == "隐藏悬浮状态栏");
      // 打开设置和关于走设置窗口的路由；系统表情和隐藏工具栏由 Server 自己处理。
      const auto settings = shell_surface_request(C::OpenSettings);
      require(settings && settings->panel.empty() && settings->page.empty());
      require(shell_route_argument(*settings) == L"settings");
      const auto about = shell_surface_request(C::OpenAbout);
      require(about && about->panel.empty() && about->page == "about");
      require(shell_route_argument(*about) == L"settings:about");
      for (const auto command : {C::OpenSystemEmoji, C::HideFloatingToolbar})
        require(!shell_surface_request(command));
      require(tray_menu_geometry(utility, TrayMenuMetrics{}).rows.size() == utility.size());
      auto bare = all;
      bare.settings = false;
      const auto offline = toolbar_utility_menu_items(bare);
      for (const auto command : {C::OpenSettings, C::OpenAbout})
        require(!offline[find(offline, command)].available);
      for (const auto command : {C::OpenSystemEmoji, C::HideFloatingToolbar})
        require(offline[find(offline, command)].available);
    }

    // 菜单贴着工具栏打开：上方放得下就在上方，放不下就在下方，不盖住工具栏；上下都放不下时留在工作区内。
    {
      const TrayMenuSize menu{260.0, 300.0};
      const auto above = toolbar_menu_bounds(1000, 900, 950, 0, 0, 1920, 1040, 96, menu);
      require(above.y == 600 && above.x == 870 && above.height == 300);
      const auto below = toolbar_menu_bounds(1000, 20, 70, 0, 0, 1920, 1040, 96, menu);
      require(below.y == 70);
      const auto cramped = toolbar_menu_bounds(1000, 100, 150, 0, 0, 1920, 400, 96, menu);
      require(cramped.y >= 0 && cramped.y + cramped.height <= 400);
      // 靠近屏幕右边的按钮：菜单向左收回到工作区内。
      const auto edge = toolbar_menu_bounds(1910, 900, 950, 0, 0, 1920, 1040, 96, menu);
      require(edge.x == 1920 - 260);
      require(rejected([] {
        (void)toolbar_menu_bounds(0, 100, 50, 0, 0, 100, 100, 96, TrayMenuSize{260.0, 100.0});
      }));
    }
    std::cout << "Toolbar menus: scheme and utility rows match the reduced feature set\n";
  } catch (const std::exception &failure) {
    std::cerr << failure.what() << '\n';
    return 1;
  }
}
