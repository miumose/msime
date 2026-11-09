#pragma once
#include <algorithm>
#include <array>
#include <cstdint>
#include <mutex>
#include <nlohmann/json.hpp>
#include <optional>
#include <vector>

namespace msime::windows {
struct FloatingToolbarSettings {
  unsigned scale_percent = 100;
  unsigned font_size = 24;
  // 按 character_set、punctuation、fullwidth、settings 排列。表情、屏幕键盘、手写和语音按钮已随对应功能移除。
  std::array<bool, 4> items{true, true, true, true};
  // The shared `english_mode` item. The reference always shows its 中/英 button, so this stays on unless the preference turns it off.
  bool language = true;
  // 共享偏好里的切换输入方案按钮，默认值与 client-core 的 FloatingToolbarPreferences 一致。它不进 `items`，因为 `items` 也是 PreviewConfig 的 floating_toolbar_items 契约。
  bool input_scheme = true;
  // 顶层偏好 `show_app_logo`：关掉时左端画两列三行圆点的握把而不是 logo，仍然可以拖动。读不到时按新装处理，不画 logo，与 macOS 一致。
  bool show_logo = false;
  bool operator==(const FloatingToolbarSettings &other) const {
    return scale_percent == other.scale_percent && font_size == other.font_size &&
           items == other.items && language == other.language &&
           input_scheme == other.input_scheme && show_logo == other.show_logo;
  }
  bool valid() const {
    return scale_percent >= 75 && scale_percent <= 150 &&
           font_size >= 16 && font_size <= 28;
  }
};

inline std::optional<FloatingToolbarSettings>
floating_toolbar_settings(const nlohmann::json &preferences) {
  try {
    if (!preferences.is_object()) return std::nullopt;
    const auto toolbar = preferences.value("floating_toolbar", nlohmann::json::object());
    if (!toolbar.is_object()) return std::nullopt;
    FloatingToolbarSettings result;
    for (const auto &[key, target] :
         {std::pair{"scale_percent", &result.scale_percent},
          std::pair{"font_size", &result.font_size}}) {
      if (!toolbar.contains(key)) continue;
      const auto &value = toolbar.at(key);
      if (!value.is_number_integer() || value < 0 || value > 200)
        return std::nullopt;
      *target = value.get<unsigned>();
    }
    constexpr const char *names[] = {"character_set", "punctuation", "fullwidth",
                                     "settings"};
    for (size_t i = 0; i < result.items.size(); ++i)
      result.items[i] = toolbar.value(names[i], result.items[i]);
    result.language = toolbar.value("english_mode", result.language);
    result.input_scheme = toolbar.value("input_scheme", result.input_scheme);
    result.show_logo = preferences.value("show_app_logo", result.show_logo);
    return result.valid() ? std::optional{result} : std::nullopt;
  } catch (...) {
    return std::nullopt;
  }
}

// 中英文切换提示读的 `show_app_logo`：只认真正的布尔 true，缺键或类型不对都当作关，与 macOS InputModeHUDPanel 的 applySizingPreferences: 相同。和 floating_toolbar_settings 分开读：那里工具栏尺寸越界时整份设置作废，徽标的 logo 不受工具栏尺寸影响。
inline bool hud_show_app_logo(const nlohmann::json &preferences) {
  if (!preferences.is_object()) return false;
  const auto found = preferences.find("show_app_logo");
  return found != preferences.end() && found->is_boolean() && found->get<bool>();
}

// 工具栏按顺序画的按钮：0 中/英，11 切换输入方案，2 标点，1 全角，3 简繁，6 设置。`items` 按偏好的顺序排列：character_set、punctuation、fullwidth、settings。表情、手写、屏幕键盘和语音按钮已随对应功能移除。
inline std::vector<int> floating_toolbar_slots(const FloatingToolbarSettings &settings) {
  const auto &items = settings.items;
  std::vector<int> result;
  result.reserve(6);
  if (settings.language) result.push_back(0);
  if (settings.input_scheme) result.push_back(11);
  if (items[1]) result.push_back(2);
  if (items[2]) result.push_back(1);
  if (items[0]) result.push_back(3);
  if (items[3]) result.push_back(6);
  return result;
}

// 图片工具栏向共享布局桥请求的按钮：只取有图片皮肤键的按钮，按 status、fullwidth、punctuation、character_set、settings 的固定顺序。普通条把标点排在全角之前，这里不跟它：布局桥按请求顺序排 buttonFlow 里的按钮，命中测试和读屏顺序也随这个顺序。
inline std::vector<int> floating_toolbar_image_slots(const std::vector<int> &slots) {
  std::vector<int> result;
  for (const int id : {0, 1, 2, 3, 6})
    if (std::find(slots.begin(), slots.end(), id) != slots.end()) result.push_back(id);
  return result;
}

// Only presentation settings cross threads. Keep the newest revision even
// after consumption, so a delayed publisher cannot restore an older layout.
class FloatingToolbarMailbox {
public:
  bool publish(uint64_t revision, FloatingToolbarSettings value) {
    if (!value.valid()) return false;
    std::lock_guard lock(mutex_);
    if (revision_ && revision <= *revision_) return false;
    revision_ = revision;
    pending_ = value;
    return true;
  }
  std::optional<FloatingToolbarSettings> take() {
    std::lock_guard lock(mutex_);
    const auto result = pending_;
    pending_.reset();
    return result;
  }
private:
  std::mutex mutex_;
  std::optional<uint64_t> revision_;
  std::optional<FloatingToolbarSettings> pending_;
};
} // namespace msime::windows
