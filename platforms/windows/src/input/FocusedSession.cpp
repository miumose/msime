#include "FocusedSession.h"
#include "ipc_negotiation.h"
#include "../../../common/HostApiString.h"
#include "FloatingToolbarVisibilityPolicy.h"
#include "InputSchemeTraits.h"
#include "LifecyclePolicy.h"
#include "PredictionPolicy.h"
#include <ctime>
#include <memory>
#include <stdexcept>
#include <thread>
#include <utility>
#include "FullscreenForeground.h"

namespace msime::windows {
void FocusedSession::check_thread() const {
  if (std::this_thread::get_id() != thread_)
    throw std::logic_error("Wrong focused session thread");
}
bool FocusedSession::prepared(const FocusLease &lease) const {
  return lease_ && composer_ && lease_->epoch == lease.epoch &&
         lease_->token == lease.token &&
         same_ticket(lease_->transport, lease.transport);
}
void FocusedSession::attach_online_query(
    const FocusLease &lease, std::optional<PendingReply> &reply) {
  (void)lease;
  (void)reply;
}

std::optional<nlohmann::json>
FocusedSession::dedicated_english(const FocusLease &lease, bool exit) {
  check_thread();
  if (!prepared(lease) || composer_->has_pending())
    return std::nullopt;
  std::optional<nlohmann::json> result;
  gate_.with_active(lease, [&] {
    result = session_.dedicated_english(lease.epoch, exit);
    if (exit) composer_->cancel();
  });
  return result;
}
std::optional<nlohmann::json>
FocusedSession::set_dedicated_english(const FocusLease &lease, bool enabled) {
  check_thread();
  if (!prepared(lease) || composer_->has_pending())
    return std::nullopt;
  std::optional<nlohmann::json> result;
  gate_.with_active(lease, [&] {
    const auto current = session_.view();
    // 托盘点击不经过 TIP：组字时切换会让 Engine 丢掉组字，而 TIP 手里的组字还留在编辑器里，两边从此对不上。所以只在没有组字、没有候选时切换，组字中的点击什么也不做。
    if (!current.at("editing_text").get<std::string>().empty() ||
        (!current.at("candidates").empty() && !idle_prediction_view(current)))
      return;
    const bool changed = current.at("dedicated_english").get<bool>() != enabled;
    result = session_.set_dedicated_english(lease.epoch, enabled);
    // 切换后回复合成器记下的前缀和译文页属于旧模式，一并作废。
    if (changed) composer_->cancel();
  });
  return result;
}
bool FocusedSession::prepare(const FocusLease &lease) {
  check_thread();
  if (lease.transport.client != client_)
    return false;
  try {
    return gate_.with_pending(lease, [&] {
      if (prepared(lease))
        return;
      if (composer_)
        composer_->cancel();
      preferences_retry_.reset();
      continuation_hide_.clear();
      // 先算好此刻的字段属性再获得焦点：activate 在 msime_client_focus(true) 之前把它原样再报一次，脚本运行时据此确认属性、把 focus_in 交给脚本。激活的会话随后成为背景音乐的所有者，放不放也看这时的属性，密码框和全屏前台都不放。
      refresh_field_traits();
      session_.activate(lease.epoch);
      composer_.emplace(client_, lease.epoch, true,
                        (lease.transport.capabilities & FanyImeProtocol::QuickPhraseCursor) != 0);
      lease_ = lease;
      session_.set_active(true);
    });
  } catch (...) {
    gate_.deactivate(lease);
    composer_.reset();
    lease_.reset();
    throw;
  }
}
std::optional<PendingReply>
FocusedSession::key(const FocusLease &lease, const FanyImeNamedpipeData &packet,
                    ReplyPath path, bool uiless,
                    std::optional<std::string> local_text) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<PendingReply> result;
  gate_.with_active(lease, [&] {
    result = composer_->dispatch(session_, packet, lease.epoch, path, uiless,
                                 std::move(local_text));
    attach_online_query(lease, result);
  });
  return result;
}
std::optional<PendingReply> FocusedSession::edit(const FocusLease &lease,
    const FanyImeNamedpipeData &packet, TsfPreeditStyle style) {
  check_thread();
  if (!prepared(lease)) return std::nullopt;
  std::optional<PendingReply> result;
  gate_.with_active(lease, [&] {
    result = composer_->edit(session_, packet, lease.epoch, style);
    attach_online_query(lease, result);
  });
  return result;
}
std::optional<PendingReply> FocusedSession::basic_key(
    const FocusLease &lease, const FanyImeNamedpipeData &packet,
    TsfPreeditStyle style, std::optional<std::string> local_text) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<PendingReply> result;
  gate_.with_active(lease, [&] {
    result = composer_->basic_key(session_, packet, lease.epoch, style,
                                  std::move(local_text));
    attach_online_query(lease, result);
  });
  return result;
}
std::optional<PendingReply> FocusedSession::toggle_character_set(
    const FocusLease &lease, const FanyImeNamedpipeData &packet, bool enabled,
    const std::function<bool(bool)> &persist) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<PendingReply> result;
  gate_.with_active(lease, [&] {
    result = composer_->toggle_character_set(session_, packet, lease.epoch,
                                              enabled, persist);
    attach_online_query(lease, result);
  });
  return result;
}
std::optional<PendingReply>
FocusedSession::navigate(const FocusLease &lease,
                         const FanyImeNamedpipeData &packet,
                         const NavigationBindings &bindings) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<PendingReply> result;
  gate_.with_active(lease, [&] {
    result = composer_->navigate(session_, packet, lease.epoch, bindings);
    attach_online_query(lease, result);
  });
  return result;
}
bool FocusedSession::confirm(const FocusLease &lease, uint64_t request) {
  check_thread();
  if (!prepared(lease))
    return false;
  // Read before the acknowledgement, which clears the pending reply.
  const bool auto_commit = composer_->pending().worker.has_value();
  const bool confirmed = gate_.with_active(lease, [&] {
    composer_->confirm_delivery(client_, lease.epoch, request);
    if (preferences_retry_) {
      session_.update_preferences(lease.epoch, preferences_retry_->dump());
      preferences_retry_.reset();
    }
  });
  if (confirmed) {
    if (auto_commit)
      continuation_hide_.arm(ContinuationHide::Clock::now());
  }
  return confirmed;
}
std::optional<PendingReply>
FocusedSession::select_candidate(const FocusLease &lease, uint64_t session,
                                 uint64_t generation, size_t index,
                                 uint64_t translation_page,
                                 const std::optional<std::string> &expanded_text) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<PendingReply> result;
  gate_.with_active(lease, [&] {
    // 指针点选开启新的一轮：先报 MSIME_KEY_POINTER，不出声、不画。
    (void)session_.key(MSIME_KEY_POINTER);
    result = composer_->select_candidate(session_, session, generation, index,
                                         translation_page,
        (lease.transport.capabilities & FanyImeProtocol::CandidateContinuation) != 0,
        expanded_text);
    attach_online_query(lease, result);
  });
  return result;
}
std::optional<CandidateExpansionFetch>
FocusedSession::expanded_candidates(const FocusLease &lease) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<CandidateExpansionFetch> result;
  gate_.with_active(lease, [&] {
    // 回复还没确认送达时不动引擎的代次：送达确认和随后的选词都按那一代核对。释义页上的是义项，不是 Engine 的列表。
    if (composer_->has_pending() || composer_->translation_page() != 0 ||
        !session_.input_enabled())
      return;
    const auto current = session_.view();
    if (!current.at("focused").get<bool>() ||
        current.at("editing_text").get<std::string>().empty() ||
        current.at("candidates").empty() ||
        scheme::KeyboardOnlyCandidateList(static_cast<int>(current.value("scheme", 0u))))
      return;
    CandidateExpansionFetch fetch;
    fetch.snapshot = session_.all_candidates(lease.epoch);
    if (fetch.snapshot.at("generation") != current.at("generation"))
      fetch.view = session_.view();
    result = std::move(fetch);
  });
  return result;
}
std::optional<nlohmann::json>
FocusedSession::candidate_action(const FocusLease &lease, uint64_t session,
                                 uint64_t generation, size_t index,
                                 CandidateAction action, uint8_t position) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<nlohmann::json> result;
  gate_.with_active(lease, [&] {
    // 释义页打开时 Engine 的列表还在底下、同 session 同 generation：这时的动作不管点的是哪一行，都会落到 Engine 同一序号的候选上，而用户看到的是释义。
    if (composer_->has_pending() || composer_->translation_page() != 0)
      return;
    const auto current = session_.view();
    if (current.at("session").get<uint64_t>() != session ||
        current.at("generation").get<uint64_t>() != generation ||
        !current.at("focused").get<bool>())
      return;
    bool found = false;
    for (const auto &candidate : current.at("candidates")) {
      const auto &id = candidate.at("id");
      if (id.at("session").get<uint64_t>() == session &&
          id.at("generation").get<uint64_t>() == generation &&
          id.at("index").get<size_t>() == index) {
        found = true;
        break;
      }
    }
    if (found)
      result = session_.candidate_action(lease.epoch, generation, index,
                                         action, position);
  });
  return result;
}
std::optional<nlohmann::json>
FocusedSession::page_candidate(const FocusLease &lease, uint64_t session,
                               uint64_t generation, bool previous,
                               unsigned steps) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<nlohmann::json> result;
  gate_.with_active(lease, [&] {
    // 释义页打开时翻页不起作用，和方向键、翻页键在释义页上一样（ReplyComposer::translation_page_key）：翻的会是底下的 Engine 列表，候选窗换成它的下一页，释义页却还开着，数字键上屏的仍是看不见的义项。
    if (composer_->has_pending() || composer_->translation_page() != 0)
      return;
    const auto current = session_.view();
    if (!current.at("focused").get<bool>() ||
        current.at("session").get<uint64_t>() != session ||
        current.at("generation").get<uint64_t>() != generation)
      return;
    // A page turned here would leave the list the TIP drives from its own host session on another page.
    if (scheme::KeyboardOnlyCandidateList(static_cast<int>(current.value("scheme", 0u))))
      return;
    // 指针翻页同样开启新的一轮。
    (void)session_.key(MSIME_KEY_POINTER);
    result = session_.page_candidate(lease.epoch, session, generation,
                                     previous, steps);
  });
  return result;
}
bool FocusedSession::confirm_ui(const FocusLease &lease, uint64_t generation) {
  check_thread();
  if (!prepared(lease))
    return false;
  const bool confirmed = gate_.with_active(lease, [&] {
    composer_->confirm_ui_delivery(client_, lease.epoch, generation);
    if (preferences_retry_) {
      session_.update_preferences(lease.epoch, preferences_retry_->dump());
      preferences_retry_.reset();
    }
  });
  return confirmed;
}
std::optional<PendingReply> FocusedSession::pending(const FocusLease &lease) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<PendingReply> result;
  gate_.with_active(lease, [&] {
    if (composer_->has_pending())
      result = composer_->pending();
  });
  return result;
}
bool FocusedSession::set_input_enabled(const FocusLease &lease, bool enabled) {
  check_thread();
  if (!prepared(lease))
    return false;
  return gate_.with_active(lease, [&] {
    if (composer_->has_pending())
      throw std::logic_error("Input mode changed before reply delivery");
    session_.set_input_enabled(lease.epoch, enabled);
    if (!enabled)
      composer_->cancel();
  });
}
bool FocusedSession::set_chinese_punctuation(const FocusLease &lease, bool enabled) {
  check_thread();
  if (!prepared(lease))
    return false;
  return gate_.with_active(lease, [&] {
    if (composer_->has_pending())
      throw std::logic_error("Punctuation mode changed before reply delivery");
    session_.set_chinese_punctuation(lease.epoch, enabled);
  });
}
bool FocusedSession::balance_paired_punctuation(const FocusLease &lease,
                                                uint8_t opening) {
  check_thread();
  if (!prepared(lease))
    return false;
  return gate_.with_active(lease, [&] {
    session_.balance_paired_punctuation(lease.epoch, opening);
  });
}
bool FocusedSession::cancel_composition(const FocusLease &lease) {
  check_thread();
  if (!prepared(lease))
    return false;
  return gate_.with_active(lease, [&] {
    if (composer_->has_pending())
      throw std::logic_error("Composition cancelled before reply delivery");
    session_.cancel_composition(lease.epoch);
    composer_->cancel();
    continuation_hide_.clear();
  });
}
HideCandidateDisposition
FocusedSession::hide_candidate(const FocusLease &lease) {
  check_thread();
  if (!prepared(lease))
    return HideCandidateDisposition::Rejected;
  HideCandidateDisposition disposition = HideCandidateDisposition::Rejected;
  gate_.with_active(lease, [&] {
    if (composer_->has_pending())
      throw std::logic_error("Composition hidden before reply delivery");
    const auto current = session_.view();
    if (continuation_hide_.consume(ContinuationHide::Clock::now()) &&
        !current.at("editing_text").get<std::string>().empty()) {
      disposition = HideCandidateDisposition::Suppressed;
      return;
    }
    // 上屏后联想没有组字。上屏那一下 TIP 结束组字时总会发一个隐藏，它比 TIP 在光标处显示联想的 Show 先到；这时取消会把刚出来的联想清掉。TIP 为这一代发过 Show 之后的隐藏才是关闭联想。
    if (idle_prediction_view(current) &&
        current.at("generation").get<uint64_t>() != shown_prediction_generation_) {
      disposition = HideCandidateDisposition::PredictionKept;
      return;
    }
    shown_prediction_generation_ = 0;
    session_.cancel_composition(lease.epoch);
    composer_->cancel();
    disposition = HideCandidateDisposition::Cancelled;
  });
  return disposition;
}
void FocusedSession::show_candidate(const FocusLease &lease) {
  check_thread();
  if (!prepared(lease))
    return;
  gate_.with_active(lease, [&] {
    const auto current = session_.view();
    if (idle_prediction_view(current))
      shown_prediction_generation_ = current.at("generation").get<uint64_t>();
  });
}
bool FocusedSession::reset_cache() {
  check_thread();
  try {
    session_.reset_cache();
    return true;
  } catch (...) {
    return false;
  }
}
bool FocusedSession::cancel(const FocusLease &lease) {
  check_thread();
  if (!prepared(lease))
    return false;
  // This can be cleanup for a previous owner; don't invalidate a new lease.
  gate_.deactivate(lease);
  composer_->cancel();
  preferences_retry_.reset();
  continuation_hide_.clear();
  session_.set_active(false);
  session_.deactivate(lease.epoch);
  composer_.reset();
  lease_.reset();
  return true;
}
bool FocusedSession::cancel_focus_token(uint64_t token) {
  check_thread();
  if (!token)
    return false;
  // Not focused, or focused under a later token: the session the DLL named is
  // already gone, so there is nothing left to tear down.
  if (!lease_ || lease_->token != token)
    return true;
  return cancel(*lease_);
}
std::optional<nlohmann::json>
FocusedSession::rerank_settled(const FocusLease &lease) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<nlohmann::json> result;
  gate_.with_active(lease, [&] { result = session_.rerank_settled(lease.epoch); });
  return result;
}
std::optional<nlohmann::json>
FocusedSession::update_preferences(const FocusLease &lease,
                                   const std::string &snapshot) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<nlohmann::json> result;
  gate_.with_active(lease, [&] {
    if (!composer_->has_pending())
      result = session_.update_preferences(lease.epoch, snapshot);
  });
  return result;
}
bool FocusedSession::queue_current_preferences(const std::string &snapshot) {
  check_thread();
  return lease_ && queue_preferences(*lease_, snapshot);
}
bool FocusedSession::queue_preferences(const FocusLease &lease,
                                       const std::string &snapshot) {
  check_thread();
  if (!prepared(lease))
    return false;
  return gate_.with_active(lease, [&] {
    if (snapshot.empty() || snapshot.size() > 1048576)
      throw std::invalid_argument("Invalid preferences snapshot size");
    auto document = nlohmann::json::parse(snapshot);
    if (!document.is_object() || !document.contains("revision") ||
        !document.at("revision").is_number_unsigned() ||
        document.value("format_version", 0) != 1 ||
        !document.contains("preferences") ||
        !document.at("preferences").is_object())
      throw std::invalid_argument("Invalid preferences snapshot envelope");
    if (preferences_retry_) {
      const auto revision = document.at("revision").get<uint64_t>();
      const auto previous = preferences_retry_->at("revision").get<uint64_t>();
      if (revision < previous ||
          (revision == previous && document != *preferences_retry_))
        throw std::invalid_argument("Stale or conflicting queued preferences");
    }
    if (composer_->has_pending())
      preferences_retry_ = std::move(document);
    else {
      session_.update_preferences(lease.epoch, snapshot);
      preferences_retry_.reset();
    }
  });
}
std::optional<PendingReply> FocusedSession::configured_key(
    const FocusLease &lease, const FanyImeNamedpipeData &packet,
    TsfPreeditStyle style, const NavigationBindings &bindings,
    std::optional<std::string> local_text, WordCharacterBinding word_binding) {
  check_thread();
  if (!prepared(lease))
    return std::nullopt;
  std::optional<PendingReply> result;
  gate_.with_active(lease, [&] {
    // 不论输入法收不收这个键（英文模式也算），都是用户在打字：空闲隐藏的工具栏据此回来。
    ServerKeyActivity::instance().note();
    // 生命周期的一轮从这个键开始，排在引擎处理之前：先比较一次字段属性，变了先报。按键音和打字特效已随音效包移除。
    refresh_field_traits();
    {
      // TIP 已经从自己的宿主会话把这次的上屏写进文档（韩文、注音、越南文、藏文，整句改字）：这次派发的上屏只能观察。本地回车（ReplyPath::LocalCommit）要等路由选定才知道，由 ReplyComposer::dispatch 自己打开。上屏音只来自 Server 这一个会话的核心，TIP 的会话不选择加入，所以同一次上屏不会响两遍。
      const CommitObserveOnly observe(session_, session_.input_enabled() && tip_writes_commits(session_.view()));
      result = composer_->configured_key(session_, packet, lease.epoch, style,
                                         bindings, std::move(local_text),
                                         word_binding);
    }
    // 上屏音和上屏特效已移除；之后才构建在线查询，不给它们添延迟。
    attach_online_query(lease, result);
  });
  return result;
}
void FocusedSession::refresh_field_traits() {
  // 全屏应用在前台（游戏、视频、演示，用户没想在那里听打字声）作为 FULLSCREEN 交给核心：声音和背景音乐静音，特效和连击照常。
  const bool fullscreen = fullscreen_check_ ? fullscreen_check_() : foreground_is_fullscreen(GetForegroundWindow());
  session_.set_field_traits(server_field_traits(tip_field_traits_, fullscreen));
}
void FocusedSession::set_field_traits(uint32_t traits) {
  check_thread();
  tip_field_traits_ = traits & FanyImeFieldTraits::Reported;
  refresh_field_traits();
}
bool FocusedSession::passthrough_key(uint64_t token, uint32_t key_class) {
  check_thread();
  // 只认此刻持有焦点的那次激活：令牌对不上的是已经离开的会话，或者别的线程。
  if (!token || !lease_ || lease_->token != token || !prepared(*lease_))
    return false;
  bool active = false;
  gate_.with_active(*lease_, [&] {
    ServerKeyActivity::instance().note();
    (void)key_class;
    // 按键音已移除；这里只确认这个客户端仍持有焦点，TIP 照常把键交给应用。
    active = session_.input_enabled();
  });
  return active;
}
} // namespace msime::windows
