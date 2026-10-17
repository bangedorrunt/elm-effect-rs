//! `packages/effect/test/ai/ResponseIdTracker.test.ts` prepare witnesses.
//!
//! The tracker pins incremental prompts by message identity: `mark_messages`
//! records which prompt produced a response id, and `prepare` returns the
//! previous id plus only the messages after the latest assistant turn when
//! the tracked prefix is fully recognized. Anything else is `None`.

use elm_effect_unstable::ai::{ResponseIdTracker, TrackMessage};

fn assert_some(
    prepared: Option<(String, Vec<TrackMessage>)>,
    id: &str,
    prompt: Vec<TrackMessage>,
    _row: &str,
) {
    let (got_id, got_prompt) = prepared.expect("prepare returns Some");
    assert_eq!(got_id, id);
    assert_eq!(got_prompt, prompt);
}

fn assert_none(prepared: Option<(String, Vec<TrackMessage>)>) {
    assert!(prepared.is_none());
}

#[test]
fn returns_none_for_a_fresh_tracker() {
    // ResponseIdTracker.test.ts:49
    let tracker = ResponseIdTracker::new();
    assert_none(tracker.prepare(&[TrackMessage::user("msg1")]));
}

#[test]
fn prepares_only_the_new_user_message_after_a_tracked_first_turn() {
    // ResponseIdTracker.test.ts:59
    let tracker = ResponseIdTracker::new();
    let msg1 = TrackMessage::user("msg1");
    let asst = TrackMessage::assistant("done");
    let msg2 = TrackMessage::user("msg2");
    tracker.mark_messages(&[msg1.clone()], "resp_123");
    assert_some(
        tracker.prepare(&[msg1, asst, msg2.clone()]),
        "resp_123",
        vec![msg2],
        ":59",
    );
}

#[test]
fn shares_tracker_state_through_one_instance() {
    // ResponseIdTracker.test.ts:72 — clear then re-mark serves the new id.
    let tracker = ResponseIdTracker::new();
    let msg1 = TrackMessage::user("msg1");
    let msg1_new = TrackMessage::user("msg1-new");
    let asst = TrackMessage::assistant("done");
    let msg2 = TrackMessage::user("msg2");
    tracker.mark_messages(&[msg1], "resp_123");
    tracker.clear();
    tracker.mark_messages(&[msg1_new.clone()], "resp_456");
    assert_some(
        tracker.prepare(&[msg1_new, asst, msg2.clone()]),
        "resp_456",
        vec![msg2],
        ":72",
    );
}

#[test]
fn keeps_latest_response_id_when_mark_parts_is_called_twice() {
    // ResponseIdTracker.test.ts:88
    let tracker = ResponseIdTracker::new();
    let msg1 = TrackMessage::user("msg1");
    let asst = TrackMessage::assistant("done");
    let msg2 = TrackMessage::user("msg2");
    tracker.mark_messages(&[msg1.clone()], "resp_1");
    tracker.mark_messages(&[msg1.clone()], "resp_2");
    assert_some(
        tracker.prepare(&[msg1, asst, msg2.clone()]),
        "resp_2",
        vec![msg2],
        ":88",
    );
}

#[test]
fn returns_none_after_clear() {
    // ResponseIdTracker.test.ts:102
    let tracker = ResponseIdTracker::new();
    let msg1 = TrackMessage::user("msg1");
    let asst = TrackMessage::assistant("done");
    let msg2 = TrackMessage::user("msg2");
    tracker.mark_messages(&[msg1.clone()], "resp_123");
    tracker.clear();
    assert_none(tracker.prepare(&[msg1, asst, msg2]));
}

#[test]
fn recovers_after_clear_with_new_marks() {
    // ResponseIdTracker.test.ts:116
    let tracker = ResponseIdTracker::new();
    let msg1 = TrackMessage::user("msg1");
    let msg1_new = TrackMessage::user("msg1-new");
    let asst = TrackMessage::assistant("done");
    let msg2 = TrackMessage::user("msg2");
    tracker.mark_messages(&[msg1], "resp_123");
    tracker.clear();
    tracker.mark_messages(&[msg1_new.clone()], "resp_456");
    assert_some(
        tracker.prepare(&[msg1_new, asst, msg2.clone()]),
        "resp_456",
        vec![msg2],
        ":116",
    );
}

#[test]
fn remains_usable_after_concurrent_mark_parts_and_clear() {
    // ResponseIdTracker.test.ts:132 — racy use never corrupts the tracker.
    let tracker = std::sync::Arc::new(ResponseIdTracker::new());
    let race = TrackMessage::user("race");
    std::thread::scope(|s| {
        let tracker = &tracker;
        let race = race.clone();
        let mark = s.spawn(move || {
            for _ in 0..200 {
                tracker.mark_messages(std::slice::from_ref(&race), "resp_race");
            }
        });
        let clear = s.spawn(|| {
            for _ in 0..200 {
                tracker.clear();
            }
        });
        mark.join();
        clear.join();
    });
    let msg1 = TrackMessage::user("msg1");
    let asst = TrackMessage::assistant("done");
    let msg2 = TrackMessage::user("msg2");
    tracker.mark_messages(&[msg1.clone()], "resp_final");
    assert_some(
        tracker.prepare(&[msg1, asst, msg2.clone()]),
        "resp_final",
        vec![msg2],
        ":132",
    );
}

