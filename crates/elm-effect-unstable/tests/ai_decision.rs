//! `packages/effect/test/ai/DecisionModel.test.ts` decision witnesses.
//!
//! The `Schema` half of the TS surface (input codecs, `FiniteFromString`,
//! `toCodecJson`, encoding services, `Model.make` provider metadata) has no
//! counterpart in this crate: the caller encodes its input to JSON and the
//! provider sees that JSON as `state`. Ported here is the decision algebra:
//! constructors with their throwing contracts, single-call `decide` with
//! per-decision validation, and usage.

use elm_effect_unstable::ai::{
    AiError, AiReason, DecideResponse, Decision, DecisionAnswer, DecisionDefinition, DecisionModel,
    DecisionProviderResponse, DecisionUsage, ProviderAnswer, ProviderOptions, decide,
};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn classify_billing() -> Decision {
    Decision::classify(
        "Which team should handle this",
        &[
            ("billing", "payments"),
            ("technical", "bugs"),
            ("sales", "pricing"),
        ],
    )
    .expect("classify")
}

fn rate_frustration() -> Decision {
    Decision::rate("How frustrated", &["calm", "frustrated", "angry"]).expect("rate")
}

fn probability_urgent() -> Decision {
    Decision::probability(
        "The message is time-sensitive",
        Some(("No time pressure", "Needs action now")),
    )
}

fn triage() -> DecisionDefinition {
    DecisionDefinition::new(vec![
        ("department".to_owned(), classify_billing()),
        ("frustration".to_owned(), rate_frustration()),
        ("urgent".to_owned(), probability_urgent()),
    ])
    .expect("triage")
}

