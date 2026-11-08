#pragma once
#include "../../compat/WindowsDpi.h"
#include <windows.h>

namespace msime::windows {
// 一块显示器的有效 DPI，取不到时退回系统 DPI，从不为 0。调用方要在每显示器
// 感知的线程上下文里，否则 GetDpiForMonitor 一律回答 96。
inline UINT monitor_effective_dpi(HMONITOR monitor) {
  using GetDpiForMonitorFn = HRESULT(WINAPI *)(HMONITOR, int, UINT *, UINT *);
  static GetDpiForMonitorFn query = [] {
    const HMODULE shcore = LoadLibraryW(L"shcore.dll");
    if (!shcore)
      return static_cast<GetDpiForMonitorFn>(nullptr);
    return reinterpret_cast<GetDpiForMonitorFn>(
        reinterpret_cast<void *>(GetProcAddress(shcore, "GetDpiForMonitor")));
  }();
  UINT dpi_x = 0;
  UINT dpi_y = 0;
  if (query && SUCCEEDED(query(monitor, 0, &dpi_x, &dpi_y)) && dpi_x)
    return dpi_x;
  const UINT system = msime::windows::compat::system_dpi();
  return system ? system : 96;
}
} // namespace msime::windows
