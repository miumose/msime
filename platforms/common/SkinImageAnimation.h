#pragma once

#include <algorithm>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <optional>
#include <vector>

namespace msime::skin {
// 皮肤已移除：工具栏只使用系统配色和内置图标，不再解码皮肤动图。
struct ImageFrame {
  std::vector<uint8_t> rgba;
  uint32_t delay_ms = 0;
};
struct ImageAnimation {
  uint32_t width = 0;
  uint32_t height = 0;
  uint32_t play_count = 0;
  std::vector<ImageFrame> frames;
  // 所有完整动画帧的 alpha 并集，透明处不能截获另一个窗口的点击。
  std::vector<uint8_t> alpha;

  size_t frame_at(uint64_t elapsed, bool reduce_motion = false) const {
    if (reduce_motion || frames.size() < 2) return 0;
    uint64_t duration = 0;
    for (const auto &frame : frames) duration += frame.delay_ms;
    if (!duration) return 0;
    if (play_count && elapsed / duration >= play_count) return frames.size() - 1;
    elapsed %= duration;
    for (size_t at = 0; at < frames.size(); ++at) {
      if (elapsed < frames[at].delay_ms) return at;
      elapsed -= frames[at].delay_ms;
    }
    return frames.size() - 1;
  }

  static std::optional<ImageAnimation> decode(const std::vector<uint8_t> &,
                                             uint64_t &) {
    return std::nullopt;
  }

  static std::optional<ImageAnimation> load(const std::filesystem::path &directory,
                                           const std::filesystem::path &relative,
                                           uint64_t &remaining_pixels) {
    if (relative.empty() || relative.is_absolute()) return std::nullopt;
    std::error_code error;
    const auto root = std::filesystem::canonical(directory, error);
    if (error) return std::nullopt;
    const auto path = std::filesystem::canonical(root / relative, error);
    if (error) return std::nullopt;
    const auto mismatch = std::mismatch(root.begin(), root.end(), path.begin(), path.end());
    if (mismatch.first != root.end() || mismatch.second == path.end()) return std::nullopt;
    const auto length = std::filesystem::file_size(path, error);
    if (error || !length || length > 8 * 1024 * 1024) return std::nullopt;
    std::ifstream stream(path, std::ios::binary);
    std::vector<uint8_t> bytes(static_cast<size_t>(length));
    if (!stream.read(reinterpret_cast<char *>(bytes.data()), static_cast<std::streamsize>(length)))
      return std::nullopt;
    return decode(bytes, remaining_pixels);
  }
};
} // namespace msime::skin
