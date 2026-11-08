#pragma once

#include "msime_client.h"
#include "../../common/HostApiString.h"
#include <nlohmann/json.hpp>
#include <stdexcept>

namespace msime::windows {
inline nlohmann::json panel_response(char *raw) {
  auto owned = msime::host_api::own_string(raw);
  if (!owned) throw std::runtime_error("Missing panel host response");
  const auto document = nlohmann::json::parse(owned.get());
  if (!document.at("ok").get<bool>()) throw std::runtime_error("Panel host operation failed");
  return document.at("value");
}

// TIP 与 Server 复用同一个来源/布局切换规则；候选与多义解码仍只由共享引擎负责。
inline nlohmann::json panel_character(uint64_t session, const nlohmann::json &,
                                      uint8_t letter, bool shift, bool grouped) {
  if (grouped)
    throw std::invalid_argument("Grouped panel schemes were removed");
  return panel_response(msime_client_character(session, letter, shift));
}
} // namespace msime::windows
