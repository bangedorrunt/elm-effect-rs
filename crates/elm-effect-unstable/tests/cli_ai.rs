//! CLI + AI fence: clap-backed argv parsing and the LanguageModel fake
//! contract. Parsing runs on argument vectors, never on the process.

use elm_effect_unstable::ai::{
    FakeModel, GeneratedText, LanguageModel, ModelError, object_has_keys,
};
use elm_effect_unstable::cli::{CliError, demo_app, parse};

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_serve_defaults() {
    let app = demo_app();
    let req = parse(&app, &argv(&["demo", "serve"])).expect("parses");
    assert_eq!(req.command, ["serve".to_owned()]);
    assert_eq!(req.flags.get("port").map(String::as_str), Some("8080"));
    assert!(!req.flags.contains_key("verbose"));
    assert!(req.positional.is_empty());
}

#[test]
fn parse_flags_and_positionals() {
    let app = demo_app();
    let req = parse(
        &app,
        &argv(&["demo", "serve", "--port", "1", "-v", "a.txt"]),
    )
    .expect("parses");
    assert_eq!(req.flags.get("port").map(String::as_str), Some("1"));
    assert_eq!(req.flags.get("verbose").map(String::as_str), Some("true"));
    assert_eq!(req.positional, ["a.txt".to_owned()]);
}

#[test]
fn parse_errors_and_help() {
    let app = demo_app();
    assert!(matches!(
        parse(&app, &argv(&["demo", "nope"])),
        Err(CliError::Unknown(_))
    ));
    assert!(matches!(
        parse(&app, &argv(&["demo", "--help"])),
        Err(CliError::Help(_))
    ));
    assert!(matches!(
        parse(&app, &argv(&["demo", "serve", "--port"])),
        Err(CliError::MissingValue(_)) | Err(CliError::Unknown(_))
    ));
}

#[test]
fn ai_fake_contract() {
    let model = FakeModel::new();
    model.push_text(Ok(GeneratedText {
        text: "hi".to_owned(),
        finish_reason: "stop".to_owned(),
    }));
    model.push_object(Ok("{\"a\":1}".to_owned()));
    model.push_chunks(vec!["x".to_owned(), "y".to_owned()]);
    assert_eq!(model.generate_text("p").expect("text").text, "hi");
    let obj = model.generate_object("p", "{}").expect("object");
    assert!(object_has_keys(&obj, &["a"]));
    assert!(!object_has_keys(&obj, &["zzz"]));
    assert!(!object_has_keys("nope", &["a"]));
    assert_eq!(model.stream_text("p"), ["x".to_owned(), "y".to_owned()]);
    assert_eq!(model.calls().len(), 3);
    model.push_text(Err(ModelError::Provider("down".to_owned())));
    assert!(matches!(
        model.generate_text("p"),
        Err(ModelError::Provider(_))
    ));
}
