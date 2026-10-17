//! `packages/effect/test/ai/Chat.test.ts` history witnesses.

use elm_effect_unstable::ai::{ApprovalTracker, ChatMessage, ChatRole, ChatStore};

fn message(id: &str, role: ChatRole, content: &str) -> ChatMessage {
    ChatMessage { id: id.to_owned(), role, content: content.to_owned() }
}

#[test]
fn does_not_replay_a_completed_approved_tool() {
    // Chat.test.ts:19 — the approved tool executes once; later turns skip it.
    let approvals = ApprovalTracker::new();
    let mut executions = 0;
    assert!(approvals.run_once("counter-call", || executions += 1));
    assert!(!approvals.run_once("counter-call", || executions += 1));
    assert!(!approvals.run_once("counter-call", || executions += 1));
    assert_eq!(executions, 1);
}

#[test]
fn persists_chat_history_to_the_backing_store() {
    // Chat.test.ts:56 — saved history loads back identically, ids intact.
    let store = ChatStore::new();
    store.save(
        "1",
        vec![
            message("msg_abc123", ChatRole::User, "test user message"),
            message("msg_abc123", ChatRole::Assistant, "test assistant message"),
        ],
    );
    let history = store.load("1").expect("saved");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id, "msg_abc123");
    assert_eq!(history[0].content, "test user message");
    assert_eq!(history[1].content, "test assistant message");
}

#[test]
fn respects_the_specified_time_to_live() {
    // Chat.test.ts:92 — history expires after its TTL passes.
    let store = ChatStore::with_ttl(std::time::Duration::from_millis(50));
    store.save("1", vec![message("", ChatRole::User, "test user message")]);
    assert!(store.load("1").is_ok());
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(store.load("1").is_err());
}

#[test]
fn prefers_the_message_identifier_of_the_most_recent_assistant_message() {
    // Chat.test.ts:135 — follow-ups keep the latest assistant message's id.
    let store = ChatStore::new();
    store.save(
        "2",
        vec![
            message("msg_123abc", ChatRole::User, "first user message"),
            message("msg_123abc", ChatRole::Assistant, "first assistant message"),
        ],
    );
    assert_eq!(store.latest_assistant_id("2").as_deref(), Some("msg_123abc"));
    assert_eq!(store.latest_assistant_id("missing"), None);
}

#[test]
fn raises_an_error_when_retrieving_a_chat_that_does_not_exist() {
    // Chat.test.ts:175 — missing chats fail with their id attached.
    let store = ChatStore::new();
    let err = store.load("chat-321").unwrap_err();
    assert_eq!(err.chat_id, "chat-321");
}