fn probs(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

fn triage_answers() -> HashMap<String, ProviderAnswer> {
    HashMap::from([
        (
            "department".to_owned(),
            ProviderAnswer::Classify {
                label: "billing".to_owned(),
                probabilities: probs(&[("billing", 0.8), ("technical", 0.15), ("sales", 0.05)]),
                confidence: Some(0.8),
            },
        ),
        (
            "frustration".to_owned(),
            ProviderAnswer::Rate {
                rating: 1.4,
                probabilities: probs(&[("calm", 0.1), ("frustrated", 0.5), ("angry", 0.4)]),
                confidence: Some(0.5),
            },
        ),
        (
            "urgent".to_owned(),
            ProviderAnswer::Probability { probability: 0.9 },
        ),
    ])
}

fn usage_none() -> DecisionUsage {
    DecisionUsage {
        input_tokens: None,
        output_tokens: None,
    }
}

fn succeed_with(answers: HashMap<String, ProviderAnswer>, usage: DecisionUsage) -> DecisionModel {
    DecisionModel::new(move |_| {
        Ok(DecisionProviderResponse {
            answers: answers.clone(),
            usage: usage.clone(),
        })
    })
}

fn invalid_output(result: &Result<DecideResponse, AiError>) -> bool {
    matches!(
        result,
        Err(error) if error.reason_tag() == "InvalidOutputError"
    )
}

// --- Decision constructors ---

#[test]
fn make_keeps_the_input_schema_and_decisions() {
    // DecisionModel.test.ts:93 — the definition keeps every decision in order,
    // tagged by kind. (The input schema itself is not representable: there is
    // no Schema layer here, the input is caller-encoded JSON.)
    let definition = triage();
    let keys: Vec<&str> = definition
        .decisions
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(keys, ["department", "frustration", "urgent"]);
    assert_eq!(definition.decisions[0].1.tag(), "Classify");
    assert_eq!(definition.decisions[1].1.tag(), "Rate");
    assert_eq!(definition.decisions[2].1.tag(), "Probability");
}

#[test]
fn constructors_keep_instructions_and_criteria() {
    // DecisionModel.test.ts:101 — instructions and criteria survive construction.
    let Decision::Classify(classify) = classify_billing() else {
        unreachable!()
    };
    assert_eq!(classify.instructions, "Which team should handle this");
    assert_eq!(
        classify.criteria,
        [
            ("billing".to_owned(), "payments".to_owned()),
            ("technical".to_owned(), "bugs".to_owned()),
            ("sales".to_owned(), "pricing".to_owned()),
        ]
    );
    let Decision::Rate(rate) = rate_frustration() else {
        unreachable!()
    };
    assert_eq!(rate.criteria, ["calm", "frustrated", "angry"]);
    let Decision::Probability(probability) = probability_urgent() else {
        unreachable!()
    };
    assert_eq!(
        probability.criteria,
        Some(("No time pressure".to_owned(), "Needs action now".to_owned()))
    );
}

#[test]
fn probability_accepts_omitted_criteria() {
    // DecisionModel.test.ts:115 — no criteria is `None`, not a default pair.
    let Decision::Probability(probability) = Decision::probability("Needs action now", None) else {
        unreachable!()
    };
    assert_eq!(probability.instructions, "Needs action now");
    assert_eq!(probability.criteria, None);
}

#[test]
fn probability_preserves_explicit_criteria_including_empty_descriptions() {
    // DecisionModel.test.ts:123 — an empty description is kept verbatim.
    let Decision::Probability(probability) =
        Decision::probability("Is urgent", Some(("", "Needs action now")))
    else {
        unreachable!()
    };
    assert_eq!(probability.instructions, "Is urgent");
    assert_eq!(
        probability.criteria,
        Some(("".to_owned(), "Needs action now".to_owned()))
    );
}

#[test]
fn make_fails_on_empty_decisions() {
    // DecisionModel.test.ts:131 — an empty decision map is rejected.
    assert!(DecisionDefinition::new(vec![]).is_err());
}

#[test]
fn rate_fails_with_fewer_than_two_criteria() {
    // DecisionModel.test.ts:135 — a single level cannot form a scale.
    assert!(Decision::rate("How much", &["only"]).is_err());
}

// --- decide plumbing ---

#[test]
fn probability_without_criteria_reaches_the_provider_and_returns_an_answer() {
    // DecisionModel.test.ts:141 — the provider sees the string state and the
    // decision (criteria `None` intact) in a single call.
    let definition = DecisionDefinition::new(vec![(
        "urgent".to_owned(),
        Decision::probability("Needs action now", None),
    )])
    .expect("definition");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let probe = seen.clone();
    let model = DecisionModel::new(move |options: ProviderOptions| {
        probe
            .lock()
            .expect("probe")
            .push((options.state, options.decisions.len()));
        Ok(DecisionProviderResponse {
            answers: HashMap::from([(
                "urgent".to_owned(),
                ProviderAnswer::Probability { probability: 0.9 },
            )]),
            usage: usage_none(),
        })
    });
    let response = model
        .decide(&definition, Value::String("Help now".to_owned()))
        .expect("decide");
    let calls = seen.lock().expect("probe");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, Value::String("Help now".to_owned()));
    assert_eq!(calls[0].1, 1);
    assert_eq!(definition.decisions[0].1.tag(), "Probability");
    let Decision::Probability(urgent) = &definition.decisions[0].1 else {
        unreachable!()
    };
    assert_eq!(urgent.criteria, None);
    assert_eq!(response.answers.len(), 1);
}

#[test]
fn preserves_weird_rate_probability_keys() {
    // DecisionModel.test.ts:170 — `__proto__`-style labels are ordinary keys
    // here (Rust maps have no prototype chain) and round-trip intact.
    for key in ["__proto__", "constructor", "toString"] {
        let definition = DecisionDefinition::new(vec![(
            "intensity".to_owned(),
            Decision::rate("How intense", &["low", key, "high"]).expect("rate"),
        )])
        .expect("definition");
        let model = succeed_with(
            HashMap::from([(
                "intensity".to_owned(),
                ProviderAnswer::Rate {
                    rating: 1.1,
                    probabilities: probs(&[("low", 0.1), (key, 0.7), ("high", 0.2)]),
                    confidence: None,
                },
            )]),
            usage_none(),
        );
        let response = model.decide(&definition, json!("Help")).expect("decide");
        let answer = response.answers.get("intensity").expect("answer");
        let DecisionAnswer::Rate(rate) = answer else {
            unreachable!()
        };
        assert!(rate.probabilities.contains_key(key));
        assert_eq!(rate.probabilities.get(key), Some(&0.7));
        assert_eq!(rate.label, key);
        assert_eq!(rate.rating, 1.1);
    }
}

