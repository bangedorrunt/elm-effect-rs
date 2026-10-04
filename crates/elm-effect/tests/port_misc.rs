//! Wave-5 misc: Graph, Utils, Formatter, JsonPointer, JsonPatch behavior
//! ported from the tag's suites. Modules load via `#[path]` includes (the
//! .15 precedent): no facade touch, no scope leak into lib.rs.
//!
//! Each test pins a behavior GROUP; residuals stay `unported` and are
//! listed in the close report instead of follow-up beads.

#[path = "../src/misc/formatter.rs"]
mod formatter;
#[path = "../src/misc/graph.rs"]
mod graph;
#[path = "../src/json.rs"]
mod json;
#[path = "../src/misc/json_patch.rs"]
mod json_patch;
#[path = "../src/misc/json_pointer.rs"]
mod json_pointer;
#[path = "../src/misc/utils.rs"]
mod utils;

use formatter::{Formatter, JsonFormatter, format_unknown};
use graph::Graph;
use json_patch::{Op, apply, diff};
use json_pointer::{
    Json, escape_token, format_uri_fragment, get, parse, parse_uri_fragment, set, unescape_token,
};
use std::collections::BTreeMap;
use utils::SingleShotGen;

fn obj(entries: &[(&str, Json)]) -> Json {
    Json::Obj(
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn strj(s: &str) -> Json {
    Json::Str(s.to_owned())
}

// --- Utils ---

#[test]
fn single_shot_yields_once() {
    let mut once = SingleShotGen::new(7);
    assert_eq!(once.next(), Some(7));
    assert_eq!(once.next(), None);
}

// --- Formatter ---

#[test]
fn format_unknown_primitives() {
    assert_eq!(format_unknown(&Json::Null), "null");
    assert_eq!(format_unknown(&Json::Bool(true)), "true");
    assert_eq!(format_unknown(&strj("a")), "\"a\"");
    assert_eq!(format_unknown(&Json::Num(123.0)), "123");
    assert_eq!(format_unknown(&Json::Num(1.5)), "1.5");
}

#[test]
fn format_unknown_compound_and_pretty() {
    let v = obj(&[("a", Json::Num(1.0)), ("b", Json::Num(2.0))]);
    assert_eq!(format_unknown(&v), "{\"a\":1,\"b\":2}");
    let pretty = JsonFormatter::pretty(2).format(&v);
    assert_eq!(pretty, "{\n  \"a\": 1,\n  \"b\": 2\n}");
    assert_eq!(JsonFormatter::new().format(&v), "{\"a\":1,\"b\":2}");
}

// --- JsonPointer ---

#[test]
fn pointer_escape_round_trip() {
    assert_eq!(escape_token("a/b~c"), "a~1b~0c");
    assert_eq!(unescape_token("a~1b~0c"), "a/b~c");
    assert_eq!(unescape_token("~01"), "~1");
    assert_eq!(escape_token(""), "");
    assert_eq!(escape_token("plain"), "plain");
}

#[test]
fn pointer_parse_get_set() {
    assert!(parse("").is_empty());
    assert_eq!(parse("/a/b"), ["a".to_owned(), "b".to_owned()]);
    assert_eq!(parse("/a~1b"), ["a/b".to_owned()]);
    let doc = obj(&[("a", obj(&[("b", Json::Num(1.0))]))]);
    assert_eq!(get(&doc, &parse("/a/b")), Some(&Json::Num(1.0)));
    assert_eq!(get(&doc, &parse("/missing")), None);
    let arr = Json::Arr(vec![Json::Num(1.0), Json::Num(2.0)]);
    assert_eq!(get(&arr, &parse("/1")), Some(&Json::Num(2.0)));
    let updated = set(&doc, &parse("/a/c"), Json::Num(9.0)).expect("settable");
    assert_eq!(get(&updated, &parse("/a/c")), Some(&Json::Num(9.0)));
    assert_eq!(get(&doc, &parse("/a/c")), None);
}

#[test]
fn pointer_uri_fragment() {
    assert_eq!(format_uri_fragment(&["a/b".to_owned()]), "#/a~1b");
    assert_eq!(parse_uri_fragment("#/a~1b"), Some(vec!["a/b".to_owned()]));
    assert_eq!(parse_uri_fragment("no-hash"), None);
}

// --- JsonPatch ---

#[test]
fn patch_apply_ops() {
    let doc = obj(&[("a", Json::Num(1.0))]);
    let out = apply(
        &doc,
        &[Op::Add {
            path: "/b".into(),
            value: Json::Num(2.0),
        }],
    )
    .expect("add");
    assert_eq!(get(&out, &parse("/b")), Some(&Json::Num(2.0)));
    let out = apply(
        &out,
        &[Op::Replace {
            path: "/a".into(),
            value: Json::Num(10.0),
        }],
    )
    .expect("replace");
    assert_eq!(get(&out, &parse("/a")), Some(&Json::Num(10.0)));
    assert!(
        apply(
            &out,
            &[Op::Replace {
                path: "/zz".into(),
                value: Json::Num(1.0)
            }]
        )
        .is_err()
    );
    let out = apply(&out, &[Op::Remove { path: "/a".into() }]).expect("remove");
    assert_eq!(get(&out, &parse("/a")), None);
    let out = apply(
        &out,
        &[Op::Copy {
            from: "/b".into(),
            path: "/c".into(),
        }],
    )
    .expect("copy");
    assert_eq!(get(&out, &parse("/c")), Some(&Json::Num(2.0)));
    let out = apply(
        &out,
        &[Op::Move {
            from: "/c".into(),
            path: "/d".into(),
        }],
    )
    .expect("move");
    assert_eq!(get(&out, &parse("/c")), None);
    assert_eq!(get(&out, &parse("/d")), Some(&Json::Num(2.0)));
    assert!(
        apply(
            &out,
            &[Op::Test {
                path: "/d".into(),
                value: Json::Num(2.0)
            }]
        )
        .is_ok()
    );
    assert!(
        apply(
            &out,
            &[Op::Test {
                path: "/d".into(),
                value: Json::Num(3.0)
            }]
        )
        .is_err()
    );
}

#[test]
fn patch_diff_and_round_trip() {
    let old = obj(&[("a", Json::Num(1.0)), ("gone", Json::Bool(true))]);
    let new = obj(&[("a", Json::Num(2.0)), ("b", strj("x"))]);
    let ops = diff(&old, &new);
    assert!(!ops.is_empty());
    let back = apply(&old, &ops).expect("diff applies");
    assert_eq!(back, new);
    assert!(diff(&old, &old).is_empty());
    // Arrays: removals descend so indexes never shift.
    let a = Json::Arr(vec![Json::Num(1.0), Json::Num(2.0), Json::Num(3.0)]);
    let b = Json::Arr(vec![Json::Num(1.0)]);
    let ops = diff(&a, &b);
    assert_eq!(ops.len(), 2);
    assert_eq!(apply(&a, &ops).expect("array diff"), b);
    // Escapes: '/' as '~1', '~' as '~0', '__proto__' an ordinary key.
    let o = obj(&[("a/b", Json::Num(1.0))]);
    let n = obj(&[("a/b", Json::Num(2.0)), ("__proto__", Json::Num(0.0))]);
    let ops = diff(&o, &n);
    assert_eq!(apply(&o, &ops).expect("escape diff"), n);
}

// --- Graph ---

fn triangle() -> Graph<&'static str, i32> {
    let mut g = Graph::directed();
    g.add_edge("a", "b", 1);
    g.add_edge("b", "c", 2);
    g.add_edge("a", "c", 5);
    g
}

#[test]
fn graph_nodes_and_edges() {
    let mut g = Graph::directed();
    assert!(g.add_node("a"));
    assert!(!g.add_node("a"));
    assert!(g.has_node(&"a"));
    assert!(!g.has_node(&"z"));
    g.add_edge("a", "b", 1);
    assert!(g.has_node(&"b"));
    assert!(g.has_edge(&"a", &"b"));
    assert!(!g.has_edge(&"b", &"a"));
    assert_eq!(g.edge_value(&"a", &"b"), Some(&1));
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.edge_count(), 1);
    assert!(g.remove_edge(&"a", &"b"));
    assert!(!g.has_edge(&"a", &"b"));
    assert!(g.remove_node(&"a"));
    assert!(!g.has_node(&"a"));
    let mut u = Graph::<&str, i32>::undirected();
    u.add_edge("a", "b", 1);
    assert!(u.has_edge(&"b", &"a"));
    assert_eq!(u.degree(&"a"), 1);
}

#[test]
fn graph_traversal_and_paths() {
    let g = triangle();
    let mut bfs = g.bfs(&"a");
    bfs.sort();
    assert_eq!(bfs, ["a", "b", "c"]);
    assert_eq!(g.dfs(&"a").first(), Some(&"a"));
    assert_eq!(g.dfs(&"a").len(), 3);
    assert_eq!(g.shortest_path(&"a", &"c"), Some(vec!["a", "c"]));
    assert_eq!(g.shortest_path(&"c", &"a"), None);
    assert!(g.bfs(&"zzz").is_empty());
}

#[test]
fn graph_cycles_and_components() {
    let g = triangle();
    assert!(!g.has_cycle());
    let mut cyc = Graph::directed();
    cyc.add_edge("a", "b", 1);
    cyc.add_edge("b", "a", 1);
    assert!(cyc.has_cycle());
    let mut u = Graph::<&str, i32>::undirected();
    u.add_edge("a", "b", 1);
    u.add_node("z");
    assert_eq!(u.components().len(), 2);
    let mut n = u.neighbors(&"a");
    n.sort();
    assert_eq!(n, ["b"]);
}
