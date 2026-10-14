//! `makeFilter`: a refinement that answers with a value, not a boolean.
//!
//! The pin lets a filter return `undefined`, `true`, `false`, a message, an
//! issue, an object pointing at a path, or an array of those. That is a small
//! algebra: a pass, one issue, or several. This module is the algebra and the
//! chain around it, so a chain that is decoded with every issue collected
//! reports all of them and a filter built to abort stops the chain at itself.

use std::sync::Arc;

use crate::{Schema, SchemaError, Unknown};

/// One filter failure: a message, at a path relative to the filter's position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterIssue {
    /// The segments the filter pointed at (`{ path: ["a"] }`). Empty means the
    /// value itself, which is where a message or an issue sits.
    pub path: Vec<String>,
    /// The message, which is the whole of what a caller sees.
    pub message: String,
}

impl FilterIssue {
    /// An issue at the value itself.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            path: Vec::new(),
            message: message.into(),
        }
    }

    /// The same issue at a named path.
    pub fn at(mut self, segment: impl Into<String>) -> Self {
        self.path.push(segment.into());
        self
    }

    /// The port's path form: `$`, or `$.a.b` for a pointed issue.
    pub fn rendered_path(&self) -> String {
        let mut out = "$".to_string();
        for segment in &self.path {
            out.push('.');
            out.push_str(segment);
        }
        out
    }
}

/// What a filter answered. The kinds are the pin's behaviours: a pass, a bare
/// `false`, one issue, or a list that is itself a composite.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterOutcome {
    /// `undefined` or `true`: the value passes.
    Pass,
    /// `false`, with no message of its own. The chain renders it from the
    /// filter's annotations, which is where the pin keeps that message.
    False,
    /// A message, an issue, or an object carrying a path.
    Fail(FilterIssue),
    /// An array result. Empty passes, one element collapses to that element,
    /// and more than one is a composite.
    Many(Vec<FilterIssue>),
}

impl FilterOutcome {
    /// A boolean answer: `true` is a pass and `false` is the bare refusal.
    pub fn from_bool(holds: bool) -> Self {
        if holds { Self::Pass } else { Self::False }
    }

    /// The issues this outcome reports for a filter, empty for a pass. An array
    /// result collapses at one element so a caller never sees a one-member
    /// composite, and a bare `false` becomes the filter's `message` annotation
    /// or the pin's `Expected <filter>`.
    pub fn issues(self, annotation: &[(String, String)]) -> Vec<FilterIssue> {
        match self {
            Self::Pass => Vec::new(),
            Self::False => vec![FilterIssue::new(bare_refusal(annotation))],
            Self::Fail(issue) => vec![issue],
            Self::Many(issues) => issues,
        }
    }
}

/// What a filter that answered `false` names itself as.
fn bare_refusal(annotation: &[(String, String)]) -> String {
    annotation
        .iter()
        .find(|(key, _)| key == "message")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| "Expected <filter>".to_string())
}

/// One filter in a chain: its predicate, the annotations the pin attaches
/// (`{ title: "...", message: "..." }`), and whether failing it stops the chain
/// (`makeFilter(fn, annotations, true)`).
#[derive(Clone)]
pub struct FilterSpec {
    /// The predicate: what the filter answers for a value.
    pub filter: Arc<dyn Fn(&Unknown) -> FilterOutcome + Send + Sync>,
    /// The annotation pairs, in the order the pin prints them.
    pub annotation: Vec<(String, String)>,
    /// Whether a failure here stops the chain rather than collecting the rest.
    pub abort: bool,
}

impl FilterSpec {
    /// A filter with no annotations that does not abort.
    pub fn new(filter: impl Fn(&Unknown) -> FilterOutcome + Send + Sync + 'static) -> Self {
        Self {
            filter: Arc::new(filter),
            annotation: Vec::new(),
            abort: false,
        }
    }

    /// The same filter with annotation pairs.
    pub fn annotated(mut self, annotation: Vec<(String, String)>) -> Self {
        self.annotation = annotation;
        self
    }

    /// The same filter as an aborting one.
    pub fn aborting(mut self) -> Self {
        self.abort = true;
        self
    }
}