#[test]
fn preserves_weird_answer_keys() {
    // DecisionModel.test.ts:197 — `__proto__`-style decision names are
    // ordinary keys and survive `decide` intact.
    for key in ["__proto__", "constructor", "toString"] {
        let definition = DecisionDefinition::new(vec![(
            key.to_owned(),
            Decision::probability("Needs action now", Some(("No", "Yes"))),
        )])
        .expect("definition");
        let model = succeed_with(
            HashMap::from([(
                key.to_owned(),
                ProviderAnswer::Probability { probability: 0.75 },
            )]),
            usage_none(),
        );
        let response = model.decide(&definition, json!("Help")).expect("decide");
        assert_eq!(response.answers.len(), 1);
        assert!(response.answers.contains_key(key));
    }
}

#[test]
fn preserves_weird_classification_probability_keys() {
    // DecisionModel.test.ts:219 — hostile labels inside a classification
    // distribution are ordinary keys and survive intact.
    for key in ["__proto__", "constructor", "toString"] {
        let definition = DecisionDefinition::new(vec![(
            "category".to_owned(),
            Decision::classify(
                "Choose a category",
                &[(key, "Special category"), ("ordinary", "Ordinary category")],
            )
            .expect("classify"),
        )])
        .expect("definition");
        let mut distribution = Map::new();
        distribution.insert(key.to_owned(), json!(0.75));
        distribution.insert("ordinary".to_owned(), json!(0.25));
        let model = succeed_with(
            HashMap::from([(
                "category".to_owned(),
                ProviderAnswer::Classify {
                    label: key.to_owned(),
                    probabilities: probs(&[(key, 0.75), ("ordinary", 0.25)]),
                    confidence: None,
                },
            )]),
            usage_none(),
        );
        let response = model.decide(&definition, json!("Help")).expect("decide");
        let DecisionAnswer::Classify(answer) = response.answers.get("category").expect("answer")
        else {
            unreachable!()
        };
        assert!(answer.probabilities.contains_key(key));
        assert_eq!(answer.probabilities.get(key), Some(&0.75));
        assert_eq!(answer.label, key);
    }
}

#[test]
fn round_trips_usage_with_undefined_tokens_through_json() {
    // DecisionModel.test.ts:247 — absent token counts encode to `{}` and back.
    let usage = usage_none();
    let encoded = usage.to_json();
    assert_eq!(encoded, json!({}));
    let round_tripped =
        serde_json::from_value::<Value>(serde_json::to_value(&encoded).expect("value"))
            .expect("json");
    let decoded = DecisionUsage::from_json(&round_tripped).expect("decodes");
    assert_eq!(
        decoded,
        DecisionUsage {
            input_tokens: None,
            output_tokens: None
        }
    );
}

#[test]
fn decide_returns_answers_keyed_by_decision_with_usage() {
    // DecisionModel.test.ts:259 — the full triage answers every decision and
    // reports usage; the probability answer carries no confidence.
    let definition = triage();
    let model = succeed_with(
        triage_answers(),
        DecisionUsage {
            input_tokens: Some(12.0),
            output_tokens: None,
        },
    );
    let response = model
        .decide(&definition, json!({"subject": "x"}))
        .expect("decide");
    let mut keys: Vec<&str> = response.answers.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["department", "frustration", "urgent"]);
    let DecisionAnswer::Classify(department) = &response.answers["department"] else {
        unreachable!()
    };
    assert_eq!(department.label, "billing");
    assert_eq!(
        department.probabilities,
        probs(&[("billing", 0.8), ("technical", 0.15), ("sales", 0.05)])
    );
    assert_eq!(department.confidence, Some(0.8));
    let DecisionAnswer::Rate(frustration) = &response.answers["frustration"] else {
        unreachable!()
    };
    assert_eq!(frustration.rating, 1.4);
    assert_eq!(frustration.label, "frustrated");
    assert_eq!(frustration.confidence, Some(0.5));
    let DecisionAnswer::Probability(urgent) = &response.answers["urgent"] else {
        unreachable!()
    };
    assert_eq!(urgent.probability, 0.9);
    assert_eq!(response.usage.input_tokens, Some(12.0));
    assert_eq!(response.usage.output_tokens, None);
}