#[test]
fn returns_none_for_first_turn_with_system_and_user_messages() {
    // ResponseIdTracker.test.ts:168
    let tracker = ResponseIdTracker::new();
    assert_none(tracker.prepare(&[
        TrackMessage::system("sys"),
        TrackMessage::user("user1"),
    ]));
}

#[test]
fn returns_none_when_tracked_prompt_has_no_assistant_message() {
    // ResponseIdTracker.test.ts:179
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    tracker.mark_messages(&[sys.clone(), user1.clone()], "resp_1");
    assert_none(tracker.prepare(&[sys, user1]));
}

#[test]
fn returns_incremental_user_follow_up_after_assistant() {
    // ResponseIdTracker.test.ts:191
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let asst1 = TrackMessage::assistant("assistant1");
    let user2 = TrackMessage::user("user2");
    tracker.mark_messages(&[sys.clone(), user1.clone()], "resp_1");
    assert_some(
        tracker.prepare(&[sys, user1, asst1, user2.clone()]),
        "resp_1",
        vec![user2],
        ":191",
    );
}

#[test]
fn returns_incremental_tool_results_after_assistant_tool_call() {
    // ResponseIdTracker.test.ts:205
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let asst1 = TrackMessage::assistant_tool_call_with_approval("call_1", "test_tool", "{}", "appr_1");
    let tool1 = TrackMessage::tool_result("call_1", "test_tool", "ok");
    tracker.mark_messages(&[sys.clone(), user1.clone()], "resp_1");
    assert_some(
        tracker.prepare(&[sys, user1, asst1, tool1.clone()]),
        "resp_1",
        vec![tool1],
        ":205",
    );
}

#[test]
fn returns_incremental_tool_results_plus_user_message_in_multi_step_flow() {
    // ResponseIdTracker.test.ts:219
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let asst1 = TrackMessage::assistant_tool_call_with_approval("call_1", "test_tool", "{}", "appr_1");
    let tool1 = TrackMessage::tool_result("call_1", "test_tool", "ok");
    let user2 = TrackMessage::user("user2");
    tracker.mark_messages(&[sys.clone(), user1.clone()], "resp_1");
    assert_some(
        tracker.prepare(&[sys, user1, asst1, tool1.clone(), user2.clone()]),
        "resp_1",
        vec![tool1, user2],
        ":219",
    );
}

#[test]
fn returns_none_when_no_new_messages_follow_assistant() {
    // ResponseIdTracker.test.ts:234
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let asst1 = TrackMessage::assistant("assistant1");
    tracker.mark_messages(&[sys.clone(), user1.clone()], "resp_1");
    assert_none(tracker.prepare(&[sys, user1, asst1]));
}

#[test]
fn returns_none_in_multi_turn_prompt_with_no_messages_after_last_assistant() {
    // ResponseIdTracker.test.ts:247
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let asst1 = TrackMessage::assistant("assistant1");
    let user2 = TrackMessage::user("user2");
    let asst2 = TrackMessage::assistant("assistant2");
    tracker.mark_messages(&[sys.clone(), user1.clone(), asst1.clone(), user2.clone()], "resp_2");
    assert_none(tracker.prepare(&[sys, user1, asst1, user2, asst2]));
}

#[test]
fn returns_none_for_empty_prompt() {
    // ResponseIdTracker.test.ts:262
    let tracker = ResponseIdTracker::new();
    assert_none(tracker.prepare(&[]));
}

#[test]
fn returns_none_when_system_prompt_object_changes() {
    // ResponseIdTracker.test.ts:271
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let sys_new = TrackMessage::system("sys-new");
    let user1 = TrackMessage::user("user1");
    let asst1 = TrackMessage::assistant("assistant1");
    let user2 = TrackMessage::user("user2");
    tracker.mark_messages(&[sys, user1.clone()], "resp_1");
    assert_none(tracker.prepare(&[sys_new, user1, asst1, user2]));
}

#[test]
fn returns_none_when_user_message_object_changes() {
    // ResponseIdTracker.test.ts:286
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let user1_edited = TrackMessage::user("user1-edited");
    let asst1 = TrackMessage::assistant("assistant1");
    let user2 = TrackMessage::user("user2");
    tracker.mark_messages(&[sys.clone(), user1], "resp_1");
    assert_none(tracker.prepare(&[sys, user1_edited, asst1, user2]));
}

#[test]
fn returns_none_when_multiple_prefix_messages_diverge() {
    // ResponseIdTracker.test.ts:301
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let sys_new = TrackMessage::system("sys-new");
    let user1_edited = TrackMessage::user("user1-edited");
    let asst1 = TrackMessage::assistant("assistant1");
    let user2 = TrackMessage::user("user2");
    tracker.mark_messages(&[sys, user1], "resp_1");
    assert_none(tracker.prepare(&[sys_new, user1_edited, asst1, user2]));
}

