//! 纯推导和 `LifecycleState` 的单元测试。经 C ABI 的黄金轨迹在 crate 的 `tests.rs` 里。

use super::*;

fn idle() -> Sample {
    Sample::default()
}

fn composing(composition_epoch: u64, candidates_epoch: u64, visible: bool) -> Sample {
    Sample {
        composing: true,
        composition_epoch,
        candidates_epoch,
        candidates_visible: visible,
    }
}

fn page(count: u16) -> PageSample {
    PageSample {
        count,
        page: 0,
        highlighted: Some(0),
    }
}

fn typed(chars: u32) -> CommitSample {
    CommitSample {
        chars,
        plain: true,
        typed: true,
    }
}

fn picked(how: SelectionHow, outcome: SelectionOutcome) -> DispatchFacts {
    DispatchFacts {
        selection: Some(SelectionFact {
            seat: 0,
            source: 0,
            how,
            outcome,
        }),
        ..DispatchFacts::default()
    }
}

/// 普通字段：属性已知，没有任何限制位。
fn ordinary() -> Context {
    Context {
        flags: 0,
        traits: TRAIT_KNOWN,
        pieces_held: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn events(
    call: Call,
    context: Context,
    pre: Sample,
    post: Sample,
    facts: DispatchFacts,
    commit: Option<CommitSample>,
    page: PageSample,
) -> Vec<LifecycleEvent> {
    let mut events = Vec::new();
    derive(call, context, pre, post, facts, commit, page, |event| {
        events.push(event)
    });
    events
}

fn dispatch(class: InputClass) -> Call {
    Call::Dispatch(class)
}

#[test]
fn the_first_key_starts_updates_and_shows() {
    let events = events(
        dispatch(InputClass::Character),
        ordinary(),
        idle(),
        composing(1, 1, true),
        DispatchFacts::default(),
        None,
        page(5),
    );
    assert_eq!(
        events,
        [
            LifecycleEvent::CompositionStart,
            LifecycleEvent::CompositionUpdate,
            LifecycleEvent::CandidatesShown {
                count: 5,
                page: 0,
                highlighted: Some(0)
            },
        ]
    );
}

#[test]
fn a_pick_that_ends_the_composition_runs_the_whole_commit_sequence() {
    let events = events(
        dispatch(InputClass::Selection),
        ordinary(),
        composing(4, 4, true),
        idle(),
        picked(SelectionHow::Highlighted, SelectionOutcome::Committed),
        Some(typed(2)),
        PageSample::default(),
    );
    assert_eq!(
        events,
        [
            LifecycleEvent::CandidateSelected(SelectionFact {
                seat: 0,
                source: 0,
                how: SelectionHow::Highlighted,
                outcome: SelectionOutcome::Committed,
            }),
            LifecycleEvent::BeforeCommit {
                origin: CommitOrigin::Selection,
                body_chars: 2,
                tail_chars: 0,
                rewritable: true,
            },
            LifecycleEvent::Commit {
                origin: CommitOrigin::Selection,
                chars: 2,
                rewritten: false,
                continues: false,
                generated: false,
            },
            LifecycleEvent::CompositionEnd(EndReason::Commit),
            LifecycleEvent::CandidatesHidden(EndReason::Commit),
        ]
    );
}

/// 任何组合下 start 和 end 都不会在同一次调用里一起出现，一次调用最多一个候选事件；同时带着上屏和新组字时既不 end 也不 start。
#[test]
fn start_and_end_never_appear_together_and_continuation_suppresses_both() {
    let samples = [
        idle(),
        composing(1, 1, false),
        composing(1, 1, true),
        composing(2, 2, true),
    ];
    for pre in samples {
        for post in samples {
            for commit in [None, Some(typed(1))] {
                for call in [
                    dispatch(InputClass::Character),
                    Call::Focus(false),
                    Call::Focus(true),
                    Call::SideChannel,
                ] {
                    let events = events(
                        call,
                        ordinary(),
                        pre,
                        post,
                        DispatchFacts::default(),
                        commit,
                        page(1),
                    );
                    let count = |wanted: fn(&LifecycleEvent) -> bool| {
                        events.iter().filter(|event| wanted(event)).count()
                    };
                    let starts = count(|e| matches!(e, LifecycleEvent::CompositionStart));
                    let ends = count(|e| matches!(e, LifecycleEvent::CompositionEnd(_)));
                    let views = count(|e| {
                        matches!(
                            e,
                            LifecycleEvent::CandidatesShown { .. }
                                | LifecycleEvent::CandidatesHidden(_)
                        )
                    });
                    assert!(starts + ends <= 1, "{pre:?} {post:?} {events:?}");
                    assert!(views <= 1, "{events:?}");
                    assert_eq!(ends, usize::from(pre.composing && !post.composing));
                    let continues = commit.is_some() && pre.composing && post.composing;
                    if continues {
                        assert!(events.iter().any(|event| matches!(
                            event,
                            LifecycleEvent::Commit {
                                continues: true,
                                ..
                            }
                        )));
                    }
                    assert_eq!(starts, usize::from(!pre.composing && post.composing));
                }
            }
        }
    }
}

#[test]
fn a_korean_syllable_that_continues_commits_without_ending() {
    let events = events(
        dispatch(InputClass::Character),
        ordinary(),
        composing(3, 0, false),
        composing(4, 0, false),
        DispatchFacts::default(),
        Some(typed(1)),
        PageSample::default(),
    );
    assert_eq!(
        events,
        [
            LifecycleEvent::BeforeCommit {
                origin: CommitOrigin::SchemeText,
                body_chars: 1,
                tail_chars: 0,
                rewritable: false,
            },
            LifecycleEvent::Commit {
                origin: CommitOrigin::SchemeText,
                chars: 1,
                rewritten: false,
                continues: true,
                generated: false,
            },
            LifecycleEvent::CompositionUpdate,
        ]
    );
}

#[test]
fn the_commit_origin_follows_the_facts() {
    let origin = |call: Call, facts: DispatchFacts| {
        events(
            call,
            ordinary(),
            composing(1, 1, true),
            idle(),
            facts,
            Some(typed(2)),
            PageSample::default(),
        )
        .into_iter()
        .find_map(|event| match event {
            LifecycleEvent::Commit { origin, .. } => Some(origin),
            _ => None,
        })
        .unwrap()
    };
    let call = dispatch(InputClass::Selection);
    assert_eq!(
        origin(
            call,
            picked(SelectionHow::Select, SelectionOutcome::Committed)
        ),
        CommitOrigin::Selection
    );
    assert_eq!(
        origin(
            call,
            picked(SelectionHow::Select, SelectionOutcome::Partial)
        ),
        CommitOrigin::Selection
    );
    let flushed = DispatchFacts {
        held_flushed: true,
        ..DispatchFacts::default()
    };
    assert_eq!(origin(call, flushed), CommitOrigin::HeldFlush);
    let raw = DispatchFacts {
        raw: true,
        ..DispatchFacts::default()
    };
    assert_eq!(origin(dispatch(InputClass::Commit), raw), CommitOrigin::Raw);
    assert_eq!(
        origin(dispatch(InputClass::Selection), DispatchFacts::default()),
        CommitOrigin::SchemeText
    );
    // 失焦上屏不论事实如何都是 FocusLoss，而且没有 before_commit。
    assert_eq!(origin(Call::Focus(false), flushed), CommitOrigin::FocusLoss);
    // 命令 20 替失焦结束组字，引擎即使记了选中候选也按失焦上屏算。
    assert_eq!(
        origin(
            dispatch(InputClass::FocusLoss),
            picked(SelectionHow::Finish, SelectionOutcome::Committed)
        ),
        CommitOrigin::FocusLoss
    );
    let blur = events(
        Call::Focus(false),
        ordinary(),
        composing(1, 1, false),
        idle(),
        flushed,
        Some(typed(1)),
        PageSample::default(),
    );
    assert!(!blur
        .iter()
        .any(|event| matches!(event, LifecycleEvent::BeforeCommit { .. })));
    assert_eq!(blur.last(), Some(&LifecycleEvent::FocusOut));
}

#[test]
fn only_a_plain_typed_pick_in_an_ordinary_field_is_rewritable() {
    let rewritable = |context: Context, facts: DispatchFacts, commit: CommitSample| {
        events(
            dispatch(InputClass::Selection),
            context,
            composing(1, 1, true),
            idle(),
            facts,
            Some(commit),
            PageSample::default(),
        )
        .into_iter()
        .find_map(|event| match event {
            LifecycleEvent::BeforeCommit { rewritable, .. } => Some(rewritable),
            _ => None,
        })
        .unwrap()
    };
    let pick = picked(SelectionHow::Select, SelectionOutcome::Committed);
    assert!(rewritable(ordinary(), pick, typed(2)));
    // 原样上屏和暂存段送出也可改写；方案自己的文本不行。
    let raw = DispatchFacts {
        raw: true,
        ..DispatchFacts::default()
    };
    assert!(rewritable(ordinary(), raw, typed(2)));
    assert!(!rewritable(ordinary(), DispatchFacts::default(), typed(2)));
    // 以词定字一律不可改写。
    let edge = picked(SelectionHow::Edge, SelectionOutcome::Committed);
    assert!(!rewritable(ordinary(), edge, typed(1)));
    // 本地模式里的上屏、引擎生成的文本不可改写。
    let in_mode = CommitSample {
        plain: false,
        ..typed(2)
    };
    assert!(!rewritable(ordinary(), pick, in_mode));
    let generated = CommitSample {
        typed: false,
        ..typed(2)
    };
    assert!(!rewritable(ordinary(), pick, generated));
    // 只能观察、属性未知、SECURE、PRIVATE、ELEVATED 都不可改写。
    let observe = Context {
        flags: COMMIT_OBSERVE_ONLY,
        ..ordinary()
    };
    assert!(!rewritable(observe, pick, typed(2)));
    assert!(!rewritable(Context::default(), pick, typed(2)));
    for restricted in [TRAIT_SECURE, TRAIT_PRIVATE, TRAIT_ELEVATED] {
        let context = Context {
            traits: TRAIT_KNOWN | restricted,
            ..ordinary()
        };
        assert!(!rewritable(context, pick, typed(2)), "{restricted:#x}");
    }
    // 其余属性不影响。
    let fullscreen = Context {
        traits: TRAIT_KNOWN | TRAIT_FULLSCREEN | TRAIT_NO_SPELLCHECK,
        ..ordinary()
    };
    assert!(rewritable(fullscreen, pick, typed(2)));
    // 宿主暂存部分选词时（Windows），部分选词的那一段、以及暂存过一段之后补齐整个词的上屏都不可改写；宿主不暂存时部分选词照样可改写。
    let partial = picked(SelectionHow::Select, SelectionOutcome::Partial);
    assert!(rewritable(ordinary(), partial, typed(1)));
    let holds = Context {
        flags: HOST_HOLDS_PIECES,
        ..ordinary()
    };
    assert!(!rewritable(holds, partial, typed(1)));
    assert!(rewritable(holds, pick, typed(1)));
    let held = Context {
        pieces_held: true,
        ..holds
    };
    assert!(!rewritable(held, pick, typed(1)));
}

#[test]
fn the_tail_is_counted_in_characters_and_never_exceeds_the_commit() {
    let split = |tail_chars: u32, chars: u32| {
        let facts = DispatchFacts {
            tail_chars,
            ..picked(SelectionHow::PunctuationFinish, SelectionOutcome::Committed)
        };
        events(
            dispatch(InputClass::Punctuation),
            ordinary(),
            composing(1, 1, true),
            idle(),
            facts,
            Some(typed(chars)),
            PageSample::default(),
        )
        .into_iter()
        .find_map(|event| match event {
            LifecycleEvent::BeforeCommit {
                body_chars,
                tail_chars,
                ..
            } => Some((body_chars, tail_chars)),
            _ => None,
        })
        .unwrap()
    };
    assert_eq!(split(1, 3), (2, 1));
    assert_eq!(split(0, 3), (3, 0));
    assert_eq!(split(5, 3), (0, 3));
}

#[test]
fn the_end_reason_names_what_ended_the_composition() {
    let reason = |call: Call, commit: Option<CommitSample>| {
        events(
            call,
            ordinary(),
            composing(1, 1, true),
            idle(),
            DispatchFacts::default(),
            commit,
            PageSample::default(),
        )
        .into_iter()
        .filter_map(|event| match event {
            LifecycleEvent::CompositionEnd(reason) => Some(("end", reason)),
            LifecycleEvent::CandidatesHidden(reason) => Some(("hidden", reason)),
            _ => None,
        })
        .collect::<Vec<_>>()
    };
    for (call, commit, expected) in [
        (dispatch(InputClass::Cancel), None, EndReason::Cancel),
        (
            dispatch(InputClass::HostCommit),
            None,
            EndReason::HostCommit,
        ),
        (dispatch(InputClass::Edit), None, EndReason::Deleted),
        (dispatch(InputClass::Other), None, EndReason::Other),
        (
            dispatch(InputClass::Commit),
            Some(typed(1)),
            EndReason::Commit,
        ),
        (Call::Focus(false), None, EndReason::Focus),
        (Call::Focus(true), None, EndReason::Focus),
        (Call::SideChannel, None, EndReason::Other),
        (Call::Destroy, None, EndReason::Destroyed),
    ] {
        assert_eq!(
            reason(call, commit),
            [("end", expected), ("hidden", expected)],
            "{call:?}"
        );
    }
}

#[test]
fn a_view_change_without_a_composition_change_only_reshows_the_candidates() {
    // 高亮移动和翻页在 runtime 里提前返回，只动候选的 epoch。
    let moved = events(
        dispatch(InputClass::Navigation),
        ordinary(),
        composing(2, 2, true),
        composing(2, 3, true),
        DispatchFacts::default(),
        None,
        PageSample {
            count: 5,
            page: 1,
            highlighted: Some(0),
        },
    );
    assert_eq!(
        moved,
        [LifecycleEvent::CandidatesShown {
            count: 5,
            page: 1,
            highlighted: Some(0)
        }]
    );
    // 什么都没变：一个事件都没有。
    let still = events(
        Call::SideChannel,
        ordinary(),
        composing(2, 2, true),
        composing(2, 2, true),
        DispatchFacts::default(),
        None,
        page(5),
    );
    assert!(still.is_empty());
    // 候选清空而组字还在：hidden 的原因是这次调用的类别。
    let emptied = events(
        dispatch(InputClass::Character),
        ordinary(),
        composing(2, 2, true),
        composing(3, 3, false),
        DispatchFacts::default(),
        None,
        PageSample::default(),
    );
    assert_eq!(
        emptied,
        [
            LifecycleEvent::CompositionUpdate,
            LifecycleEvent::CandidatesHidden(EndReason::Other)
        ]
    );
}

#[test]
fn retraction_and_selection_lead_and_only_in_a_dispatch() {
    let facts = DispatchFacts {
        retracted: Some(SelectionRetraction::Restore),
        ..picked(SelectionHow::Select, SelectionOutcome::Held)
    };
    let dispatched = events(
        dispatch(InputClass::Edit),
        ordinary(),
        composing(1, 1, true),
        composing(2, 1, true),
        facts,
        None,
        page(1),
    );
    assert_eq!(
        dispatched[..2],
        [
            LifecycleEvent::SelectionRetracted(SelectionRetraction::Restore),
            LifecycleEvent::CandidateSelected(facts.selection.unwrap()),
        ]
    );
    let blurred = events(
        Call::Focus(false),
        ordinary(),
        composing(1, 1, true),
        idle(),
        facts,
        None,
        PageSample::default(),
    );
    assert!(!blurred.iter().any(|event| matches!(
        event,
        LifecycleEvent::SelectionRetracted(_) | LifecycleEvent::CandidateSelected(_)
    )));
}

#[test]
fn input_classes_and_local_modes_map_from_the_runtime() {
    use msime_engine::host::Command;
    use msime_input_runtime::Action;
    assert_eq!(
        InputClass::of(
            &Action::Character {
                value: b'a',
                shift: false
            },
            None
        ),
        InputClass::Character
    );
    assert_eq!(
        InputClass::of(&Action::PunctuationAscii(b','), None),
        InputClass::Punctuation
    );
    assert_eq!(
        InputClass::of(&Action::SelectHighlighted, None),
        InputClass::Selection
    );
    assert_eq!(
        InputClass::of(&Action::NextPage, None),
        InputClass::Navigation
    );
    assert_eq!(
        InputClass::of(&Action::Command(Command::Backspace), None),
        InputClass::Edit
    );
    assert_eq!(
        InputClass::of(&Action::Command(Command::CommitRaw), None),
        InputClass::Commit
    );
    assert_eq!(
        InputClass::of(&Action::Command(Command::Cancel), None),
        InputClass::Cancel
    );
    assert_eq!(
        InputClass::of(
            &Action::Command(Command::Cancel),
            Some(InputClass::HostCommit)
        ),
        InputClass::HostCommit
    );
    assert_eq!(
        InputClass::of(&Action::Finish, Some(InputClass::FocusLoss)),
        InputClass::FocusLoss
    );
    assert_eq!(
        InputClass::of(&Action::FinishOnFocusLoss, Some(InputClass::FocusLoss)),
        InputClass::FocusLoss
    );
    assert_eq!(
        InputClass::of(&Action::FinishOnFocusLoss, None),
        InputClass::Commit
    );
    assert_eq!(LocalModeClass::of("none"), LocalModeClass::None);
    assert_eq!(LocalModeClass::of("url"), LocalModeClass::Url);
    assert_eq!(LocalModeClass::of("date_time"), LocalModeClass::DateTime);
    assert_eq!(LocalModeClass::of("chaizi"), LocalModeClass::Chaizi);
    assert_eq!(LocalModeClass::Chaizi as u8, 13);
    assert_eq!(LocalModeClass::of("symbol"), LocalModeClass::Symbol);
    assert_eq!(LocalModeClass::Symbol as u8, 14);
    assert_eq!(LocalModeClass::of("unknown"), LocalModeClass::Unknown);
}

const FLASH: u32 = 1 << 17;

/// 取走本线程上内置消费者要过的声音。
fn cues() -> Vec<Cue> {
    CUES.with(|cues| std::mem::take(&mut *cues.borrow_mut()))
}

/// 一次选词上屏（选中第 0 个、两个字）交给 `state`，返回本次的打字特效。
fn commit_once(state: &mut LifecycleState, sound: &SessionSound) -> u32 {
    state.after(
        sound,
        dispatch(InputClass::Selection),
        composing(1, 1, true),
        idle(),
        picked(SelectionHow::Select, SelectionOutcome::Committed),
        Some(typed(2)),
        PageSample::default(),
    )
}