#[test]
fn classify_and_rate_retain_distributions_when_the_provider_omits_confidence() {
    // DecisionModel.test.ts:284 — a missing confidence stays missing; the
    // distributions are still validated and returned.
    let strip = |mut answers: HashMap<String, ProviderAnswer>| {
        for answer in answers.values_mut() {
            match answer {
                ProviderAnswer::Classify { confidence, .. }
                | ProviderAnswer::Rate { confidence, .. } => *confidence = None,
                ProviderAnswer::Probability { .. } => {}
            }
        }
        answers
    };
    let model = succeed_with(strip(triage_answers()), usage_none());
    let response = model
        .decide(&triage(), json!({"subject": "x"}))
        .expect("decide");
    let DecisionAnswer::Classify(department) = &response.answers["department"] else {
        unreachable!()
    };
    assert_eq!(department.confidence, None);
    assert_eq!(
        department.probabilities,
        probs(&[("billing", 0.8), ("technical", 0.15), ("sales", 0.05)])
    );
    let DecisionAnswer::Rate(frustration) = &response.answers["frustration"] else {
        unreachable!()
    };
    assert_eq!(frustration.confidence, None);
    assert_eq!(
        frustration.probabilities,
        probs(&[("calm", 0.1), ("frustrated", 0.5), ("angry", 0.4)])
    );
}

#[test]
fn the_service_exposes_the_same_decide_operation() {
    // DecisionModel.test.ts:316 — the model method and the free `decide`
    // function answer identically.
    let model = succeed_with(triage_answers(), usage_none());
    let via_method = model
        .decide(&triage(), json!({"subject": "x"}))
        .expect("method");
    let via_function = decide(&model, &triage(), json!({"subject": "x"})).expect("function");
    assert_eq!(via_method.answers, via_function.answers);
    assert_eq!(via_method.usage, via_function.usage);
}

#[test]
fn the_provider_receives_the_encoded_input_as_state_and_every_decision_in_one_call() {
    // DecisionModel.test.ts:327 — one call, JSON-encoded state (the numeric
    // priority encodes as the string `"3"`), decisions passed through.
    let seen: Arc<Mutex<Vec<(Value, usize)>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = seen.clone();
    let model = DecisionModel::new(move |options: ProviderOptions| {
        probe
            .lock()
            .expect("probe")
            .push((options.state, options.decisions.len()));
        Ok(DecisionProviderResponse {
            answers: triage_answers(),
            usage: usage_none(),
        })
    });
    let state = json!({"subject": "Card was charged twice", "priority": "3"});
    model.decide(&triage(), state.clone()).expect("decide");
    let calls = seen.lock().expect("calls");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, state);
    assert_eq!(calls[0].1, 3);
}

#[test]
fn a_string_input_is_passed_to_the_provider_as_a_string_state() {
    // DecisionModel.test.ts:349 — a JSON string input stays a string state.
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = seen.clone();
    let definition = DecisionDefinition::new(vec![(
        "tone".to_owned(),
        Decision::classify(
            "Tone of the message",
            &[("positive", "happy"), ("negative", "unhappy")],
        )
        .expect("classify"),
    )])
    .expect("definition");
    let model = DecisionModel::new(move |options: ProviderOptions| {
        probe.lock().expect("probe").push(options.state);
        Ok(DecisionProviderResponse {
            answers: HashMap::from([(
                "tone".to_owned(),
                ProviderAnswer::Classify {
                    label: "positive".to_owned(),
                    probabilities: probs(&[("positive", 0.95), ("negative", 0.05)]),
                    confidence: Some(0.95),
                },
            )]),
            usage: usage_none(),
        })
    });
    let response = model
        .decide(&definition, Value::String("I love this".to_owned()))
        .expect("decide");
    assert_eq!(
        *seen.lock().expect("states"),
        [Value::String("I love this".to_owned())]
    );
    let DecisionAnswer::Classify(answer) = &response.answers["tone"] else {
        unreachable!()
    };
    assert_eq!(answer.label, "positive");
}

