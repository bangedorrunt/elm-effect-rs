//! `packages/effect/test/ai/LanguageModelTrackerLifecycle.test.ts` tracker witnesses.

use elm_effect_unstable::ai::{
    TrackMessage, TrackerModel,
};
use elm_effect_unstable::ai::ResponseIdTracker;

fn system(text: &str) -> TrackMessage {
    TrackMessage::system(text)
}

fn user(text: &str) -> TrackMessage {
    TrackMessage::user(text)
}

fn assistant(text: &str) -> TrackMessage {
    TrackMessage::assistant(text)
}

#[test]
fn tracks_generate_text_across_turns_and_recovers_after_session_drop() {
    // LanguageModelTrackerLifecycle.test.ts:48 — incremental prompts with previous ids; full prompt after clear.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_1", "resp_2", "resp_3", "resp_4"]);

    let system_msg = system("system");
    let user1 = user("user1");
    let assistant1 = assistant("assistant1");
    let user2 = user("user2");
    let assistant2 = assistant("assistant2");
    let user3 = user("user3");
    let assistant3 = assistant("assistant3");
    let user4 = user("user4");

    model.generate_text(Some(&tracker), vec![system_msg.clone(), user1.clone()]);
    let first = model.calls()[0].prompt.clone();

    let mut second_prompt = first.clone();
    second_prompt.push(assistant1.clone());
    second_prompt.push(user2.clone());
    model.generate_text(Some(&tracker), second_prompt);
    let second = model.calls()[1].prompt.clone();

    tracker.clear();

    let mut third_prompt = second.clone();
    third_prompt.push(assistant2.clone());
    third_prompt.push(user3.clone());
    model.generate_text(Some(&tracker), third_prompt);
    let third = model.calls()[2].prompt.clone();
    let mut expected_full = second.clone();
    expected_full.push(assistant2.clone());
    expected_full.push(user3.clone());

    let mut fourth_prompt = third.clone();
    fourth_prompt.push(assistant3.clone());
    fourth_prompt.push(user4.clone());
    model.generate_text(Some(&tracker), fourth_prompt);

    let calls = model.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0].previous_response_id, None);
    assert_eq!(calls[0].incremental_prompt, None);
    assert_eq!(calls[1].previous_response_id.as_deref(), Some("resp_1"));
    assert_eq!(calls[1].incremental_prompt, Some(vec![user2.clone()]));
    assert_eq!(calls[2].previous_response_id, None);
    assert_eq!(calls[2].incremental_prompt, None);
    assert_eq!(calls[2].prompt, expected_full);
    assert_eq!(calls[3].previous_response_id.as_deref(), Some("resp_3"));
    assert_eq!(calls[3].incremental_prompt, Some(vec![user4.clone()]));
}

#[test]
fn falls_back_to_full_prompt_after_stale_previous_response_id_and_keeps_tracking() {
    // LanguageModelTrackerLifecycle.test.ts:118 — object calls keep previous id plus new-message suffix.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_object_1", "resp_object_2"]);

    let system_msg = system("system");
    let user1 = user("user1");
    let assistant1 = assistant("assistant1");
    let user2 = user("user2");

    model.generate_object(Some(&tracker), vec![system_msg.clone(), user1.clone()], "count");
    let first = model.calls()[0].prompt.clone();

    let mut second_prompt = first.clone();
    second_prompt.push(assistant1.clone());
    second_prompt.push(user2.clone());
    model.generate_object(Some(&tracker), second_prompt, "count");

    let calls = model.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].previous_response_id, None);
    assert_eq!(calls[0].incremental_prompt, None);
    assert_eq!(calls[1].previous_response_id.as_deref(), Some("resp_object_1"));
    assert_eq!(calls[1].incremental_prompt, Some(vec![user2.clone()]));
}

