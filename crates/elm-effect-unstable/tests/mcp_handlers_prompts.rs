//! PromptsTest carry rows: list/get with argument substitution.
use elm_effect_unstable::mcp_handlers::{PromptArg, PromptRegistry};

fn registry() -> PromptRegistry {
    let mut prompts = PromptRegistry::new();
    prompts.register(
        "greet",
        Some("Greeter"),
        "says hi",
        vec![
            PromptArg { name: "name".to_owned(), required: true, choices: vec![] },
            PromptArg { name: "tone".to_owned(), required: false, choices: vec!["warm".to_owned(), "terse".to_owned()] },
        ],
        vec![("user".to_owned(), "hello {{name}} ({{tone}})".to_owned())],
    );
    prompts.register("plain", None, "no args", vec![], vec![("user".to_owned(), "hi".to_owned())]);
    prompts
}

#[test]
fn advertises_when_registered_and_not_when_empty() {
    assert!(registry().capability());
    assert!(!PromptRegistry::new().capability());
}

#[test]
fn lists_every_prompt_with_required_markers() {
    let reg = registry();
    let listed = reg.list();
    assert_eq!(listed.len(), 2);
    let greet = &listed[0];
    assert_eq!(greet.name, "greet");
    assert_eq!(greet.title.as_deref(), Some("Greeter"));
    assert_eq!(greet.arguments[0].name, "name");
    assert!(greet.arguments[0].required);
    assert!(!greet.arguments[1].required);
}

#[test]
fn gets_prompt_without_arguments() {
    let out = registry().get("plain", &serde_json::json!({})).unwrap();
    assert_eq!(out.messages[0]["content"]["text"], serde_json::json!("hi"));
    assert_eq!(out.messages.len(), 1);
}

#[test]
fn substitutes_valid_arguments() {
    let out = registry().get("greet", &serde_json::json!({ "name": "bo", "tone": "warm" })).unwrap();
    assert_eq!(out.messages[0]["content"]["text"], serde_json::json!("hello bo (warm)"));
    assert_eq!(out.messages[0]["role"], serde_json::json!("user"));
}

#[test]
fn rejects_unknown_prompt_missing_required_and_bad_choices() {
    assert!(registry().get("nope", &serde_json::json!({})).is_err());
    assert!(registry().get("greet", &serde_json::json!({})).is_err());
    assert!(registry().get("greet", &serde_json::json!({ "name": "bo", "tone": "icy" })).is_err());
}

#[test]
fn omits_optional_arguments_cleanly() {
    let out = registry().get("greet", &serde_json::json!({ "name": "bo" })).unwrap();
    assert_eq!(out.messages[0]["content"]["text"], serde_json::json!("hello bo ()"));
}