#[test]
fn derives_json_codecs_for_nested_input_before_calling_the_provider() {
    // DecisionModel.test.ts:386 — the caller-side encoder lowers bigints to
    // strings, dates to ISO strings, and numbers-as-strings before the call.
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = seen.clone();
    let model = DecisionModel::new(move |options: ProviderOptions| {
        probe.lock().expect("probe").push(options.state);
        Ok(DecisionProviderResponse {
            answers: triage_answers(),
            usage: usage_none(),
        })
    });
    let encoded = json!({
        "amount": "42",
        "createdAt": "2026-01-01T00:00:00.000Z",
        "priority": "3",
        "values": ["1", "2"],
    });
    model.decide(&triage(), encoded.clone()).expect("decide");
    assert_eq!(*seen.lock().expect("states"), [encoded]);
}

#[test]
fn json_encoding_distinguishes_absent_optional_fields_from_explicit_undefined() {
    // DecisionModel.test.ts:415 — an absent field stays absent; an explicit
    // null encodes as `null` (Rust: the encoder builds the map itself).
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = seen.clone();
    let model = DecisionModel::new(move |options: ProviderOptions| {
        probe.lock().expect("probe").push(options.state);
        Ok(DecisionProviderResponse {
            answers: triage_answers(),
            usage: usage_none(),
        })
    });
    model.decide(&triage(), json!({})).expect("absent");
    let mut explicit = Map::new();
    explicit.insert("note".to_owned(), Value::Null);
    model
        .decide(&triage(), Value::Object(explicit))
        .expect("explicit");
    assert_eq!(
        *seen.lock().expect("states"),
        [json!({}), json!({"note": null})]
    );
}

#[test]
fn json_codec_encoding_failures_become_invalid_user_input_error_before_the_provider_runs() {
    // DecisionModel.test.ts:433 — a value with no JSON codec fails encoding
    // before the provider is touched.
    let calls = Arc::new(Mutex::new(0usize));
    let probe = calls.clone();
    let model = DecisionModel::new(move |_| {
        *probe.lock().expect("calls") += 1;
        Ok(DecisionProviderResponse {
            answers: triage_answers(),
            usage: usage_none(),
        })
    });
    let result = elm_effect_unstable::ai::decide_encoded(&model, &triage(), &"unencodable", |_| {
        Err::<Value, String>("no JSON codec for input".to_owned())
    });
    assert_eq!(*calls.lock().expect("calls"), 0);
    let Err(error) = result else { unreachable!() };
    assert!(AiError::is_ai_error(&error));
    assert_eq!(error.reason_tag(), "InvalidUserInputError");
}

#[test]
fn input_encoding_failures_surface_as_ai_error() {
    // DecisionModel.test.ts:484 — a min-length violation fails encoding as
    // `InvalidUserInputError`; the provider never runs.
    let calls = Arc::new(Mutex::new(0usize));
    let probe = calls.clone();
    let definition = DecisionDefinition::new(vec![(
        "urgent".to_owned(),
        Decision::probability("Needs action now", Some(("No", "Yes"))),
    )])
    .expect("definition");
    let model = DecisionModel::new(move |_| {
        *probe.lock().expect("calls") += 1;
        Ok(DecisionProviderResponse {
            answers: HashMap::new(),
            usage: usage_none(),
        })
    });
    let result = elm_effect_unstable::ai::decide_encoded(&model, &definition, &"hi", |input| {
        if input.len() < 3 {
            return Err::<Value, String>("too short".to_owned());
        }
        Ok(Value::String((*input).to_owned()))
    });
    assert_eq!(*calls.lock().expect("calls"), 0);
    let Err(error) = result else { unreachable!() };
    assert!(AiError::is_ai_error(&error));
    assert_eq!(error.reason_tag(), "InvalidUserInputError");
}

#[test]
fn provider_ai_error_propagates_through_decide() {
    // DecisionModel.test.ts:512 — a provider failure is returned unchanged.
    let expected = AiError::new(AiReason::Unknown)
        .with_description("boom")
        .with_context("DecisionModelTest", "decide");
    let failure = expected.clone();
    let model =
        DecisionModel::new(move |_| Err::<DecisionProviderResponse, AiError>(failure.clone()));
    let result = model.decide(&triage(), json!({"subject": "x"}));
    assert_eq!(result, Err(expected));
}

