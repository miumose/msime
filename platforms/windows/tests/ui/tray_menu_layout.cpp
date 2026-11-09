#include "TrayMenuLayout.h"
#include <cmath>
#include <set>
#include <stdexcept>
#include <utility>

using namespace msime::windows;
void require(bool value) {
  if (!value)
    throw std::runtime_error("Tray menu layout validation failed");
}
bool near(double value, double expected) {
  return std::fabs(value - expected) < 0.001;
}
bool rejected(void (*action)()) {
  try {
    action();
  } catch (const std::invalid_argument &) {
    return true;
  }
  return false;
}
size_t find(const std::vector<TrayMenuItem> &items, TrayMenuCommand command) {
  for (size_t index = 0; index < items.size(); ++index)
    if ((items[index].kind == TrayMenuRowKind::Item ||
         items[index].kind == TrayMenuRowKind::Tool) &&
        items[index].command == command)
      return index;
  throw std::runtime_error("Tray menu row missing");
}
int main() {
  TrayMenuCapabilities all;
  all.floating_toolbar = true;
  all.settings = true;
  all.schemes = scheme::OfferedSchemes{};
  all.schemes.offered[scheme::Quanpin] = true;
  all.schemes.offered[scheme::Shuangpin] = true;
  all.product_name = "水杉输入法";
  TrayMenuState state;
  state.chinese = true;
  state.fullwidth = false;
  state.chinese_punctuation = true;
  state.scheme = "quanpin";
  state.shuangpin_profile = "xiaohe";
  state.theme_title = "水杉";
  state.theme = "shuishan";
  state.themes = {{"system", "跟随系统"}, {"shuishan", "水杉"}, {"night", "夜青"}};
  state.language_hint = tray_menu_language_hint(true, false, true);
  const auto items = tray_menu_items(all, state);
  auto schemes_of = [&](const TrayMenuCapabilities &capabilities,
                        const TrayMenuState &value) {
    return tray_menu_items(capabilities, value, TrayMenuPage::Schemes);
  };

  // 主页的顺序：标题、输入语言和英文候选模式、几个开关、输入方案和主题两个翻页行、工具栏一格，然后是设置应用的页面。
  {
    using K = TrayMenuRowKind;
    using C = TrayMenuCommand;
    const std::vector<std::pair<K, C>> expected{
        {K::Header, C::OpenSettings},
        {K::Separator, C::OpenSettings},
        {K::Item, C::SelectChinese},
        {K::Item, C::SelectEnglish},
        {K::Item, C::ToggleDedicatedEnglish},
        {K::Separator, C::OpenSettings},
        {K::Item, C::ToggleTraditionalOutput},
        {K::Item, C::ToggleFullwidth},
        {K::Item, C::ToggleChinesePunctuation},
        {K::Separator, C::OpenSettings},
        {K::Item, C::ShowSchemes},
        {K::Item, C::ShowThemes},
        {K::Separator, C::OpenSettings},
        {K::Tool, C::ToggleFloatingToolbar},
        {K::Separator, C::OpenSettings},
        {K::Item, C::OpenSettings},
        {K::Item, C::OpenAbout},
    };
    require(items.size() == expected.size());
    for (size_t index = 0; index < items.size(); ++index) {
      require(items[index].kind == expected[index].first);
      if (items[index].kind == K::Item || items[index].kind == K::Tool)
        require(items[index].command == expected[index].second);
    }
    require(items[0].label == "水杉输入法");
    require(items[2].label == "中文输入" && items[3].label == "英文输入" &&
            items[4].label == "英文候选模式");
    require(items[6].label == "繁体输出" && items[7].label == "全角字符" &&
            items[8].label == "中文标点");
    require(items[10].label == "输入方案" && items[10].hint == "全拼" &&
            items[10].submenu);
    require(items[11].label == "主题" && items[11].hint == "水杉" &&
            items[11].submenu);
    require(items[13].label == "工具栏");
    require(items[15].label == "水杉输入法设置…" &&
            items[16].label == "关于水杉输入法…");
    for (const auto &row : items)
      require(!row.back && (row.submenu == (row.command == C::ShowSchemes ||
                                            row.command == C::ShowThemes) ||
                            row.kind != K::Item));
  }
  // 输入方案页：第一行返回主页，下面是全拼和双拼，和悬浮工具栏的方案菜单是同一组行。
  {
    const auto page = schemes_of(all, state);
    require(page.size() == 4);
    require(page[0].kind == TrayMenuRowKind::Item &&
            page[0].command == TrayMenuCommand::ShowMain && page[0].back &&
            page[0].label == "输入方案");
    require(page[1].kind == TrayMenuRowKind::Separator);
    const auto shared = tray_menu_scheme_items(all, state);
    require(shared.size() == 2);
    for (size_t index = 0; index < shared.size(); ++index)
      require(page[index + 2].command == shared[index].command &&
              page[index + 2].label == shared[index].label &&
              page[index + 2].checked == shared[index].checked);
    require(page[2].label == "全拼" && page[3].label == "双拼（小鹤）");
  }
  // 主题页：返回行、目录里的主题（勾着当前的，行的值是主题 id），最后是设置应用的主题页。
  {
    const auto page = tray_menu_items(all, state, TrayMenuPage::Themes);
    require(page.size() == 7);
    require(page[0].command == TrayMenuCommand::ShowMain && page[0].back &&
            page[0].label == "主题");
    require(page[1].kind == TrayMenuRowKind::Separator);
    require(page[2].command == TrayMenuCommand::SelectTheme &&
            page[2].value == "system" && page[2].label == "跟随系统" &&
            !page[2].checked);
    require(page[3].value == "shuishan" && page[3].checked);
    require(page[4].value == "night" && !page[4].checked);
    require(page[5].kind == TrayMenuRowKind::Separator);
    require(page[6].command == TrayMenuCommand::OpenTheme &&
            page[6].label == "主题设置…" && page[6].available);
    // 再长的目录也不超出卡片的行数上限。
    auto many = state;
    many.themes.assign(40, TrayMenuTheme{"theme", "主题"});
    const auto capped = tray_menu_items(all, many, TrayMenuPage::Themes);
    require(capped.size() == tray_menu_max_rows);
    require(capped.back().command == TrayMenuCommand::OpenTheme);
    (void)tray_menu_geometry(capped, TrayMenuMetrics{});
    // 没有目录时主题行不可用，而不是翻到一张空页。
    auto empty = state;
    empty.themes.clear();
    const auto rows = tray_menu_items(all, empty);
    require(!rows[11].available && rows[11].command == TrayMenuCommand::ShowThemes);
  }
  // 翻页的行只在卡片里换页，不收起卡片。
  require(tray_menu_page_target(TrayMenuCommand::ShowSchemes) == TrayMenuPage::Schemes);
  require(tray_menu_page_target(TrayMenuCommand::ShowThemes) == TrayMenuPage::Themes);
  require(tray_menu_page_target(TrayMenuCommand::ShowMain) == TrayMenuPage::Main);
  require(!tray_menu_page_target(TrayMenuCommand::OpenTheme));
  for (auto command : {TrayMenuCommand::ShowSchemes, TrayMenuCommand::ShowThemes,
                       TrayMenuCommand::ShowMain})
    require(!tray_menu_closes_after(command));
  require(tray_menu_closes_after(TrayMenuCommand::SelectTheme) &&
          tray_menu_closes_after(TrayMenuCommand::ToggleTraditionalOutput) &&
          tray_menu_closes_after(TrayMenuCommand::ToggleDedicatedEnglish));

  // Hints: the configured CN/EN key, the TIP's own shortcuts and the theme name.
  require(items[find(items, TrayMenuCommand::SelectChinese)].hint == "Shift");
  require(items[find(items, TrayMenuCommand::ToggleFullwidth)].hint ==
          "Ctrl + Shift + Space");
  require(items[find(items, TrayMenuCommand::ToggleChinesePunctuation)].hint ==
          "Ctrl + .");
  require(items[find(items, TrayMenuCommand::ShowThemes)].hint == "水杉");
  require(items[find(items, TrayMenuCommand::OpenSettings)].hint.empty());
  require(items[find(items, TrayMenuCommand::ToggleDedicatedEnglish)].hint ==
          "Ctrl + Shift + E");
  require(items[find(items, TrayMenuCommand::ToggleTraditionalOutput)].hint ==
          "Ctrl + Shift + F");
  {
    // 关掉 Ctrl+Shift+F 后这个键不再切换简繁，旁边也不写它。
    auto unbound = state;
    unbound.character_set_shortcut = false;
    const auto rows = tray_menu_items(all, unbound);
    require(rows[find(rows, TrayMenuCommand::ToggleTraditionalOutput)].hint.empty());
  }
  require(tray_menu_language_hint(true, true, true) == "Shift");
  require(tray_menu_language_hint(false, true, true) == "Ctrl");
  require(tray_menu_language_hint(false, false, true) == "Ctrl + Alt + Space");
  require(tray_menu_language_hint(false, false, false).empty());

  // Marks follow the reported state.
  auto checked = [](const std::vector<TrayMenuItem> &rows, TrayMenuCommand command) {
    return rows[find(rows, command)].checked;
  };
  auto available = [](const std::vector<TrayMenuItem> &rows,
                      TrayMenuCommand command) {
    return rows[find(rows, command)].available;
  };
  require(checked(items, TrayMenuCommand::SelectChinese) &&
          !checked(items, TrayMenuCommand::SelectEnglish));
  require(!checked(items, TrayMenuCommand::ToggleFullwidth) &&
          checked(items, TrayMenuCommand::ToggleChinesePunctuation));
  {
    const auto page = schemes_of(all, state);
    require(checked(page, TrayMenuCommand::SelectQuanpin) &&
            !checked(page, TrayMenuCommand::SelectShuangpin));
  }
  // 繁体输出跟随存储的开关。
  require(!checked(items, TrayMenuCommand::ToggleTraditionalOutput));
  {
    auto traditional = state;
    traditional.traditional_output = true;
    require(checked(tray_menu_items(all, traditional),
                    TrayMenuCommand::ToggleTraditionalOutput));
  }
  // 标点被 punctuation_lock 钉住时「中文标点」不可用，勾仍然显示当前状态。
  {
    auto locked = state;
    locked.punctuation_locked = true;
    const auto rows = tray_menu_items(all, locked);
    require(!available(rows, TrayMenuCommand::ToggleChinesePunctuation) &&
            checked(rows, TrayMenuCommand::ToggleChinesePunctuation));
  }
  // 英文候选模式：TIP 报告中文、方案有这个模式时可用，勾跟随 Engine 的英文模式；TIP 在英文状态或状态未知时不可用。
  {
    require(available(items, TrayMenuCommand::ToggleDedicatedEnglish) &&
            !checked(items, TrayMenuCommand::ToggleDedicatedEnglish));
    auto dedicated = state;
    dedicated.dedicated_english = true;
    auto rows = tray_menu_items(all, dedicated);
    require(available(rows, TrayMenuCommand::ToggleDedicatedEnglish) &&
            checked(rows, TrayMenuCommand::ToggleDedicatedEnglish));
    dedicated.chinese = false;
    rows = tray_menu_items(all, dedicated);
    require(!available(rows, TrayMenuCommand::ToggleDedicatedEnglish) &&
            !checked(rows, TrayMenuCommand::ToggleDedicatedEnglish));
    using A = TrayMenuEnglishAction;
    require(tray_menu_dedicated_english_action(true, false, "quanpin") == A::Enter);
    require(tray_menu_dedicated_english_action(true, true, "quanpin") == A::Leave);
    require(tray_menu_dedicated_english_action(false, true, "quanpin") == A::Unavailable);
    require(tray_menu_dedicated_english_action(std::nullopt, false, "quanpin") ==
            A::Unavailable);
  }
  // Exactly one scheme is marked, whichever it is.
  for (const char *scheme : {"quanpin", "shuangpin"}) {
    auto next = state;
    next.scheme = scheme;
    const auto rows = schemes_of(all, next);
    int marked = 0;
    for (auto command : {TrayMenuCommand::SelectQuanpin, TrayMenuCommand::SelectShuangpin})
      marked += checked(rows, command) ? 1 : 0;
    require(marked == 1);
  }
  // Every scheme row names the stored value it selects.
  require(std::string(tray_menu_scheme(TrayMenuCommand::SelectQuanpin)) == "quanpin");
  require(std::string(tray_menu_scheme(TrayMenuCommand::SelectShuangpin)) == "shuangpin");
  require(!tray_menu_scheme(TrayMenuCommand::SelectChinese));
  require(!tray_menu_scheme(TrayMenuCommand::OpenSettings));
  // The shuangpin row names the stored layout, and falls back to the plain name.
  require(tray_menu_shuangpin_label("ziranma") == "双拼（自然码）");
  require(tray_menu_shuangpin_label("microsoft") == "双拼（微软）");
  require(tray_menu_shuangpin_label("shoudao") == "双拼（首道）");
  require(tray_menu_shuangpin_label("smart_abc") == "双拼（智能 ABC）");
  require(tray_menu_shuangpin_label("unknown") == "双拼");
  {
    auto stored = state;
    stored.scheme = "shuangpin";
    stored.shuangpin_profile = "ziranma";
    const auto rows = schemes_of(all, stored);
    require(rows[find(rows, TrayMenuCommand::SelectShuangpin)].label == "双拼（自然码）" &&
            checked(rows, TrayMenuCommand::SelectShuangpin));
  }
  {
    // English, and the Engine's own English mode while the TIP still reports Chinese.
    auto english = state;
    english.chinese = false;
    auto rows = tray_menu_items(all, english);
    require(!checked(rows, TrayMenuCommand::SelectChinese) &&
            checked(rows, TrayMenuCommand::SelectEnglish));
    english.chinese = true;
    english.dedicated_english = true;
    rows = tray_menu_items(all, english);
    require(!checked(rows, TrayMenuCommand::SelectChinese) &&
            checked(rows, TrayMenuCommand::SelectEnglish));
  }
  {
    // Unknown modes are not assumed: the rows stay visible, unmarked and disabled.
    auto unknown = state;
    unknown.chinese.reset();
    unknown.fullwidth.reset();
    unknown.chinese_punctuation.reset();
    const auto rows = tray_menu_items(all, unknown);
    require(rows.size() == items.size());
    for (auto command :
         {TrayMenuCommand::SelectChinese, TrayMenuCommand::SelectEnglish,
          TrayMenuCommand::ToggleDedicatedEnglish,
          TrayMenuCommand::ToggleFullwidth,
          TrayMenuCommand::ToggleChinesePunctuation})
      require(!available(rows, command) && !checked(rows, command));
    // Stored preferences do not depend on a focused session.
    require(available(rows, TrayMenuCommand::ToggleTraditionalOutput) &&
            available(schemes_of(all, unknown), TrayMenuCommand::SelectQuanpin));
  }

  // The tool strip keeps the toolbar switch, with a glyph and a text fallback, following the live state.
  {
    std::set<wchar_t> glyphs;
    size_t tools = 0;
    for (const auto &row : items) {
      if (row.kind != TrayMenuRowKind::Tool) {
        require(row.icon == 0);
        continue;
      }
      ++tools;
      require(row.icon != 0);
      require(row.icon_fallback && row.icon_fallback[0] != L'\0');
      require(glyphs.insert(row.icon).second);
      require(row.toggle && row.command == TrayMenuCommand::ToggleFloatingToolbar);
    }
    require(tools == 1);
    require(!checked(items, TrayMenuCommand::ToggleFloatingToolbar));
    auto shown = state;
    shown.floating_toolbar = true;
    require(checked(tray_menu_items(all, shown),
                    TrayMenuCommand::ToggleFloatingToolbar));
  }

  // A host without a capability shows the row disabled rather than hiding it or accepting a click that would do nothing.
  TrayMenuCapabilities server_only;
  const auto limited = tray_menu_items(server_only, state);
  require(limited.size() == items.size());
  require(available(limited, TrayMenuCommand::ToggleFloatingToolbar));
  for (auto command : {TrayMenuCommand::OpenSettings, TrayMenuCommand::OpenAbout})
    require(!available(limited, command));
  // 主题页的主题行只写偏好，不需要外壳；「主题设置…」要设置窗口。
  {
    const auto page = tray_menu_items(server_only, state, TrayMenuPage::Themes);
    require(available(page, TrayMenuCommand::SelectTheme) &&
            !available(page, TrayMenuCommand::OpenTheme));
    require(available(limited, TrayMenuCommand::ShowThemes) &&
            available(limited, TrayMenuCommand::ShowSchemes));
  }
  // Modes and stored switches need no shell.
  for (auto command :
       {TrayMenuCommand::SelectChinese, TrayMenuCommand::ToggleFullwidth,
        TrayMenuCommand::ToggleTraditionalOutput})
    require(available(limited, command));
  require(available(schemes_of(server_only, state), TrayMenuCommand::SelectQuanpin));

  // Geometry: the design's 260 wide card, 4 padding, 32 rows, one 52 strip for the toolbar tool.
  const TrayMenuMetrics metrics;
  require(near(metrics.width, 260.0) && near(metrics.padding, 4.0) &&
          near(metrics.row_height, 32.0) && near(metrics.item_radius, 4.0) &&
          near(metrics.radius, 8.0));
  const auto geometry = tray_menu_geometry(items, metrics);
  require(geometry.rows.size() == items.size());
  require(near(geometry.size.width, 260.0));
  const double expected_height =
      metrics.padding * 2.0 + metrics.header_height +
      metrics.separator_height * 5.0 + metrics.row_height * 10.0 +
      metrics.tool_height;
  require(near(geometry.size.height, expected_height));
  require(near(tray_menu_size(items, metrics).height, expected_height));
  // 方案收进自己的一页后，主页按设计尺寸放得进 150% 缩放下 1080p 的工作区，方案页和主题页也一样。更矮的工作区里命令行变矮，什么都不被裁掉。
  {
    const double work = 1040.0 / 1.5;
    require(geometry.size.height <= work);
    require(near(tray_menu_fitted_metrics(items, metrics, work).row_height,
                 metrics.row_height));
    require(tray_menu_geometry(schemes_of(all, state), metrics).size.height <= work);
    require(tray_menu_geometry(tray_menu_items(all, state, TrayMenuPage::Themes),
                               metrics)
                .size.height <= work);
    // 200% 缩放的 1080p 工作区（约 500 DIP）：比设计高度矮，压到最小行高放得下，最后一行不被裁掉。
    const double short_work = 500.0;
    const auto fitted = tray_menu_fitted_metrics(items, metrics, short_work);
    require(fitted.row_height < metrics.row_height &&
            fitted.row_height >= tray_menu_compact_row_height);
    require(tray_menu_geometry(items, fitted).size.height <= short_work + 0.001);
    require(near(tray_menu_fitted_metrics(items, metrics, 1400.0).row_height,
                 metrics.row_height));
    // A work area too short even for compact rows stops at the compact height; the placement clamps what is left.
    require(near(tray_menu_fitted_metrics(items, metrics, 300.0).row_height,
                 tray_menu_compact_row_height));
  }
  // Rows stack without gaps, inside the padding.
  require(near(geometry.rows[0].top, metrics.padding));
  require(near(geometry.rows[0].bottom, metrics.padding + metrics.header_height));
  for (size_t index = 1; index < items.size(); ++index) {
    const auto &row = geometry.rows[index];
    const auto &previous = geometry.rows[index - 1];
    const bool same_strip = items[index].kind == TrayMenuRowKind::Tool &&
                            items[index - 1].kind == TrayMenuRowKind::Tool;
    require(near(row.top, same_strip ? previous.top : previous.bottom));
    if (same_strip)
      require(near(row.left, previous.right));
    require(row.left >= metrics.padding - 0.001 &&
            row.right <= metrics.width - metrics.padding + 0.001);
  }
  {
    const auto chinese = geometry.rows[find(items, TrayMenuCommand::SelectChinese)];
    require(near(chinese.bottom - chinese.top, 32.0));
    require(near(chinese.left, 4.0) && near(chinese.right, 256.0));
    const auto tool =
        geometry.rows[find(items, TrayMenuCommand::ToggleFloatingToolbar)];
    require(near(tool.left, 4.0) && near(tool.right, 256.0));
    require(near(tool.bottom - tool.top, metrics.tool_height));
    require(near(geometry.rows[1].bottom - geometry.rows[1].top,
                 metrics.separator_height));
  }
  require(rejected([] {
    (void)tray_menu_geometry(std::vector<TrayMenuItem>{}, TrayMenuMetrics{});
  }));
  require(rejected([] {
    (void)tray_menu_geometry(std::vector<TrayMenuItem>(33), TrayMenuMetrics{});
  }));
  require(rejected([] {
    TrayMenuMetrics narrow;
    narrow.width = 8.0;
    (void)tray_menu_geometry(std::vector<TrayMenuItem>(1), narrow);
  }));

  // Clicks land on the row that was drawn; decorative and disabled rows swallow theirs.
  auto hit = [&](const std::vector<TrayMenuItem> &rows, double x, double y) {
    return tray_menu_hit(x, y, rows, metrics);
  };
  auto centre = [](const TrayMenuRect &row) {
    return std::pair<double, double>{(row.left + row.right) / 2.0,
                                     (row.top + row.bottom) / 2.0};
  };
  for (size_t index = 0; index < items.size(); ++index) {
    const auto [x, y] = centre(geometry.rows[index]);
    const bool actionable = items[index].kind == TrayMenuRowKind::Item ||
                            items[index].kind == TrayMenuRowKind::Tool;
    require(hit(items, x, y) ==
            (actionable ? std::optional<size_t>(index) : std::nullopt));
    require(tray_menu_actionable(items[index]) == actionable);
  }
  require(!hit(items, 10.0, metrics.padding / 2.0));
  require(!hit(items, 10.0, geometry.size.height));
  {
    const auto chinese = geometry.rows[find(items, TrayMenuCommand::SelectChinese)];
    require(!hit(items, -1.0, chinese.top + 1.0) &&
            !hit(items, geometry.size.width, chinese.top + 1.0));
    // The side padding belongs to no row.
    require(!hit(items, 2.0, chinese.top + 1.0));
    const auto unavailable = find(limited, TrayMenuCommand::OpenSettings);
    const auto [ux, uy] = centre(tray_menu_geometry(limited, metrics).rows[unavailable]);
    require(!hit(limited, ux, uy));
    const auto toolbar = find(limited, TrayMenuCommand::ToggleFloatingToolbar);
    const auto [tx, ty] =
        centre(tray_menu_geometry(limited, metrics).rows[toolbar]);
    require(hit(limited, tx, ty) == std::optional<size_t>(toolbar));
  }

  // The card opens above the tray icon and stays inside the work area.
  const auto size = geometry.size;
  const auto placed = tray_menu_bounds(1700, 1040, 0, 0, 1920, 1040, 96, size);
  require(placed.width == 260 &&
          placed.height == static_cast<int>(size.height + 0.5));
  require(placed.y == 1040 - placed.height);
  require(placed.x == 1700 - placed.width / 2);
  // A corner icon cannot push the card off screen.
  const auto clamped = tray_menu_bounds(1915, 1040, 0, 0, 1920, 1040, 96, size);
  require(clamped.x == 1920 - clamped.width);
  const auto scaled = tray_menu_bounds(960, 1040, 0, 0, 1920, 1040, 192, size);
  require(scaled.width == 520);
  // A work area smaller than the card clamps instead of overflowing.
  const auto tiny = tray_menu_bounds(50, 80, 0, 0, 100, 80, 96, size);
  require(tiny.width == 100 && tiny.height == 80 && tiny.x == 0 && tiny.y == 0);
  require(rejected([] {
    (void)tray_menu_bounds(0, 0, 0, 0, 0, 0, 96, TrayMenuSize{260.0, 100.0});
  }));
  require(rejected([] {
    (void)tray_menu_bounds(0, 0, 0, 0, 100, 100, 20, TrayMenuSize{260.0, 100.0});
  }));

  // Labels start after the mark column, so they line up whether or not a row is checked.
  require(near(tray_menu_mark_x(metrics), metrics.inset));
  require(near(tray_menu_label_x(metrics),
               metrics.inset + metrics.mark_column + metrics.gap));

  // The toolbar switch flips in place and leaves the menu open, as the reference does; the page rows only turn the page; every other row dismisses it.
  require(!tray_menu_closes_after(TrayMenuCommand::ToggleFloatingToolbar));
  for (const auto &row : items)
    if ((row.kind == TrayMenuRowKind::Item || row.kind == TrayMenuRowKind::Tool) &&
        row.command != TrayMenuCommand::ToggleFloatingToolbar)
      require(tray_menu_closes_after(row.command) ==
              !tray_menu_page_target(row.command));

  // A scheme row writes the value that checks it, and no other row names a scheme.
  int scheme_rows = 0;
  for (const auto &row : schemes_of(all, state)) {
    if (row.kind != TrayMenuRowKind::Item)
      continue;
    const char *scheme = tray_menu_scheme(row.command);
    if (!scheme)
      continue;
    ++scheme_rows;
    TrayMenuState selected;
    selected.scheme = scheme;
    require(checked(schemes_of(all, selected), row.command));
  }
  require(scheme_rows == 2);
  for (const auto &row : items)
    if (row.kind == TrayMenuRowKind::Item)
      require(!tray_menu_scheme(row.command));

  // 键盘导航：导航键被卡片吞下，单按修饰键什么也不做，别的键和组合键收起卡片并照常交给应用。
  {
    using Key = TrayMenuKey;
    for (bool highlighted : {false, true})
      require(tray_menu_key(0x26, false, highlighted) == Key::Previous &&
              tray_menu_key(0x28, false, highlighted) == Key::Next &&
              tray_menu_key(0x24, false, highlighted) == Key::First &&
              tray_menu_key(0x23, false, highlighted) == Key::Last &&
              tray_menu_key(0x27, false, highlighted) == Key::Forward &&
              tray_menu_key(0x25, false, highlighted) == Key::Back &&
              tray_menu_key(0x1B, false, highlighted) == Key::Close);
    require(tray_menu_key(0x0D, false, true) == Key::Activate &&
            tray_menu_key(0x20, false, true) == Key::Activate);
    require(tray_menu_key(0x0D, false, false) == Key::Dismiss &&
            tray_menu_key(0x20, false, false) == Key::Dismiss);
    for (unsigned modifier : {0x10u, 0x11u, 0x12u, 0x14u, 0x5Bu, 0x5Cu, 0xA0u, 0xA5u})
      require(tray_menu_key(modifier, false, true) == Key::None &&
              tray_menu_key(modifier, true, true) == Key::None);
    require(tray_menu_key('A', false, true) == Key::Dismiss);
    require(tray_menu_key(0x28, true, true) == Key::Dismiss);
    require(tray_menu_key_swallowed(Key::Next) && tray_menu_key_swallowed(Key::Close) &&
            !tray_menu_key_swallowed(Key::Dismiss) && !tray_menu_key_swallowed(Key::None));

    // 上下键跳过标题、分隔线和不可用的行，首尾相接；Home/End 到第一行和最后一行可点的行。
    const size_t chinese = find(items, TrayMenuCommand::SelectChinese);
    const size_t about = find(items, TrayMenuCommand::OpenAbout);
    require(tray_menu_step(items, std::nullopt, true) == chinese);
    require(tray_menu_step(items, std::nullopt, false) == about);
    require(tray_menu_step(items, about, true) == chinese);
    require(tray_menu_step(items, chinese, false) == about);
    const size_t dedicated = find(items, TrayMenuCommand::ToggleDedicatedEnglish);
    require(tray_menu_step(items, dedicated, true) ==
            find(items, TrayMenuCommand::ToggleTraditionalOutput));
    {
      std::vector<TrayMenuItem> decorative(2);
      decorative[0].kind = TrayMenuRowKind::Separator;
      decorative[1].available = false;
      require(!tray_menu_step(decorative, std::nullopt, true));
    }
    auto press = [&](std::optional<size_t> current, Key key, bool subpage = false) {
      return tray_menu_key_result(items, current, key, subpage);
    };
    require(press(std::nullopt, Key::Next).highlight == chinese);
    require(press(std::nullopt, Key::Previous).highlight == about);
    require(press(dedicated, Key::First).highlight == chinese);
    require(press(dedicated, Key::Last).highlight == about);
    require(press(dedicated, Key::Activate).activate);
    require(!press(std::nullopt, Key::Activate).activate);
    // → 打开高亮的翻页行，在普通行上什么也不做。
    const size_t schemes_row = find(items, TrayMenuCommand::ShowSchemes);
    require(press(schemes_row, Key::Forward).activate);
    require(!press(dedicated, Key::Forward).activate &&
            press(dedicated, Key::Forward).highlight == dedicated);
    // 工具条只有一格：←→ 停在这一格。
    const size_t toolbar_tool = find(items, TrayMenuCommand::ToggleFloatingToolbar);
    require(press(toolbar_tool, Key::Forward).highlight == toolbar_tool &&
            !press(toolbar_tool, Key::Forward).activate);
    require(press(toolbar_tool, Key::Back).highlight == toolbar_tool &&
            !press(toolbar_tool, Key::Back).back);
    // Esc 在主页上收起卡片，在子页上回到主页；← 只在子页上回到主页。别的键收起卡片。
    require(press(dedicated, Key::Close).close);
    require(!press(dedicated, Key::Back).back && !press(dedicated, Key::Back).close);
    require(press(std::nullopt, Key::Dismiss).close);
    const auto page = schemes_of(all, state);
    require(tray_menu_key_result(page, 0, Key::Close, true).back);
    require(tray_menu_key_result(page, 3, Key::Back, true).back);
    // 指向不可点的行的高亮当作没有高亮。
    require(press(1, Key::Activate).highlight == std::nullopt &&
            !press(1, Key::Activate).activate);
    // 键盘翻页后的高亮：进子页时是勾着的那一行，回主页时是打开那一页的那一行。
    require(tray_menu_page_highlight(page, TrayMenuPage::Main) ==
            find(page, TrayMenuCommand::SelectQuanpin));
    require(tray_menu_page_highlight(items, TrayMenuPage::Schemes) == schemes_row);
    require(tray_menu_page_highlight(items, TrayMenuPage::Themes) ==
            find(items, TrayMenuCommand::ShowThemes));
    auto unmarked = state;
    unmarked.theme = "missing";
    const auto themes = tray_menu_items(all, unmarked, TrayMenuPage::Themes);
    require(tray_menu_page_highlight(themes, TrayMenuPage::Main) == size_t{2});
  }
}
