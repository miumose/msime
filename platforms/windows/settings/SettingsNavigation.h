#pragma once
#include <array>
#include <cstddef>
#include <string_view>

// 精简后的 Windows 设置窗口页面表：只保留拼音输入需要的输入、快捷键、候选窗口、悬浮工具栏和关于。
// 主题皮肤、账号同步、社区、插件、剪贴板、统计、AI、词库、语音、屏幕键盘和诊断页面已随对应功能移除。
namespace msime::settings {

enum class PageHost { Native, Shell };

struct ShellTarget {
  std::string_view panel;
  std::string_view page;
};

struct Page {
  std::string_view id;
  std::size_t group;
  PageHost host;
  ShellTarget shell;
};

inline constexpr std::size_t page_group_count = 3;

inline constexpr std::array<Page, 5> pages{{
    {"typing", 0, PageHost::Native, {}},
    {"shortcuts", 0, PageHost::Native, {}},
    {"candidate", 1, PageHost::Native, {}},
    {"toolbar", 1, PageHost::Native, {}},
    {"about", 2, PageHost::Native, {}},
}};

inline constexpr std::string_view default_page = "typing";

namespace shell_links {
inline constexpr std::array<ShellTarget, 0> all{};
} // namespace shell_links

struct RouteAlias {
  std::string_view route;
  std::string_view page;
};

// 仅保留仍存在的页面和它们的共享路由别名。已移除功能的旧路由落回默认页。
inline constexpr std::array<RouteAlias, 7> route_aliases{{
    {"appearance", "candidate"},
    {"input", "typing"},
    {"shortcuts", "shortcuts"},
    {"floating-toolbar", "toolbar"},
    {"help", "about"},
    {"about", "about"},
    {"feedback", "about"},
}};

constexpr const Page *find_page(std::string_view id) {
  for (const auto &page : pages)
    if (page.id == id)
      return &page;
  return nullptr;
}

constexpr std::string_view page_for_route(std::string_view route) {
  for (const auto &alias : route_aliases)
    if (alias.route == route)
      return alias.page;
  if (const auto *page = find_page(route))
    return page->id;
  return default_page;
}

// 手写已移除，传入的版本开关仅为兼容旧调用点保留。
constexpr std::string_view offered_page_for_route(std::string_view route, bool) {
  return page_for_route(route);
}

} // namespace msime::settings