#[test]
fn decide_fails_with_invalid_output_error_when_an_answer_is_missing() {
    // DecisionModel.test.ts:528 — every decision needs an answer.
    let mut answers = triage_answers();
    answers.remove("urgent");
    let result = succeed_with(answers, usage_none()).decide(&triage(), json!({}));
    assert!(invalid_output(&result));
}

#[test]
fn decide_fails_with_invalid_output_error_when_a_classify_label_is_not_a_criteria_key() {
    // DecisionModel.test.ts:540 — an unknown label is rejected.
    let mut answers = triage_answers();
    if let Some(ProviderAnswer::Classify { label, .. }) = answers.get_mut("department") {
        *label = "legal".to_owned();
    }
    let result = succeed_with(answers, usage_none()).decide(&triage(), json!({}));
    assert!(invalid_output(&result));
}

#[test]
fn decide_fails_with_invalid_output_error_when_a_classify_distribution_is_incomplete() {
    // DecisionModel.test.ts:552 — every criteria label needs a probability.
    let mut answers = triage_answers();
    answers.insert(
        "department".to_owned(),
        ProviderAnswer::Classify {
            label: "billing".to_owned(),
            probabilities: probs(&[("billing", 1.0)]),
            confidence: None,
        },
    );
    let result = succeed_with(answers, usage_none()).decide(&triage(), json!({}));
    assert!(invalid_output(&result));
}

#[test]
fn decide_fails_with_invalid_output_error_when_a_rate_distribution_is_incomplete() {
    // DecisionModel.test.ts:564 — every level needs a probability.
    let mut answers = triage_answers();
    answers.insert(
        "frustration".to_owned(),
        ProviderAnswer::Rate {
            rating: 1.4,
            probabilities: probs(&[("calm", 0.5), ("angry", 0.5)]),
            confidence: None,
        },
    );
    let result = succeed_with(answers, usage_none()).decide(&triage(), json!({}));
    assert!(invalid_output(&result));
}

#[test]
fn decide_fails_with_invalid_output_error_when_a_probability_is_outside_zero_to_one() {
    // DecisionModel.test.ts:576 — probabilities stay in `[0, 1]`.
    let mut answers = triage_answers();
    answers.insert(
        "urgent".to_owned(),
        ProviderAnswer::Probability { probability: 1.5 },
    );
    let result = succeed_with(answers, usage_none()).decide(&triage(), json!({}));
    assert!(invalid_output(&result));
}

// --- Decision validation contracts ---

#[test]
fn classify_rejects_zero_or_one_labels() {
    // DecisionModel.test.ts:604 — classification needs at least two labels.
    assert!(Decision::classify("Choose", &[]).is_err());
    assert!(Decision::classify("Choose", &[("only", "Only label")]).is_err());
}

#[test]
fn rate_rejects_duplicate_levels() {
    // DecisionModel.test.ts:609 — levels must be distinct.
    assert!(Decision::rate("Rate", &["low", "high", "low"]).is_err());
}

#[test]
fn rate_derives_the_argmax_label_without_a_provider_label() {
    // DecisionModel.test.ts:613 — the provider `Rate` answer carries no label
    // (there is no label field to read); the core derives it from argmax.
    let answers = triage_answers();
    let response = succeed_with(answers, usage_none())
        .decide(&triage(), json!({}))
        .expect("decide");
    let DecisionAnswer::Rate(frustration) = &response.answers["frustration"] else {
        unreachable!()
    };
    assert_eq!(frustration.label, "frustrated");
}