#[test]
fn clears_stale_state_after_divergence_and_recovers_when_the_new_prefix_is_re_marked() {
    // ResponseIdTracker.test.ts:317
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let sys_new = TrackMessage::system("sys-new");
    let asst1 = TrackMessage::assistant("assistant1");
    let user2 = TrackMessage::user("user2");
    tracker.mark_messages(&[sys, user1.clone()], "resp_1");
    assert_none(tracker.prepare(&[sys_new.clone(), user1.clone(), asst1.clone(), user2.clone()]));
    tracker.mark_messages(&[sys_new.clone(), user1.clone()], "resp_2");
    assert_some(
        tracker.prepare(&[sys_new, user1, asst1, user2.clone()]),
        "resp_2",
        vec![user2],
        ":317",
    );
}

#[test]
fn supports_full_lifecycle_across_two_turns() {
    // ResponseIdTracker.test.ts:336
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let asst1 = TrackMessage::assistant("assistant1");
    let user2 = TrackMessage::user("user2");
    let asst2 = TrackMessage::assistant("assistant2");
    let user3 = TrackMessage::user("user3");
    tracker.mark_messages(&[sys.clone(), user1.clone()], "resp_1");
    let first = tracker.prepare(&[sys.clone(), user1.clone(), asst1.clone(), user2.clone()]);
    assert_some(first, "resp_1", vec![user2.clone()], ":336a");
    tracker.mark_messages(&[sys.clone(), user1.clone(), asst1.clone(), user2.clone()], "resp_2");
    assert_some(
        tracker.prepare(&[sys, user1, asst1, user2, asst2, user3.clone()]),
        "resp_2",
        vec![user3],
        ":336b",
    );
}

#[test]
fn returns_none_for_structurally_equal_but_identity_different_prefix_message() {
    // ResponseIdTracker.test.ts:355 — identity is the uid, not the content.
    let tracker = ResponseIdTracker::new();
    let msg1 = TrackMessage::user("msg1");
    let msg1_copy = TrackMessage::user("msg1");
    let asst1 = TrackMessage::assistant("assistant1");
    let msg2 = TrackMessage::user("msg2");
    tracker.mark_messages(&[msg1], "resp_1");
    assert_none(tracker.prepare(&[msg1_copy, asst1, msg2]));
}

#[test]
fn returns_none_after_explicit_clear() {
    // ResponseIdTracker.test.ts:369
    let tracker = ResponseIdTracker::new();
    let sys = TrackMessage::system("sys");
    let user1 = TrackMessage::user("user1");
    let asst1 = TrackMessage::assistant("assistant1");
    let user2 = TrackMessage::user("user2");
    tracker.mark_messages(&[sys.clone(), user1.clone()], "resp_1");
    tracker.clear();
    assert_none(tracker.prepare(&[sys, user1, asst1, user2]));
}

#[test]
fn isolates_two_conversations_on_the_same_tracker() {
    // ResponseIdTracker.test.ts:384
    let tracker = ResponseIdTracker::new();
    let sys_a = TrackMessage::system("sysA");
    let user_a1 = TrackMessage::user("userA1");
    let asst_a1 = TrackMessage::assistant("asstA1");
    let user_a2 = TrackMessage::user("userA2");
    let sys_b = TrackMessage::system("sysB");
    let user_b1 = TrackMessage::user("userB1");
    let asst_b1 = TrackMessage::assistant("asstB1");
    let user_b2 = TrackMessage::user("userB2");
    tracker.mark_messages(&[sys_a.clone(), user_a1.clone()], "resp_A1");
    tracker.mark_messages(&[sys_b.clone(), user_b1.clone()], "resp_B1");
    assert_some(
        tracker.prepare(&[sys_a, user_a1, asst_a1, user_a2.clone()]),
        "resp_A1",
        vec![user_a2],
        ":384a",
    );
    assert_some(
        tracker.prepare(&[sys_b, user_b1, asst_b1, user_b2.clone()]),
        "resp_B1",
        vec![user_b2],
        ":384b",
    );
}

#[test]
fn does_not_leak_tracked_state_across_conversations_without_assistant_turns() {
    // ResponseIdTracker.test.ts:406
    let tracker = ResponseIdTracker::new();
    let sys_a = TrackMessage::system("sysA");
    let user_a1 = TrackMessage::user("userA1");
    let asst_a1 = TrackMessage::assistant("asstA1");
    let user_a2 = TrackMessage::user("userA2");
    let sys_b = TrackMessage::system("sysB");
    let user_b1 = TrackMessage::user("userB1");
    tracker.mark_messages(&[sys_a.clone(), user_a1.clone()], "resp_A1");
    tracker.mark_messages(&[sys_b.clone(), user_b1.clone()], "resp_B1");
    assert_some(
        tracker.prepare(&[sys_a, user_a1, asst_a1, user_a2.clone()]),
        "resp_A1",
        vec![user_a2],
        ":406a",
    );
    assert_none(tracker.prepare(&[sys_b, user_b1]));
}
