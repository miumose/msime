#include "../../src/candidate/FloatingToolbarSettings.h"
#include <cassert>
#include <thread>
using namespace msime::windows;
int main() {
  const auto defaults = floating_toolbar_settings(nlohmann::json::object());
  assert(defaults && defaults->scale_percent == 100 && defaults->font_size == 24);
  nlohmann::json preferences = {{"floating_toolbar", {
    {"scale_percent", 125}, {"font_size", 28}, {"character_set", false},
    {"punctuation", false}, {"fullwidth", false}, {"settings", false}}}};
  auto settings = floating_toolbar_settings(preferences);
  assert(settings && settings->scale_percent == 125 && settings->font_size == 28);
  assert((settings->items == std::array<bool, 4>{false, false, false, false}));
  // The shared `english_mode` item governs the 中/英 button; absent it stays, as in the reference.
  assert(defaults->language && settings->language);
  // 切换输入方案默认开，logo 默认不画；表情、手写、屏幕键盘和语音按钮已移除。
  assert(defaults->input_scheme && !defaults->show_logo);
  const auto default_slots = floating_toolbar_slots(*defaults);
  assert((default_slots == std::vector<int>{0, 11, 2, 1, 3, 6}));
  assert(default_slots.capacity() >= 6);
  auto english_off = preferences;
  english_off["floating_toolbar"]["english_mode"] = false;
  english_off["floating_toolbar"]["input_scheme"] = false;
  const auto without_language = floating_toolbar_settings(english_off);
  assert(without_language && !without_language->language && !without_language->input_scheme);
  const auto no_language_slots = floating_toolbar_slots(*without_language);
  assert((no_language_slots == std::vector<int>{6}));
  // 全开时的顺序与 macOS 工具栏相同：中/英、切换输入方案、标点、全角、简繁、设置。
  nlohmann::json everything = {{"show_app_logo", true}, {"floating_toolbar", {
    {"english_mode", true}, {"input_scheme", true}, {"character_set", true},
    {"punctuation", true}, {"fullwidth", true}, {"settings", true}}}};
  const auto full = floating_toolbar_settings(everything);
  assert(full && full->show_logo);
  assert((floating_toolbar_slots(*full) == std::vector<int>{0, 11, 2, 1, 3, 6}));
  // 图片工具栏按共享键序请求按钮：全角在标点之前，没有图片键的切换输入方案不请求。
  assert((floating_toolbar_image_slots(floating_toolbar_slots(*full)) == std::vector<int>{0, 1, 2, 3, 6}));
  assert((floating_toolbar_image_slots(default_slots) == std::vector<int>{0, 1, 2, 3, 6}));
  // 设置不同，比较结果也不同，工具栏据此重新排版。
  auto without_settings = *full;
  without_settings.items[3] = false;
  assert(!(without_settings == *full) && *full == *full);
  auto without_logo = *full;
  without_logo.show_logo = false;
  assert(!(without_logo == *full));
  {
    auto bad = everything;
    bad["floating_toolbar"]["input_scheme"] = "true";
    assert(!floating_toolbar_settings(bad));
  }
  auto bad_logo = everything;
  bad_logo["show_app_logo"] = 1;
  assert(!floating_toolbar_settings(bad_logo));
  // 中英文切换提示的 logo 只认布尔 true，与 macOS 相同；工具栏尺寸越界让工具栏设置作废时，提示照样画 logo。
  assert(hud_show_app_logo(everything) && !hud_show_app_logo(bad_logo));
  assert(!hud_show_app_logo(nlohmann::json::object()) && !hud_show_app_logo(nullptr));
  auto oversized = everything;
  oversized["floating_toolbar"]["scale_percent"] = 151;
  assert(!floating_toolbar_settings(oversized) && hud_show_app_logo(oversized));
  english_off["floating_toolbar"]["english_mode"] = "false";
  assert(!floating_toolbar_settings(english_off));
  for (const auto &invalid : {nlohmann::json(-1), nlohmann::json(74),
                            nlohmann::json(151), nlohmann::json(1ULL << 40),
                            nlohmann::json(100.5), nlohmann::json("100")}) {
    auto bad = preferences;
    bad["floating_toolbar"]["scale_percent"] = invalid;
    assert(!floating_toolbar_settings(bad));
  }
  for (int size : {15, 29}) {
    auto bad = preferences;
    bad["floating_toolbar"]["font_size"] = size;
    assert(!floating_toolbar_settings(bad));
  }
  for (int percent : {75, 150}) {
    preferences["floating_toolbar"]["scale_percent"] = percent;
    assert(floating_toolbar_settings(preferences));
  }
  // 已移除按钮的旧键仍可能留在偏好里，读工具栏设置时不认它们，也不因此作废整份设置。
  preferences["floating_toolbar"]["emoji"] = "false";
  preferences["floating_toolbar"]["voice"] = "true";
  assert(floating_toolbar_settings(preferences));
  assert(!floating_toolbar_settings(nullptr));
  FloatingToolbarMailbox mailbox;
  assert(!mailbox.take());
  assert(mailbox.publish(1, *defaults));
  std::thread publisher([&] { assert(mailbox.publish(3, *settings)); });
  publisher.join();
  assert(!mailbox.publish(2, *defaults));
  assert(mailbox.take()->scale_percent == 125);
  assert(!mailbox.take());
  assert(!mailbox.publish(3, *defaults));
  auto invalid = *settings;
  invalid.font_size = 0;
  assert(!mailbox.publish(4, invalid));
  assert(mailbox.publish(4, *defaults));
  assert(mailbox.take()->font_size == 24);
}