#[test]
fn rate_breaks_ties_in_criteria_order() {
    // DecisionModel.test.ts:632 — argmax wins; ties fall to the first criteria
    // entry, never to provider key order. (A conflicting provider label cannot
    // occur here: the `Rate` answer has no label field.)
    for (name, distribution, expected) in [
        (
            "ignores a conflicting provider label",
            probs(&[("calm", 0.1), ("frustrated", 0.5), ("angry", 0.4)]),
            "frustrated",
        ),
        (
            "breaks ties in criteria order",
            probs(&[("angry", 0.5), ("frustrated", 0.5), ("calm", 0.0)]),
            "frustrated",
        ),
    ] {
        let mut answers = triage_answers();
        answers.insert(
            "frustration".to_owned(),
            ProviderAnswer::Rate {
                rating: 1.0,
                probabilities: distribution,
                confidence: None,
            },
        );
        let response = succeed_with(answers, usage_none()).decide(&triage(), json!({}));
        let Ok(decided) = response else {
            panic!("{name} decided")
        };
        let DecisionAnswer::Rate(frustration) = &decided.answers["frustration"] else {
            unreachable!()
        };
        assert_eq!(frustration.label, expected, "{name}");
    }
}

#[test]
fn classify_retains_the_provider_label_even_when_it_is_not_the_argmax() {
    // DecisionModel.test.ts:644 — classification keeps the provider's label.
    let mut answers = triage_answers();
    if let Some(ProviderAnswer::Classify { label, .. }) = answers.get_mut("department") {
        *label = "sales".to_owned();
    }
    let response = succeed_with(answers, usage_none())
        .decide(&triage(), json!({}))
        .expect("decide");
    let DecisionAnswer::Classify(department) = &response.answers["department"] else {
        unreachable!()
    };
    assert_eq!(department.label, "sales");
}

#[test]
fn classify_and_rate_reject_confidence_outside_zero_to_one() {
    // DecisionModel.test.ts:655 — confidence must be a unit-interval number.
    for key in ["department", "frustration"] {
        for confidence in [
            -f64::EPSILON,
            1.0 + f64::EPSILON,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            let mut answers = triage_answers();
            match answers.get_mut(key) {
                Some(ProviderAnswer::Classify {
                    confidence: slot, ..
                })
                | Some(ProviderAnswer::Rate {
                    confidence: slot, ..
                }) => *slot = Some(confidence),
                _ => unreachable!(),
            }
            let result = succeed_with(answers, usage_none()).decide(&triage(), json!({}));
            assert!(
                invalid_output(&result),
                "{key} rejects confidence {confidence}"
            );
        }
    }
}

#[test]
fn classify_and_rate_accept_confidence_boundaries() {
    // DecisionModel.test.ts:665 — `0` and `1` are valid confidences.
    for key in ["department", "frustration"] {
        for confidence in [0.0, 1.0] {
            let mut answers = triage_answers();
            match answers.get_mut(key) {
                Some(ProviderAnswer::Classify {
                    confidence: slot, ..
                })
                | Some(ProviderAnswer::Rate {
                    confidence: slot, ..
                }) => *slot = Some(confidence),
                _ => unreachable!(),
            }
            let response = succeed_with(answers, usage_none())
                .decide(&triage(), json!({}))
                .expect("decide");
            let kept = match &response.answers[key] {
                DecisionAnswer::Classify(answer) => answer.confidence,
                DecisionAnswer::Rate(answer) => answer.confidence,
                _ => unreachable!(),
            };
            assert_eq!(kept, Some(confidence), "{key} accepts {confidence}");
        }
    }
}

#[test]
fn distributions_must_sum_to_one_within_tolerance() {
    // DecisionModel.test.ts:690 — distributions summing to 1 within `1e-6`
    // pass unchanged; anything past that fails.
    for key in ["department", "frustration"] {
        let levels: &[&str] = if key == "department" {
            &["billing", "technical", "sales"]
        } else {
            &["calm", "frustrated", "angry"]
        };
        for (total, accepted) in [
            (1.0, true),
            (1.0 - 0.5e-6, true),
            (1.0 + 0.5e-6, true),
            (1.0 + 1e-6, true),
            (0.9999990000000001, true),
            (0.9999989999999999, false),
            (1.0 + 1e-6 + f64::EPSILON, false),
            (0.99, false),
            (1.01, false),
            (0.0, false),
        ] {
            let distribution: HashMap<String, f64> = [
                (levels[0].to_owned(), total / 2.0),
                (levels[1].to_owned(), total / 2.0),
                (levels[2].to_owned(), 0.0),
            ]
            .into_iter()
            .collect();
            let mut answers = triage_answers();
            match answers.get_mut(key) {
                Some(ProviderAnswer::Classify { probabilities, .. })
                | Some(ProviderAnswer::Rate { probabilities, .. }) => {
                    *probabilities = distribution.clone();
                }
                _ => unreachable!(),
            }
            let result = succeed_with(answers, usage_none()).decide(&triage(), json!({}));
            if accepted {
                let Ok(decided) = result else {
                    panic!("{key} accepts total {total}")
                };
                let kept = match &decided.answers[key] {
                    DecisionAnswer::Classify(answer) => &answer.probabilities,
                    DecisionAnswer::Rate(answer) => &answer.probabilities,
                    _ => unreachable!(),
                };
                assert_eq!(kept, &distribution, "{key} keeps total {total}");
            } else {
                assert!(invalid_output(&result), "{key} rejects total {total}");
            }
        }
    }
}

