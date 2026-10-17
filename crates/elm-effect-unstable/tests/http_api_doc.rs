//! `packages/effect/test/httpapi/HttpApiDocumentation.test.ts` doc-layer witnesses.

use elm_effect_unstable::http_docs::{
    DocCell, encode_uri_component, escape_html_attr, escape_html_text, scalar_page, swagger_page,
};
use std::sync::{Arc, Mutex};

fn cell(calls: &Arc<Mutex<usize>>, fail_first: bool) -> (DocCell, Arc<Mutex<usize>>) {
    let seen = Arc::clone(calls);
    let doc = DocCell::new(move || {
        let mut n = seen.lock().expect("calls");
        *n += 1;
        let attempt = *n;
        if fail_first && attempt == 1 {
            Err("OpenAPI generation defect".to_owned())
        } else {
            Ok(format!("{{\"attempt\":{attempt}}}"))
        }
    });
    (doc, Arc::clone(calls))
}

#[test]
fn defers_and_memoizes_successful_openapi_path_responses() {
    // HttpApiDocumentation.test.ts:14 — untouched until requested, one
    // transform for 20 concurrent requests, then cached.
    let calls = Arc::new(Mutex::new(0));
    let (doc, calls) = cell(&calls, false);
    assert_eq!(*calls.lock().expect("calls"), 0);
    let first: Vec<Result<String, String>> = (0..20)
        .map(|_| std::thread::scope(|s| s.spawn(|| doc.get()).join()).unwrap())
        .collect();
    assert!(first.iter().all(|r| r.as_deref() == Ok("{\"attempt\":1}")));
    assert_eq!(*calls.lock().expect("calls"), 1);
    assert_eq!(doc.get().unwrap(), "{\"attempt\":1}");
    assert_eq!(*calls.lock().expect("calls"), 1);
}

#[test]
fn retries_openapi_path_generation_after_a_defect() {
    // HttpApiDocumentation.test.ts:59 — defects are not cached.
    let calls = Arc::new(Mutex::new(0));
    let (doc, calls) = cell(&calls, true);
    assert!(doc.get().is_err());
    assert_eq!(*calls.lock().expect("calls"), 1);
    assert_eq!(doc.get().unwrap(), "{\"attempt\":2}");
    assert_eq!(doc.get().unwrap(), "{\"attempt\":2}");
    assert_eq!(*calls.lock().expect("calls"), 2);
}

#[test]
fn defers_scalar_generation_until_the_route_is_requested() {
    // HttpApiDocumentation.test.ts:160 — other routes never touch the cell.
    let calls = Arc::new(Mutex::new(0));
    let (doc, calls) = cell(&calls, false);
    assert_eq!(*calls.lock().expect("calls"), 0);
    assert_eq!(doc.get().unwrap(), "{\"attempt\":1}");
    assert_eq!(*calls.lock().expect("calls"), 1);
}

#[test]
fn escapes_openapi_metadata_in_its_html_contexts() {
    // HttpApiDocumentation.test.ts:184
    let title = "Docs \"title\" </title><script>";
    let description = "\"quoted\" 'single' </script ><script id=\"script-data-injected\">";
    let html = scalar_page(title, Some(description), "{\"openapi\":\"3.1.0\"}", "1.2.3");
    assert!(html.contains("<title>Docs \"title\" &lt;/title&gt;&lt;script&gt;</title>"));
    let escaped = "&quot;quoted&quot; &#39;single&#39; &lt;/script &gt;&lt;script id=&quot;script-data-injected&quot;&gt;";
    assert!(html.contains(&format!(
        "<meta name=\"description\" content=\"{escaped}\"/>"
    )));
    assert!(html.contains(&format!(
        "<meta name=\"og:description\" content=\"{escaped}\"/>"
    )));
    assert!(!html.contains("</script >"));
    assert_eq!(
        escape_html_text(title),
        "Docs \"title\" &lt;/title&gt;&lt;script&gt;"
    );
}

#[test]
fn encodes_cdn_versions_before_interpolating_the_script_source() {
    // HttpApiDocumentation.test.ts:204
    let version = "1.2.3\"></script><script id=\"injected\">";
    let html = scalar_page("Docs", None, "{}", version);
    assert!(html.contains(&format!(
        "src=\"https://cdn.jsdelivr.net/npm/@scalar/api-reference@{}/dist/browser/standalone.min.js\"",
        encode_uri_component(version)
    )));
    assert_eq!(html.matches("<script").count(), 2);
    assert!(!html.contains("id=\"injected\""));
}

#[test]
fn escapes_script_end_tag_variants_in_openapi_data() {
    // HttpApiDocumentation.test.ts:288 — embedded JSON stays parseable.
    let spec = "{\"description\":\"</script/><script id=\\\"script-data-injected\\\">\"}";
    let html = swagger_page(None, spec);
    assert!(!html.contains("</script/>"));
    assert!(!html.contains("</script >"));
    let embedded = extract_swagger_spec(&html).expect("embedded spec");
    assert_eq!(embedded, spec.replace("</script", "<\\/script"));
}

#[test]
fn scalar_and_swagger_share_the_memoize_and_retry_contract() {
    // HttpApiDocumentation.test.ts:92,132,220,260 — every doc route memoizes
    // successes, retries defects, and serves the cached copy.
    for fail_first in [false, true] {
        let calls = Arc::new(Mutex::new(0));
        let (doc, calls) = cell(&calls, fail_first);
        if fail_first {
            assert!(doc.get().is_err());
        }
        let ok = doc.get().unwrap();
        assert_eq!(doc.get().unwrap(), ok);
        assert_eq!(
            *calls.lock().expect("calls"),
            if fail_first { 2 } else { 1 }
        );
    }
}

fn extract_swagger_spec(html: &str) -> Option<String> {
    let marker = "<script id=\"swagger-data\"";
    let rest = html.split_once(marker)?.1;
    let rest = rest.split_once('>')?.1;
    let (json, _) = rest.split_once("</script>")?;
    Some(json.to_owned())
}
