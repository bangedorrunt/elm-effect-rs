//! ResourcesTest carry rows: list/read plus templates with URI expansion.
use elm_effect_unstable::mcp_handlers::{ResourceContent, ResourceStore, ResourceTemplate};

fn store() -> ResourceStore {
    let mut store = ResourceStore::new();
    store.insert_text("file:///a.txt", "a", "first", "text/plain", "hello");
    store.insert_binary("file:///b.bin", "b", "second", "application/octet-stream", "aGk=");
    store.add_template(ResourceTemplate {
        uri_template: "file:///docs/{doc}".to_owned(),
        name: "doc".to_owned(),
        description: "a doc".to_owned(),
        mime_type: "text/plain".to_owned(),
        handler: Box::new(|params| {
            Ok(vec![ResourceContent {
                uri: format!("file:///docs/{}", params.get("doc").map(String::as_str).unwrap_or("?")),
                mime_type: Some("text/plain".to_owned()),
                text: Some("doc!".to_owned()),
                blob: None,
            }])
        }),
    });
    store
}

#[test]
fn advertises_when_registered_and_not_when_empty() {
    assert!(store().capability());
    assert!(!ResourceStore::new().capability());
}

#[test]
fn lists_every_resource_with_schema_fields() {
    let s = store();
    let listed = s.list();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].uri, "file:///a.txt");
    assert_eq!(listed[0].mime_type.as_deref(), Some("text/plain"));
}

#[test]
fn reads_text_and_binary_preserving_uri_and_mime() {
    let out = store().read("file:///a.txt").unwrap();
    assert_eq!(out[0].text.as_deref(), Some("hello"));
    assert_eq!(out[0].mime_type.as_deref(), Some("text/plain"));
    let out = store().read("file:///b.bin").unwrap();
    assert_eq!(out[0].blob.as_deref(), Some("aGk="));
    assert_eq!(out[0].mime_type.as_deref(), Some("application/octet-stream"));
}

#[test]
fn rejects_unknown_uris() {
    assert!(store().read("file:///missing").is_err());
}

#[test]
fn lists_templates_and_expands_concrete_uris() {
    let s = store();
    let templates = s.list_templates();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].uri_template, "file:///docs/{doc}");
    let out = store().read("file:///docs/guide").unwrap();
    assert_eq!(out[0].uri, "file:///docs/guide");
    assert_eq!(out[0].text.as_deref(), Some("doc!"));
}

#[test]
fn does_not_invoke_template_handler_on_decode_failure() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let count = Arc::new(AtomicUsize::new(0));
    let moved = count.clone();
    let mut store = ResourceStore::new();
    store.add_template(ResourceTemplate {
        uri_template: "file:///x/{id}".to_owned(),
        name: "x".to_owned(),
        description: "".to_owned(),
        mime_type: "".to_owned(),
        handler: Box::new(move |_| {
            moved.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        }),
    });
    assert!(store.read("file:///other/path/extra").is_err());
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn subscription_lifecycle_routes_only_subscribed_uris() {
    let mut store = store();
    store.subscribe("file:///a.txt");
    assert!(store.notify_update("file:///a.txt").is_some());
    assert!(store.notify_update("file:///b.bin").is_none());
    store.unsubscribe("file:///a.txt");
    assert!(store.notify_update("file:///a.txt").is_none());
}