#[test]
fn rate_rejects_ratings_outside_its_scale() {
    // DecisionModel.test.ts:718 — ratings stay finite within `[0, levels - 1]`.
    for rating in [
        -f64::EPSILON,
        2.0 + 2.0 * f64::EPSILON,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        let mut answers = triage_answers();
        if let Some(ProviderAnswer::Rate { rating: slot, .. }) = answers.get_mut("frustration") {
            *slot = rating;
        }
        let result = succeed_with(answers, usage_none()).decide(&triage(), json!({}));
        assert!(invalid_output(&result), "rejects rating {rating}");
    }
}

#[test]
fn rate_accepts_ratings_on_its_scale() {
    // DecisionModel.test.ts:728 — boundary and interior ratings pass through.
    for rating in [0.0, 1.4, 2.0] {
        let mut answers = triage_answers();
        if let Some(ProviderAnswer::Rate { rating: slot, .. }) = answers.get_mut("frustration") {
            *slot = rating;
        }
        let response = succeed_with(answers, usage_none())
            .decide(&triage(), json!({}))
            .expect("decide");
        let DecisionAnswer::Rate(frustration) = &response.answers["frustration"] else {
            unreachable!()
        };
        assert_eq!(frustration.rating, rating, "accepts rating {rating}");
    }
}

#[test]
fn extra_provider_keys_never_leak_into_answers() {
    // DecisionModel.test.ts:738 — unknown decisions and unknown labels are
    // dropped; only the definition's keys and labels survive. (Unknown scalar
    // fields cannot occur: the answer types have no such fields.)
    let mut answers = triage_answers();
    answers.insert(
        "extraDecision".to_owned(),
        ProviderAnswer::Probability { probability: 0.5 },
    );
    answers.insert(
        "department".to_owned(),
        ProviderAnswer::Classify {
            label: "billing".to_owned(),
            probabilities: probs(&[
                ("billing", 0.8),
                ("technical", 0.15),
                ("sales", 0.05),
                ("extra", 1.0),
            ]),
            confidence: Some(0.8),
        },
    );
    answers.insert(
        "frustration".to_owned(),
        ProviderAnswer::Rate {
            rating: 1.4,
            probabilities: probs(&[
                ("calm", 0.1),
                ("frustrated", 0.5),
                ("angry", 0.4),
                ("extra", 1.0),
            ]),
            confidence: Some(0.5),
        },
    );
    let response = succeed_with(answers, usage_none())
        .decide(&triage(), json!({}))
        .expect("decide");
    let mut keys: Vec<&str> = response.answers.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["department", "frustration", "urgent"]);
    let DecisionAnswer::Classify(department) = &response.answers["department"] else {
        unreachable!()
    };
    assert_eq!(
        department.probabilities,
        probs(&[("billing", 0.8), ("technical", 0.15), ("sales", 0.05)])
    );
    let DecisionAnswer::Rate(frustration) = &response.answers["frustration"] else {
        unreachable!()
    };
    assert_eq!(
        frustration.probabilities,
        probs(&[("calm", 0.1), ("frustrated", 0.5), ("angry", 0.4)])
    );
    let DecisionAnswer::Probability(urgent) = &response.answers["urgent"] else {
        unreachable!()
    };
    assert_eq!(urgent.probability, 0.9);
}
