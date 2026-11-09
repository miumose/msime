#include "../../settings/SettingsMascotIntro.h"
#include "../../settings/SettingsNavIcons.h"
#include "../../settings/SettingsNavigation.h"
#include <array>
#include <cstdio>
#include <set>
#include <stdexcept>
#include <string_view>
#include <utility>

using namespace msime::settings;
void require_at(bool value, int line) {
  if (!value)
    throw std::runtime_error("Settings navigation test failed at line " +
                             std::to_string(line));
}
#define require(...) require_at((__VA_ARGS__), __LINE__)
int main() {
  try {
    // 侧栏：5 个互不重复的页面，分在 3 个非空组里，按组的顺序排列。
    std::set<std::string_view> ids;
    std::set<std::size_t> groups;
    std::size_t previous_group = 0;
    for (const auto &page : pages) {
      require(ids.insert(page.id).second);
      require(page.group < page_group_count);
      require(page.group >= previous_group);
      previous_group = page.group;
      groups.insert(page.group);
    }
    require(ids.size() == 5);
    require(groups.size() == page_group_count);
    const std::array<std::pair<std::string_view, std::size_t>, 5> sidebar{{
        {"typing", 0}, {"shortcuts", 0}, {"candidate", 1}, {"toolbar", 1},
        {"about", 2}}};
    require(sidebar.size() == pages.size());
    for (std::size_t i = 0; i < pages.size(); ++i)
      require(pages[i].id == sidebar[i].first &&
              pages[i].group == sidebar[i].second);
    require(pages.front().id == default_page);

    // 每一页都在本窗口里原生绘制，没有交给共享应用的面板。
    for (const auto &page : pages) {
      require(page.host == PageHost::Native);
      require(page.shell.panel.empty() && page.shell.page.empty());
    }
    require(shell_links::all.empty());
    // 已移除功能的页面不再存在。
    for (auto id : {"lexicon", "expression", "voice", "osk", "themes", "clip",
                    "ai", "plugins", "stats", "account", "dev", "hand",
                    "community", "download"})
      require(find_page(id) == nullptr);
    // 保留下来的页面都能按 id 找到。
    for (auto id : {"typing", "shortcuts", "candidate", "toolbar", "about"})
      require(find_page(id) != nullptr);

    // 旧路由别名落到仍然存在的页面，未知路由落到默认页。
    require(page_for_route("appearance") == "candidate");
    require(page_for_route("input") == "typing");
    require(page_for_route("shortcuts") == "shortcuts");
    require(page_for_route("floating-toolbar") == "toolbar");
    require(page_for_route("help") == "about");
    require(page_for_route("about") == "about");
    require(page_for_route("feedback") == "about");
    for (auto removed : {"lexicon", "expression", "voice", "osk", "themes",
                         "clip", "ai", "plugins", "stats", "account", "dev",
                         "handwriting", "screen-keyboard", "dictionary"})
      require(page_for_route(removed) == default_page);
    require(page_for_route("") == default_page);
    require(page_for_route("nonexistent") == default_page);
    for (const auto &page : pages)
      require(page_for_route(page.id) == page.id);
    require(offered_page_for_route("handwriting", true) == default_page);
    require(offered_page_for_route("handwriting", false) == default_page);
    for (const auto &page : pages)
      for (const bool offered : {true, false})
        require(offered_page_for_route(page.id, offered) == page.id);

    // 侧栏每一页都有一个产品图标，按侧栏顺序排列，路径是非零环绕的 XAML 路径标记。
    require(nav_icons.size() == pages.size());
    for (std::size_t i = 0; i < pages.size(); ++i) {
      require(nav_icons[i].page == pages[i].id);
      require(nav_icon_path(pages[i].id).substr(0, 4) == L"F1 M");
      require(nav_icons[i].source.size() > 4 &&
              nav_icons[i].source.substr(nav_icons[i].source.size() - 4) == ".svg");
    }
    require(nav_icon_path("lexicon").empty());
    require(nav_icon_path("clip").empty());

    // 页首引导：除「关于」以外每一页都有一句说明，说明照共享 UI 的 `settings-mascot-intro.tsx`。
    for (const auto &page : pages) {
      const auto intro = mascot_intro(page.id);
      require(intro.has_value() == (page.id != "about"));
      if (intro)
        require(!intro->description.empty());
    }
    for (const auto &entry : mascot_intros)
      require(find_page(entry.page) != nullptr);
    require(mascot_intro("account") == std::nullopt);
    require(mascot_intro("about") == std::nullopt);
    require(mascot_intro("typing", "unknown")->description ==
            L"选择拼音方案，调整选词与输出习惯。");

    std::puts("Settings navigation: five native pages, old routes fall back");
  } catch (const std::exception &error) {
    std::fputs(error.what(), stderr);
    std::fputs("\n", stderr);
    return 1;
  }
  return 0;
}
