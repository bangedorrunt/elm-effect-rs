//! Leftover MCP carries: logging advertisement + log decode, prompt full
//! shapes with no-invoke validation, resource full shapes with ordered
//! multi-content reads.

use elm_effect_unstable::mcp_handlers::{
    Logger, PromptArg, PromptRegistry, ResourceContent, ResourceStore, ResourceTemplate,
};
use elm_effect_unstable::mcp_server::{CapabilityFlags, ServerCapabilities};

fn features() -> ServerCapabilities {
    ServerCapabilities {
        flags: CapabilityFlags {
            tools: true,
            prompts: true,
            resources: true,
            logging: true,
            sampling: true,
        },
    }
}

#[test]
fn logging_advertised_when_notifications_supported() {
    // LoggingTest:48 — a server supporting log notifications advertises it.
    assert!(features().advertise().contains_key("logging"));
    assert!(
        !ServerCapabilities::minimal()
            .advertise()
            .contains_key("logging")
    );
}

#[test]
fn log_notification_preserves_level_logger_and_data() {
    // LoggingTest:506 — decoded log notifications keep level, logger, data.
    let note = Logger::new()
        .log("warning", Some("app"), serde_json::json!({"n": 1}))
        .unwrap();
    assert_eq!(note.method, "notifications/message");
    assert!(note.id.is_none());
    assert_eq!(note.params["level"], serde_json::json!("warning"));
    assert_eq!(note.params["logger"], serde_json::json!("app"));
    assert_eq!(note.params["data"], serde_json::json!({"n": 1}));
}

fn prompt_registry() -> PromptRegistry {
    let mut reg = PromptRegistry::new();
    reg.register(
        "greet",
        Some("Greeter"),
        "Greets a person warmly",
        vec![
            PromptArg {
                name: "name".to_owned(),
                required: true,
                choices: vec![],
            },
            PromptArg {
                name: "tone".to_owned(),
                required: false,
                choices: vec!["warm".to_owned(), "icy".to_owned()],
            },
        ],
        vec![
            ("user".to_owned(), "hello {{name}}".to_owned()),
            ("assistant".to_owned(), "hi {{name}} ({{tone}})".to_owned()),
        ],
    );
    reg
}

#[test]
fn prompt_list_preserves_names_titles_descriptions_and_args() {
    // PromptsTest:90 — list keeps name, title, description, and arg shapes.
    let reg = prompt_registry();
    let listed = reg.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "greet");
    assert_eq!(listed[0].title.as_deref(), Some("Greeter"));
    assert_eq!(listed[0].description, "Greets a person warmly");
    assert_eq!(listed[0].arguments.len(), 2);
    assert_eq!(listed[0].arguments[0].name, "name");
    assert!(listed[0].arguments[0].required);
    assert!(!listed[0].arguments[1].required);
}

#[test]
fn prompt_get_preserves_description_and_message_order() {
    // PromptsTest:274 — get echoes the description and keeps message order.
    let out = prompt_registry()
        .get("greet", &serde_json::json!({"name": "bo", "tone": "warm"}))
        .unwrap();
    assert_eq!(out.description, "Greets a person warmly");
    assert_eq!(out.messages.len(), 2);
    assert_eq!(out.messages[0]["role"], serde_json::json!("user"));
    assert_eq!(out.messages[1]["role"], serde_json::json!("assistant"));
    assert_eq!(
        out.messages[0]["content"]["text"],
        serde_json::json!("hello bo")
    );
}

#[test]
fn prompt_validation_fails_without_invoking_substitution() {
    // PromptsTest:254 — missing required args fail; a later valid call is unaffected.
    let reg = prompt_registry();
    let err = reg.get("greet", &serde_json::json!({})).unwrap_err();
    assert!(err.contains("name"), "unexpected error: {err}");
    let out = reg
        .get("greet", &serde_json::json!({"name": "bo"}))
        .unwrap();
    assert_eq!(out.messages.len(), 2);
}

fn resource_store() -> ResourceStore {
    let mut store = ResourceStore::new();
    store.insert_text(
        "file:///a.txt",
        "notes",
        "plain notes",
        "text/plain",
        "hello",
    );
    store
}

#[test]
fn resource_list_preserves_uri_name_description_and_mime() {
    // ResourcesTest:69 — list keeps URI, name, description, and MIME type.
    let store = resource_store();
    let listed = store.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].uri, "file:///a.txt");
    assert_eq!(listed[0].name, "notes");
    assert_eq!(listed[0].description, "plain notes");
    assert_eq!(listed[0].mime_type.as_deref(), Some("text/plain"));
}

#[test]
fn resource_contents_preserve_uri_and_mime() {
    // ResourcesTest:145 — read contents keep the URI and MIME type.
    let out = resource_store().read("file:///a.txt").unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].uri, "file:///a.txt");
    assert_eq!(out[0].mime_type.as_deref(), Some("text/plain"));
    assert_eq!(out[0].text.as_deref(), Some("hello"));
}

#[test]
fn resource_read_returns_multiple_contents_in_order() {
    // ResourcesTest:163 — a template handler may return ordered contents.
    let mut store = ResourceStore::new();
    store.add_template(ResourceTemplate {
        uri_template: "file:///docs/{doc}".to_owned(),
        name: "doc".to_owned(),
        description: "a doc".to_owned(),
        mime_type: "text/plain".to_owned(),
        handler: Box::new(|params| {
            Ok(vec![
                ResourceContent {
                    uri: format!("file:///docs/{}", params["doc"]),
                    mime_type: Some("text/plain".to_owned()),
                    text: Some("first".to_owned()),
                    blob: None,
                },
                ResourceContent {
                    uri: format!("file:///docs/{}", params["doc"]),
                    mime_type: Some("text/plain".to_owned()),
                    text: Some("second".to_owned()),
                    blob: None,
                },
            ])
        }),
    });
    let out = store.read("file:///docs/guide").unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].text.as_deref(), Some("first"));
    assert_eq!(out[1].text.as_deref(), Some("second"));
}
