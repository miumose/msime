#include "TsfCrashCapture.h"

namespace msime::tsf {

// 使用统计与 WER 崩溃上报已移除，TSF 不再持有任何崩溃捕获租约。
TsfCrashCapture::TsfCrashCapture() noexcept = default;
TsfCrashCapture::~TsfCrashCapture() noexcept = default;

} // namespace msime::tsf