impl Schema<Unknown> {
    /// A chain of filters (`check(makeFilter(...), makeFilter(...))`). Every
    /// filter runs over the value in order, and what it answers is collected:
    /// with `collect_all` the chain reports every failing filter, and an
    /// aborting filter ends the collection at itself either way. A chain with no
    /// failures decodes through `base`, so the refinement wraps its base rather
    /// than replacing it.
    pub fn filtered(base: Schema<Unknown>, filters: Vec<FilterSpec>, collect_all: bool) -> Self {
        let base_kind = base.repr_kind();
        let annotation = filters
            .first()
            .map(|filter| filter.annotation.clone())
            .unwrap_or_default();
        let kind = crate::SchemaKind::Check {
            base: Box::new(base_kind),
            name: "makeFilter".to_string(),
            args: Vec::new(),
            annotation,
        };
        let inner = Arc::new(base);
        let decode_inner = inner.clone();
        let encode_inner = inner.clone();
        let chain = Arc::new(filters);
        let decode_chain = chain.clone();
        let encode_chain = chain.clone();
        let mut out = Self::new(
            move |unknown| {
                let issues = run_filters(&decode_chain, &unknown, collect_all);
                if !issues.is_empty() {
                    return Err(issues_error(issues));
                }
                decode_inner.decode(unknown)
            },
            // The filters are the source schema's, so they decide the encode
            // side too: a value they refuse has no wire form rather than one
            // that silently violates the refinement on the way out.
            move |value| {
                let issues = run_filters(&encode_chain, value, false);
                if !issues.is_empty() {
                    return Err(issues_error(issues));
                }
                encode_inner.try_encode(value)
            },
            kind,
        );
        out.formatter = inner.formatter.clone();
        out.wrapped = None;
        out
    }
}

/// Run a chain in order, returning the issues it reports.
fn run_filters(specs: &[FilterSpec], value: &Unknown, collect_all: bool) -> Vec<FilterIssue> {
    let mut issues = Vec::new();
    for spec in specs {
        let reported = (spec.filter)(value).issues(&spec.annotation);
        if reported.is_empty() {
            continue;
        }
        issues.extend(reported);
        // An aborting filter is the pin's "stop here"; otherwise the caller's
        // mode decides whether the rest of the chain still runs.
        if spec.abort || !collect_all {
            break;
        }
    }
    issues
}

/// One issue is that issue; more than one is the composite that prints one
/// message per line.
fn issues_error(mut issues: Vec<FilterIssue>) -> SchemaError {
    let to_error = |issue: FilterIssue| SchemaError::FilterFailed {
        path: issue.rendered_path(),
        message: issue.message,
    };
    let mut errors: Vec<SchemaError> = issues.drain(..).map(to_error).collect();
    if errors.len() == 1 {
        errors.pop().expect("one error")
    } else {
        SchemaError::Composite { issues: errors }
    }
}

/// One runnable check for the two rules that are easy to get wrong: an array
/// result collapses at one element rather than wrapping it, and an aborting
/// filter stops the chain even when every issue is collected.
#[cfg(test)]
mod tests {
    use super::*;

    fn failing(message: &str) -> FilterSpec {
        let message = message.to_string();
        FilterSpec::new(move |_| FilterOutcome::Fail(FilterIssue::new(message.clone())))
    }

    #[test]
    fn a_one_element_array_is_that_element() {
        assert_eq!(
            FilterOutcome::Many(vec![FilterIssue::new("only")]).issues(&[]),
            vec![FilterIssue::new("only")]
        );
        assert!(FilterOutcome::Many(Vec::new()).issues(&[]).is_empty());
        // A bare false takes the filter's own message when it carries one.
        assert_eq!(
            FilterOutcome::False.issues(&[("message".to_string(), "named".to_string())]),
            vec![FilterIssue::new("named")]
        );
        assert_eq!(
            FilterOutcome::False.issues(&[]),
            vec![FilterIssue::new("Expected <filter>")]
        );
    }

    #[test]
    fn an_aborting_filter_stops_the_collection() {
        let chain = vec![failing("first").aborting(), failing("second")];
        let issues = run_filters(&chain, &Unknown::Text("a".to_string()), true);
        assert_eq!(issues, vec![FilterIssue::new("first")]);
    }
}
