#pragma once

// Emoji are drawn through DirectWrite's own font fallback. The bundled
// NotoEmoji font and the HarfBuzz shaping path existed only for Windows 7 and
// are gone with that product; every supported Windows version is 10 or later.

#include <d2d1.h>
#include <dwrite_2.h>

namespace msime::windows::compat {

inline HRESULT CreateTextLayoutWithEmoji(IDWriteFactory* factory, const wchar_t* text,
                                         UINT32 length, IDWriteTextFormat* format,
                                         FLOAT width, FLOAT height,
                                         IDWriteTextLayout** layout) {
  if (!factory) return E_POINTER;
  return factory->CreateTextLayout(text, length, format, width, height, layout);
}

inline void DrawTextWithEmoji(IDWriteFactory* factory, ID2D1RenderTarget* target,
                              const wchar_t* text, UINT32 length,
                              IDWriteTextFormat* format, const D2D1_RECT_F& rectangle,
                              ID2D1Brush* brush,
                              D2D1_DRAW_TEXT_OPTIONS options = D2D1_DRAW_TEXT_OPTIONS_NONE,
                              DWRITE_MEASURING_MODE mode = DWRITE_MEASURING_MODE_NATURAL) {
  (void)factory;
  target->DrawText(text, length, format, rectangle, brush, options, mode);
}

}  // namespace msime::windows::compat
