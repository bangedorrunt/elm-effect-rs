//! `packages/effect/test/ai/EmbeddingModel.test.ts` behavior witnesses.

use elm_effect_unstable::ai::{AiError, AiReason, EmbeddingModel, EmbeddingUsage};
use std::sync::{Arc, Mutex};

fn recording(provider: impl Fn(&[String]) -> Result<(Vec<Vec<f64>>, EmbeddingUsage), AiError> + Send + Sync + 'static) -> (EmbeddingModel, Arc<Mutex<Vec<Vec<String>>>>) {
    let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_in = Arc::clone(&seen);
    let model = EmbeddingModel::new(move |inputs| {
        seen_in.lock().expect("seen").push(inputs.to_vec());
        provider(inputs)
    });
    (model, seen)
}

#[test]
fn round_trips_usage_with_undefined_input_tokens_through_json() {
    // EmbeddingModel.test.ts:19 — undefined inputTokens encodes to {} and decodes back.
    let usage = EmbeddingUsage { input_tokens: None };
    let json = usage.to_json();
    assert_eq!(json, serde_json::json!({}), "encoded JSON");
    assert_eq!(EmbeddingUsage::from_json(&json), Some(usage));
}

#[test]
fn embed_returns_a_vector() {
    // EmbeddingModel.test.ts:31 — embed delegates to embedMany and unwraps the single result.
    let (model, seen) = recording(|_| {
        Ok((
            vec![vec![1.0, 2.0, 3.0]],
            EmbeddingUsage { input_tokens: Some(5) },
        ))
    });
    let response = model.embed("hello").unwrap();
    assert_eq!(response.vector, vec![1.0, 2.0, 3.0]);
    assert_eq!(*seen.lock().expect("seen"), vec![vec!["hello".to_owned()]]);
}

#[test]
fn embed_many_returns_ordered_vectors_with_usage() {
    // EmbeddingModel.test.ts:55 — vectors stay ordered and usage passes through.
    let (model, seen) = recording(|_| {
        Ok((
            vec![vec![1.0, 2.0], vec![3.0, 4.0]],
            EmbeddingUsage { input_tokens: Some(9) },
        ))
    });
    let response = model.embed_many(&["hello", "world"]).unwrap();
    let vectors: Vec<Vec<f64>> = response.embeddings.iter().map(|e| e.vector.clone()).collect();
    assert_eq!(vectors, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
    assert_eq!(response.usage.input_tokens, Some(9));
    assert_eq!(
        *seen.lock().expect("seen"),
        vec![vec!["hello".to_owned(), "world".to_owned()]]
    );
}

#[test]
fn provider_ai_error_propagates_through_embed() {
    // EmbeddingModel.test.ts:113 — a provider failure surfaces unchanged.
    let expected = AiError::new(AiReason::Unknown).with_context("EmbeddingModelTest", "embedMany");
    let failing = expected.clone();
    let (model, _) =
        recording(move |_| Err::<(Vec<Vec<f64>>, EmbeddingUsage), AiError>(failing.clone()));
    let error = model.embed("hello").unwrap_err();
    assert_eq!(error, expected);
}

#[test]
fn embed_fails_with_invalid_output_when_provider_misses_results() {
    // EmbeddingModel.test.ts:134 — zero results for one input is an InvalidOutputError.
    let (model, _) = recording(|_| {
        Ok((
            Vec::new(),
            EmbeddingUsage { input_tokens: Some(0) },
        ))
    });
    let error = model.embed("hello").unwrap_err();
    assert_eq!(error.reason_tag(), "InvalidOutputError");
}

#[test]
fn embed_many_fails_with_invalid_output_when_provider_misses_results() {
    // EmbeddingModel.test.ts:156 — fewer results than inputs is an InvalidOutputError.
    let (model, _) = recording(|_| {
        Ok((
            vec![vec![1.0, 2.0, 3.0]],
            EmbeddingUsage { input_tokens: Some(3) },
        ))
    });
    let error = model.embed_many(&["hello", "world"]).unwrap_err();
    assert_eq!(error.reason_tag(), "InvalidOutputError");
}

#[test]
fn embed_many_empty_bypasses_provider() {
    // EmbeddingModel.test.ts:178 — embedMany([]) returns empty usage without calling the provider.
    let (model, seen) = recording(|_| panic!("provider should not be called"));
    let response = model.embed_many(&[]).unwrap();
    assert!(seen.lock().expect("seen").is_empty());
    assert!(response.embeddings.is_empty());
    assert_eq!(response.usage.input_tokens, None);
}
