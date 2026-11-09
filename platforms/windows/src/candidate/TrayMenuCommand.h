#pragma once

namespace msime::windows {
// What a tray menu row asks the Server to do. Kept apart from the menu model so the mode command mapping can name these without pulling in the menu's labels, which are UTF-8 text that only the menu targets compile with /utf-8.
// 精简后的拼音输入法只保留输入状态、全拼/双拼、主题、悬浮工具栏和设置相关的行；云剪贴板、云词库、背单词、翻译、更新、反馈和手写、屏幕键盘等入口已随对应功能移除。
enum class TrayMenuCommand {
  ToggleFloatingToolbar,
  OpenSettings,
  OpenAbout,
  // Input mode rows, sent to the focused TIP over the same worker protocol the floating toolbar uses.
  SelectChinese,
  SelectEnglish,
  ToggleFullwidth,
  ToggleChinesePunctuation,
  // Engine 自己的英文模式（Ctrl+Shift+E），由 Server 在焦点会话上设置。
  ToggleDedicatedEnglish,
  // 存储的 traditional_chinese_output，走工具栏简繁按钮用的同一个工作线程。
  ToggleTraditionalOutput,
  // Stored preferences the Server writes through the revisioned store.
  SelectQuanpin,
  SelectShuangpin,
  // Settings pages.
  OpenTheme,
  // 悬浮工具栏右键菜单的几行：系统表情面板（Win+.）和隐藏悬浮状态栏（写回偏好）。
  OpenSystemEmoji,
  HideFloatingToolbar,
  // 主题页的一行，写存储的 global_theme；行的 value 是主题 id。
  SelectTheme,
  // 托盘卡片自己的翻页：输入方案页、主题页和返回主页。只在卡片里处理，从不交给 Server。
  ShowSchemes,
  ShowThemes,
  ShowMain,
};
} // namespace msime::windows