#[test]
fn tracks_stream_text_with_metadata_parts_across_turns() {
    // LanguageModelTrackerLifecycle.test.ts:170 — streamed turns carry previous id plus new-message suffix.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_stream_1", "resp_stream_2"]);

    let system_msg = system("system");
    let user1 = user("user1");
    let assistant1 = assistant("assistant1");
    let user2 = user("user2");

    model.stream_text(Some(&tracker), vec![system_msg.clone(), user1.clone()]);
    let first = model.calls()[0].prompt.clone();

    let mut second_prompt = first.clone();
    second_prompt.push(assistant1.clone());
    second_prompt.push(user2.clone());
    model.stream_text(Some(&tracker), second_prompt);

    let calls = model.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].previous_response_id, None);
    assert_eq!(calls[0].incremental_prompt, None);
    assert_eq!(calls[1].previous_response_id.as_deref(), Some("resp_stream_1"));
    assert_eq!(calls[1].incremental_prompt, Some(vec![user2.clone()]));
}

#[test]
fn emits_finish_parts_in_stream_text_when_toolkit_is_provided_and_no_tool_calls_are_returned() {
    // LanguageModelTrackerLifecycle.test.ts:223 — a toolkit stream with no tool calls still ends in finish.
    let model = TrackerModel::new(vec!["resp_toolkit_1"]);
    let parts = model.stream_text_with_toolkit(None, vec![user("hello")], true);
    assert_eq!(parts.len(), 1);
    assert!(parts[0].is_finish(), "expected a single finish part");
}

#[test]
fn keeps_incremental_provider_fields_undefined_when_tracker_is_omitted() {
    // LanguageModelTrackerLifecycle.test.ts:244 — without a tracker every call sends the full prompt.
    let model = TrackerModel::new(vec![
        "resp_none",
        "resp_none",
        "resp_none",
        "resp_none",
        "resp_none_stream",
        "resp_none_stream",
    ]);

    let system_msg = system("system");
    let user1 = user("user1");
    let assistant1 = assistant("assistant1");
    let user2 = user("user2");
    let full = vec![
        system_msg.clone(),
        user1.clone(),
        assistant1.clone(),
        user2.clone(),
    ];
    let short = vec![system_msg.clone(), user1.clone()];

    model.generate_text(None, short.clone());
    model.generate_text(None, full.clone());
    model.generate_object(None, short.clone(), "count");
    model.generate_object(None, full.clone(), "count");
    model.stream_text(None, short.clone());
    model.stream_text(None, full.clone());

    let calls = model.calls();
    assert_eq!(calls.len(), 6);
    for call in &calls {
        assert_eq!(call.previous_response_id, None);
        assert_eq!(call.incremental_prompt, None);
    }
}

#[test]
fn prepares_after_approval_pre_resolution_using_stripped_prompt() {
    // LanguageModelTrackerLifecycle.test.ts:314 — approved tool runs once; provider sees stripped prompt plus result.
    let tracker = ResponseIdTracker::new();
    let model = TrackerModel::new(vec!["resp_tool_1", "resp_tool_2"]);

    let system_msg = system("system");
    let user1 = user("user1");

    model.generate_text(Some(&tracker), vec![system_msg.clone(), user1.clone()]);
    let first = model.calls()[0].prompt.clone();

    let tool_call_id = "call-approval";
    let approval_id = "approval-1";
    let mut pending = first.clone();
    pending.push(TrackMessage::assistant_tool_call_with_approval(
        tool_call_id,
        "ApprovalTool",
        "delete",
        approval_id,
    ));
    pending.push(TrackMessage::tool_approval_response(approval_id, true));

    model.generate_text_with_approval(Some(&tracker), pending);

    let calls = model.calls();
    assert_eq!(calls.len(), 2);
    let second = &calls[1];
    assert_eq!(second.previous_response_id.as_deref(), Some("resp_tool_1"));

    let expected_incremental = vec![TrackMessage::tool_result(
        tool_call_id,
        "ApprovalTool",
        "approved-result",
    )];
    assert_eq!(second.incremental_prompt, Some(expected_incremental));

    for message in &second.prompt {
        let text = format!("{message:?}");
        assert!(
            !text.contains("ApprovalRequest"),
            "stripped prompt keeps approval artifacts: {text}"
        );
        assert!(
            !text.contains("ApprovalResponse"),
            "stripped prompt keeps approval artifacts: {text}"
        );
    }
}
