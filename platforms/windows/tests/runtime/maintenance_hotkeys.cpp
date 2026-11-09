#include "MaintenanceHotkeys.h"
#include <iostream>
#include <stdexcept>
#include <string>

using namespace msime::windows;
namespace {
[[noreturn]] void require_failed(int line) {
  throw std::runtime_error("Maintenance hotkey policy failed at line " +
                           std::to_string(line));
}
#define require(value)                                                         \
  do {                                                                         \
    if (!(value))                                                              \
      require_failed(__LINE__);                                                \
  } while (false)
} // namespace
int main() {
  try {
    // 真实的按键可以触发快捷键，本进程注入的不行，否则处理函数自己输入的按键会再触发自己。
    require(maintenance_stroke_may_trigger(false, 0));
    require(!maintenance_stroke_may_trigger(true, 0));
    require(!maintenance_stroke_may_trigger(true, 0x4D534D45u));

    // The three letter shortcuts.
    const auto clear = maintenance_hotkey('C', true, true, true);
    require(clear && clear->action == MaintenanceAction::ClearCache);
    const auto restart = maintenance_hotkey('R', true, true, true);
    require(restart && restart->action == MaintenanceAction::Restart);
    const auto stop = maintenance_hotkey('T', true, true, true);
    require(stop && stop->action == MaintenanceAction::Stop);
    // Three distinct actions, or two of the documented shortcuts would collide.
    require(clear->action != restart->action && restart->action != stop->action);

    // Digits 1..8 map to zero-based slots, from both the number row and keypad.
    for (uint32_t i = 0; i < 8; ++i) {
      const auto row = maintenance_hotkey('1' + i, true, true, true);
      require(row && row->action == MaintenanceAction::DeleteCandidate);
      require(row->slot == i);
      const auto pad = maintenance_hotkey(0x61 + i, true, true, true);
      require(pad && pad->action == MaintenanceAction::DeleteCandidate);
      require(pad->slot == i);
    }
    // 9 and 0 are deliberately unbound, on both the row and the keypad.
    require(!maintenance_hotkey('9', true, true, true));
    require(!maintenance_hotkey('0', true, true, true));
    require(!maintenance_hotkey(0x69, true, true, true)); // VK_NUMPAD9
    require(!maintenance_hotkey(0x60, true, true, true)); // VK_NUMPAD0

    // Every one of the three modifiers is required. Dropping any of them must
    // leave the stroke to the focused application - swallowing Ctrl+Shift+C or
    // a bare digit would break ordinary typing.
    for (const uint32_t vk : {static_cast<uint32_t>('C'),
                              static_cast<uint32_t>('R'),
                              static_cast<uint32_t>('T'),
                              static_cast<uint32_t>('3')}) {
      require(!maintenance_hotkey(vk, false, true, true));
      require(!maintenance_hotkey(vk, true, false, true));
      require(!maintenance_hotkey(vk, true, true, false));
      require(!maintenance_hotkey(vk, false, false, false));
    }

    // Unrelated keys are never claimed.
    for (const uint32_t vk : {static_cast<uint32_t>('A'),
                              static_cast<uint32_t>('Z'),
                              static_cast<uint32_t>(0x20), // VK_SPACE
                              static_cast<uint32_t>(0x0D)}) // VK_RETURN
      require(!maintenance_hotkey(vk, true, true, true));

    // Win 是第四个修饰键；带上它就不再是这组快捷键。
    require(!maintenance_hotkey('C', true, true, true, true));
    require(!maintenance_hotkey('3', true, true, true, true));
    // And the Alt group still works without Win.
    require(maintenance_hotkey('C', true, true, true, false));

    // 轻点 Caps Lock 切换中英文：真实的一下打开大写锁定，TIP 注入的一下撤回。两下都要计入，报告的状态才回到关。
    CapsLockTracker caps{false, false};
    require(caps.press() && caps.on);
    caps.release();
    require(caps.press() && !caps.on);
    caps.release();
    // 之后的长按照常报告打开，再按一次报告关闭，不会反过来。
    require(caps.press() && caps.on);
    // 按着不放的自动重复不再翻转。
    require(!caps.press() && caps.on);
    caps.release();
    require(caps.press() && !caps.on);

    std::cout << "Maintenance hotkeys: the documented four are recognised\n";
  } catch (const std::exception &failure) {
    std::cerr << failure.what() << '\n';
    return 1;
  } catch (...) {
    std::cerr << "Maintenance hotkey policy failed with an unknown error\n";
    return 1;
  }
}
