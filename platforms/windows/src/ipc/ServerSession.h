#pragma once
#include "CandidateAction.h"
#include "NavigationPolicy.h"
#include "WordCharacterPolicy.h"
#include "windows_ipc.h"
#include <nlohmann/json.hpp>
#include <string>
#include <thread>

namespace msime::windows {
struct KeyResult {
  uint64_t client_id;
  uint64_t activation_epoch;
  uint64_t request_id;
  bool reply_expected;
  nlohmann::json transition;
};
struct NavigationResult {
  KeyResult key;
  NavigationReply direction;
};
struct WordCharacterResult {
  KeyResult key;
  bool exact;
};

// Lives on the Server input queue, never inside the injected TSF DLL. The pipe
// router authenticates/negotiates a client before constructing its session, and
// assigns monotonically increasing focus epochs. This class is not a pipe
// server.
class ServerSession final {
public:
  ServerSession(uint64_t client_id, const std::string &prepared_options);
  ~ServerSession();
  ServerSession(const ServerSession &) = delete;
  ServerSession &operator=(const ServerSession &) = delete;
  nlohmann::json activate(uint64_t epoch);
  nlohmann::json deactivate(uint64_t epoch);
  void cancel_composition(uint64_t epoch);
  // Server 自己合成、已经写进文档的上屏（Ctrl+Enter 和释义列的译文、翻译页的选择）之后丢弃组字：对会话的作用同 cancel_composition，但发的是命令 19（MSIME_CANCEL_AFTER_HOST_COMMIT），生命周期里组字的结束原因记 HostCommit 而不是 Cancel。结果里没有上屏，调用方自己合成 {commit, view}。
  void cancel_after_host_commit(uint64_t epoch);
  // MSIME_FINISH_COMPOSITION：当前组字成为上屏内容。韩文、注音、越南文和藏文用它处理那些结束组字但自身不带字符的按键。
  nlohmann::json finish_composition(uint64_t epoch);
  // One MsimeCommand, for a key whose command does not follow from translate_key: the keys of the Korean Hanja and Zhuyin lists (KoreanHanjaKey.h). Requires input enabled.
  nlohmann::json command(uint64_t epoch, uint32_t command);
  // Clear the Engine candidate-provider cache without requiring focus.
  void reset_cache();
  void set_input_enabled(uint64_t epoch, bool enabled);
  void set_chinese_punctuation(uint64_t epoch, bool enabled);
  // The TSF inserted the closing half of a pair whose opening the Engine resolved, so pay back the nesting the opening advanced. A no-op when the count is already zero.
  void balance_paired_punctuation(uint64_t epoch, uint8_t opening);
  // 即时切换宿主和共享运行时的简繁投影，保留组字；落盘由调用方负责。
  nlohmann::json toggle_traditional_output(uint64_t epoch);
  // Queue-owned runtime operation, never a write to default preferences.
  // Exit cancels composition without committing and returns the Engine view.
  nlohmann::json dedicated_english(uint64_t epoch, bool exit);
  // Ctrl+Shift+E: flip the dedicated English mode. Like the reference, the open composition is discarded rather than committed. Returns the new view.
  nlohmann::json toggle_dedicated_english(uint64_t epoch);
  // 托盘「英文候选模式」：把 Engine 的英文模式设成 `enabled`。和 Ctrl+Shift+E 一样丢弃正在组的字；已经是这个状态时什么也不动，原样返回视图。
  nlohmann::json set_dedicated_english(uint64_t epoch, bool enabled);
  bool input_enabled() const {
    check_thread();
    return input_enabled_;
  }
  KeyResult key(const FanyImeNamedpipeData &packet, uint64_t epoch);
  // Rebuild a deleted creating-word segment through Engine's ordinary
  // character path. This is the Windows Ctrl+Backspace restoration boundary;
  // the host composer owns the selected-prefix history.
  KeyResult restore_raw(uint64_t epoch, uint64_t request,
                        const std::string &raw);
  KeyResult punctuation(const FanyImeNamedpipeData &packet, uint64_t epoch);
  std::optional<WordCharacterResult>
  word_character(const FanyImeNamedpipeData &packet, uint64_t epoch,
                 WordCharacterBinding binding);
  std::optional<NavigationResult> navigate(const FanyImeNamedpipeData &packet,
                                           uint64_t epoch,
                                           const NavigationBindings &bindings);
  nlohmann::json select(uint64_t epoch, uint64_t generation, size_t index);
  // 卷轴展开网格里的候选：index 是 all_candidates 那份完整列表里的下标，不在当前页上，走 msime_client_select_any_candidate；source 是那一项的来源，云候选这类整段上屏的处理和 select 相同。
  nlohmann::json select_any(uint64_t epoch, uint64_t generation, size_t index, uint8_t source);
  // msime_client_all_candidates：这一代的全部候选，下标是全局的。会让引擎放出扣着的尾部，这时代次前进，调用方要重新读 view 并发布。
  nlohmann::json all_candidates(uint64_t epoch);
  nlohmann::json candidate_action(uint64_t epoch, uint64_t generation,
                                  size_t index, CandidateAction action,
                                  uint8_t position = 0);
  /// Re-rank with the settled model; the refreshed view, or nothing when the
  /// order did not move or no settled model is installed.
  std::optional<nlohmann::json> rerank_settled(uint64_t epoch);
  nlohmann::json update_preferences(uint64_t epoch,
                                    const std::string &snapshot);
  bool traditional_output() const {
    check_thread();
    return traditional_output_;
  }
  nlohmann::json page_candidate(uint64_t epoch, uint64_t session,
                                uint64_t generation, bool previous,
                                unsigned steps);
  nlohmann::json view() const;
  // One physical key down. Without FEEDBACK the core only advances input; the
  // Server no longer draws typing effects or plays key sounds.
  uint32_t key(uint32_t key);
  // 交给核心的字段属性（server_field_traits 的值），和上次报的一样时不调 FFI。activate 和 set_input_enabled 在获得焦点之前另外原样再报一次（confirm_field_traits_before_focus），调用方要先用它报好此刻的属性。
  void set_field_traits(uint32_t traits);
  uint32_t field_traits() const {
    check_thread();
    return field_traits_;
  }
  // TIP 已经从自己的宿主会话把文字写进文档的那一次派发前后开关 COMMIT_OBSERVE_ONLY（msime_client_set_lifecycle）：这些上屏只能观察、不能改写。只设标志，不会抛出。平时经 CommitObserveOnly 开关。
  void set_commit_observe_only(bool observe_only) noexcept;
  bool commit_observe_only() const noexcept { return commit_observe_only_; }
  // 输入法在这个客户端上激活或停用（msime_client_set_active）：记下来，偏好更新后再说一次。
  void set_active(bool active);

private:
  void check_thread() const;
  void check_active(uint64_t epoch) const;
  // select 与 select_any 共用的选词：completes_composition 的候选整段上屏，再丢掉剩下的读音。
  nlohmann::json select_with(uint64_t generation, size_t index, bool completes_composition,
                             bool any);
  // 取消结果让组字仍然打开时（韩文、注音的列表关闭，越南文、藏文的原文重新显示，全拼、双拼退出整句改字），再发一次同一个取消命令（MSIME_CANCEL 或 MSIME_CANCEL_AFTER_HOST_COMMIT）丢弃它，真正结束组字的那次派发仍按这个命令归类；其他结果不变。
  nlohmann::json cancel_again(nlohmann::json result, uint32_t command);
  // 移动高亮的命令之后调用：高亮落在候选窗不画的占位席位上时，按 `common/CandidateSources.h` 的 leave_hidden_seat 让过去，`result` 的 view 换成让过之后的。`before` 是这次命令之前的视图。
  nlohmann::json leave_hidden_seat(nlohmann::json result, const nlohmann::json &before);
  // cancel_composition 和 cancel_after_host_commit 的共同实现：发 `command`，必要时再发一次，确认组字已经没了。
  void discard_composition(uint64_t epoch, uint32_t command);
  const std::thread::id thread_ = std::this_thread::get_id();
  uint64_t client_;
  uint64_t session_ = 0;
  uint64_t epoch_ = 0;
  bool active_ = false;
  bool input_enabled_ = true;
  bool traditional_output_ = false;
  nlohmann::json preferences_ = nlohmann::json::object();
  bool lifecycle_active_ = false;
  uint32_t field_traits_ = 0;
  bool commit_observe_only_ = false;
  // 每次 msime_client_focus(true) 之前调用：把 field_traits_ 原样再报一次，不去重。
  void confirm_field_traits_before_focus();
};

// TIP 已经从自己的宿主会话把文字写进文档的那一次派发：派发前打开 COMMIT_OBSERVE_ONLY，离开作用域时关掉，派发抛出也一样。可以嵌套：外层已经打开时，内层什么都不做，也不会提前关掉外层的。
class CommitObserveOnly final {
public:
  CommitObserveOnly(ServerSession &session, bool active)
      : session_(active && !session.commit_observe_only() ? &session : nullptr) {
    if (session_)
      session_->set_commit_observe_only(true);
  }
  ~CommitObserveOnly() {
    if (session_)
      session_->set_commit_observe_only(false);
  }
  CommitObserveOnly(const CommitObserveOnly &) = delete;
  CommitObserveOnly &operator=(const CommitObserveOnly &) = delete;

private:
  ServerSession *session_;
};
} // namespace msime::windows
