//! Behavior ports for `JsonPointer`, `JsonPatch`, `Formatter` (JSON core),
//! `Utils` (SingleShotGen), and `Graph` (persistent graphs + algorithms).
//!
//! Modules load via `#[path]` includes (the .15 precedent): no facade
//! touch while `lib.rs` is peer-held. Each included file also compiles
//! under future `mod` wiring (`super::` paths match both layouts).

#[path = "../src/json.rs"]
mod json;
#[path = "../src/json_pointer.rs"]
mod json_pointer;
#[path = "../src/json_patch.rs"]
mod json_patch;
#[path = "../src/formatter.rs"]
mod formatter;
#[path = "../src/utils_gen.rs"]
mod utils_gen;
#[path = "../src/graph.rs"]
mod graph;

use json::Json;
use std::collections::BTreeMap;

fn obj(pairs: &[(&str, Json)]) -> Json {
    Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

fn arr(items: &[Json]) -> Json {
    Json::Arr(items.to_vec())
}

// ---------------------------------------------------------------------------
// JsonPointer: escape / unescape
// ---------------------------------------------------------------------------

#[test]
fn escape_tilde_to_0() {
    assert_eq!(json_pointer::escape_token("c~d"), "c~0d");
}

#[test]
fn escape_slash_to_1() {
    assert_eq!(json_pointer::escape_token("a/b"), "a~1b");
}

#[test]
fn escape_tilde_before_slash_prevents_double_escape() {
    assert_eq!(json_pointer::escape_token("path/to~key"), "path~1to~0key");
    assert_eq!(json_pointer::escape_token("~/"), "~0~1");
    assert_eq!(json_pointer::escape_token("/~"), "~1~0");
}

#[test]
fn escape_empty_and_plain_unchanged() {
    assert_eq!(json_pointer::escape_token(""), "");
    assert_eq!(json_pointer::escape_token("abc"), "abc");
    assert_eq!(json_pointer::escape_token("~~~"), "~0~0~0");
    assert_eq!(json_pointer::escape_token("///"), "~1~1~1");
    assert_eq!(json_pointer::escape_token("caf\u{00e9}/\u{2603}"), "caf\u{00e9}~1\u{2603}");
}

#[test]
fn unescape_0_to_tilde() {
    assert_eq!(json_pointer::unescape_token("c~0d"), "c~d");
}

#[test]
fn unescape_1_to_slash() {
    assert_eq!(json_pointer::unescape_token("a~1b"), "a/b");
}

#[test]
fn unescape_1_before_0_prevents_misdecode() {
    assert_eq!(json_pointer::unescape_token("path~1to~0key"), "path/to~key");
    assert_eq!(json_pointer::unescape_token("~01"), "~1");
    assert_eq!(json_pointer::unescape_token("~10"), "/0");
    assert_eq!(json_pointer::unescape_token("~00"), "~0");
    assert_eq!(json_pointer::unescape_token("~11"), "/1");
    assert_eq!(json_pointer::unescape_token(""), "");
    assert_eq!(json_pointer::unescape_token("abc"), "abc");
}

#[test]
fn escape_unescape_roundtrip() {
    for token in ["", "a", "a/b", "c~d", "path/to~key", "~01", "~10", "caf\u{00e9}", "a~1b"] {
        assert_eq!(json_pointer::unescape_token(&json_pointer::escape_token(token)), token, "{token}");
    }
    for _ in 0..3 {
        let t = "~0~1/";
        assert_eq!(json_pointer::unescape_token(&json_pointer::escape_token(t)), t);
    }
}

// ---------------------------------------------------------------------------
// JsonPointer: parse / format URI fragments
// ---------------------------------------------------------------------------

#[test]
fn parse_document_root() {
    assert_eq!(json_pointer::parse(""), Ok(vec![]));
}

#[test]
fn parse_decodes_percent_before_splitting() {
    assert_eq!(
        json_pointer::parse_uri_fragment("#/caf%C3%A9"),
        Some(vec!["caf\u{00e9}".to_string()])
    );
    assert_eq!(
        json_pointer::parse_uri_fragment("#/users/a~1b"),
        Some(vec!["users".to_string(), "a/b".to_string()])
    );
}

#[test]
fn encoded_separator_differs_from_escaped_slash_only_in_route() {
    // Both routes land on a key containing `/`; the encoded one splits first.
    assert_eq!(
        json_pointer::parse_uri_fragment("#/a%2Fb"),
        Some(vec!["a".to_string(), "b".to_string()])
    );
    assert_eq!(json_pointer::parse_uri_fragment("#/a~1b"), Some(vec!["a/b".to_string()]));
}

#[test]
fn reject_invalid_fragments() {
    assert_eq!(json_pointer::parse_uri_fragment("#/%"), None);
    assert_eq!(json_pointer::parse_uri_fragment("#/a#b"), None);
    assert_eq!(json_pointer::parse_uri_fragment("/a"), None);
    assert_eq!(json_pointer::parse_uri_fragment(""), None);
}

#[test]
fn format_document_root() {
    assert_eq!(json_pointer::format(&[]), "");
    assert_eq!(json_pointer::format_uri_fragment(&[]), "#");
}

#[test]
fn format_escapes_before_percent_encoding() {
    assert_eq!(
        json_pointer::format_uri_fragment(&["users".to_string(), "a/b".to_string(), "Rate%".to_string()]),
        "#/users/a~1b/Rate%25"
    );
}

#[test]
fn uri_fragment_roundtrip() {
    for path in [
        vec![],
        vec!["a".to_string()],
        vec!["a/b".to_string(), "c~d".to_string(), "caf\u{00e9}".to_string()],
    ] {
        assert_eq!(json_pointer::parse_uri_fragment(&json_pointer::format_uri_fragment(&path)), Some(path));
    }
}

#[test]
fn reject_non_empty_pointers_missing_slash() {
    assert!(json_pointer::parse("a").is_err());
    assert!(json_pointer::parse("a/b").is_err());
}

#[test]
fn pointer_resolves_objects_and_arrays() {
    let doc = obj(&[("a", arr(&[Json::Num(1.0), Json::Num(2.0)]))]);
    assert_eq!(json_pointer::resolve(&doc, &[]), Ok(&doc));
    assert_eq!(
        json_pointer::resolve(&doc, &["a".to_string(), "1".to_string()]),
        Ok(&Json::Num(2.0))
    );
    assert!(json_pointer::resolve(&doc, &["b".to_string()]).is_err());
    assert!(json_pointer::resolve(&doc, &["a".to_string(), "5".to_string()]).is_err());
}

// ---------------------------------------------------------------------------
// JsonPatch: diff
// ---------------------------------------------------------------------------

fn jdoc() -> (Json, Json) {
    let old = obj(&[
        ("users", arr(&[obj(&[("id", Json::Num(1.0)), ("name", Json::Str("Alice".into()))])])),
        ("count", Json::Num(1.0)),
    ]);
    let new = obj(&[
        ("users", arr(&[
            obj(&[("id", Json::Num(1.0)), ("name", Json::Str("Bob".into()))]),
            obj(&[("id", Json::Num(2.0)), ("name", Json::Str("Charlie".into()))]),
        ])),
        ("count", Json::Num(2.0)),
    ]);
    (old, new)
}

#[test]
fn diff_empty_for_identical_values() {
    let doc = jdoc().0;
    assert_eq!(json_patch::get(&doc, &doc), vec![]);
    assert_eq!(json_patch::get(&Json::Num(1.0), &Json::Num(1.0)), vec![]);
    assert_eq!(json_patch::get(&arr(&[]), &arr(&[])), vec![]);
    assert_eq!(json_patch::get(&obj(&[]), &obj(&[])), vec![]);
}

#[test]
fn diff_root_replace_for_primitive_and_type_changes() {
    assert_eq!(
        json_patch::get(&Json::Num(1.0), &Json::Num(2.0)),
        vec![json_patch::Operation::Replace { path: "".into(), value: Json::Num(2.0) }]
    );
    assert_eq!(
        json_patch::get(&Json::Num(1.0), &Json::Str("x".into())),
        vec![json_patch::Operation::Replace { path: "".into(), value: Json::Str("x".into()) }]
    );
}

#[test]
fn diff_computes_object_example() {
    let (old, new) = jdoc();
    let patch = json_patch::get(&old, &new);
    assert_eq!(patch.len(), 3);
    assert_eq!(patch[0], json_patch::Operation::Replace { path: "/count".into(), value: Json::Num(2.0) });
    assert_eq!(
        patch[1],
        json_patch::Operation::Replace { path: "/users/0/name".into(), value: Json::Str("Bob".into()) }
    );
    assert_eq!(json_patch::apply(&patch, &old), Ok(new));
}

#[test]
fn diff_array_append_and_descending_removal() {
    let old = arr(&[Json::Num(1.0), Json::Num(2.0), Json::Num(3.0)]);
    let new = arr(&[Json::Num(1.0)]);
    assert_eq!(
        json_patch::get(&old, &new),
        vec![
            json_patch::Operation::Remove { path: "/2".into() },
            json_patch::Operation::Remove { path: "/1".into() },
        ]
    );
    let grown = arr(&[Json::Num(1.0), Json::Num(9.0), Json::Num(3.0), Json::Num(4.0)]);
    assert_eq!(
        json_patch::get(&old, &grown),
        vec![
            json_patch::Operation::Replace { path: "/1".into(), value: Json::Num(9.0) },
            json_patch::Operation::Add { path: "/3".into(), value: Json::Num(4.0) },
        ]
    );
}

#[test]
fn diff_object_add_remove_replace_sorted() {
    let old = obj(&[("b", Json::Num(1.0)), ("a", Json::Num(1.0)), ("gone", Json::Num(0.0))]);
    let new = obj(&[("b", Json::Num(2.0)), ("a", Json::Num(1.0)), ("fresh", Json::Num(9.0))]);
    let patch = json_patch::get(&old, &new);
    assert_eq!(
        patch,
        vec![
            json_patch::Operation::Replace { path: "/b".into(), value: Json::Num(2.0) },
            json_patch::Operation::Remove { path: "/gone".into() },
            json_patch::Operation::Add { path: "/fresh".into(), value: Json::Num(9.0) },
        ]
    );
    assert_eq!(json_patch::apply(&patch, &old), Ok(new));
}

#[test]
fn diff_escapes_keys_like_pointers() {
    let old = obj(&[("a/b", Json::Num(1.0))]);
    let new = obj(&[("a/b", Json::Num(2.0))]);
    assert_eq!(
        json_patch::get(&old, &new),
        vec![json_patch::Operation::Replace { path: "/a~1b".into(), value: Json::Num(2.0) }]
    );
    let tilde = obj(&[("~1", Json::Num(1.0))]);
    let tilde2 = obj(&[("~1", Json::Num(2.0))]);
    assert_eq!(
        json_patch::get(&tilde, &tilde2),
        vec![json_patch::Operation::Replace { path: "/~01".into(), value: Json::Num(2.0) }]
    );
}

#[test]
fn diff_does_not_mutate_inputs() {
    let (old, new) = jdoc();
    let frozen_old = old.clone();
    let frozen_new = new.clone();
    let _ = json_patch::get(&old, &new);
    assert_eq!((old, new), (frozen_old, frozen_new));
}

#[test]
fn diff_deep_roundtrip() {
    let old = obj(&[("x", arr(&[obj(&[("y", Json::Num(1.0))])]))]);
    let new = obj(&[("x", arr(&[obj(&[("y", Json::Num(2.0))]), Json::Bool(true)]))]);
    let patch = json_patch::get(&old, &new);
    assert_eq!(json_patch::apply(&patch, &old), Ok(new));
    assert!(!patch.is_empty());
}

// ---------------------------------------------------------------------------
// JsonPatch: apply
// ---------------------------------------------------------------------------

#[test]
fn apply_replace_add_remove() {
    let doc = obj(&[("a", Json::Num(1.0))]);
    assert_eq!(
        json_patch::apply(&[json_patch::Operation::Replace { path: "/a".into(), value: Json::Num(2.0) }], &doc),
        Ok(obj(&[("a", Json::Num(2.0))]))
    );
    assert_eq!(
        json_patch::apply(&[json_patch::Operation::Add { path: "/b".into(), value: Json::Num(3.0) }], &doc),
        Ok(obj(&[("a", Json::Num(1.0)), ("b", Json::Num(3.0))]))
    );
    assert_eq!(
        json_patch::apply(&[json_patch::Operation::Remove { path: "/a".into() }], &doc),
        Ok(obj(&[]))
    );
}

#[test]
fn apply_sequence_and_chained_paths() {
    let doc = obj(&[]);
    let patch = vec![
        json_patch::Operation::Add { path: "/a".into(), value: obj(&[]) },
        json_patch::Operation::Add { path: "/a/b".into(), value: Json::Num(1.0) },
        json_patch::Operation::Replace { path: "/a/b".into(), value: Json::Num(2.0) },
        json_patch::Operation::Add { path: "/a/b".into(), value: Json::Num(0.0) },
    ];
    assert_eq!(
        json_patch::apply(&patch, &doc),
        Ok(obj(&[("a", obj(&[("b", Json::Num(0.0))]))]))
    );
}

#[test]
fn apply_array_append_dash_and_index() {
    let doc = arr(&[Json::Num(1.0)]);
    assert_eq!(
        json_patch::apply(&[json_patch::Operation::Add { path: "/-".into(), value: Json::Num(2.0) }], &doc),
        Ok(arr(&[Json::Num(1.0), Json::Num(2.0)]))
    );
    assert_eq!(
        json_patch::apply(&[json_patch::Operation::Add { path: "/0".into(), value: Json::Num(0.0) }], &doc),
        Ok(arr(&[Json::Num(0.0), Json::Num(1.0)]))
    );
}

#[test]
fn apply_rejects_bad_paths_and_targets() {
    let doc = obj(&[("a", arr(&[Json::Num(1.0)]))]);
    assert!(json_patch::apply(&[json_patch::Operation::Replace { path: "a".into(), value: Json::Null }], &doc).is_err());
    assert!(json_patch::apply(&[json_patch::Operation::Replace { path: "/a/x".into(), value: Json::Null }], &doc).is_err());
    assert!(json_patch::apply(&[json_patch::Operation::Replace { path: "/a/5".into(), value: Json::Null }], &doc).is_err());
    assert!(json_patch::apply(&[json_patch::Operation::Replace { path: "/a/-".into(), value: Json::Null }], &doc).is_err());
    assert!(json_patch::apply(&[json_patch::Operation::Remove { path: "".into() }], &doc).is_err());
    assert!(json_patch::apply(&[json_patch::Operation::Add { path: "/missing/deep".into(), value: Json::Null }], &doc).is_err());
    assert!(json_patch::apply(&[json_patch::Operation::Replace { path: "/nope".into(), value: Json::Null }], &doc).is_err());
}

#[test]
fn apply_root_add_and_replace_chains() {
    let doc = obj(&[("a", Json::Num(1.0))]);
    assert_eq!(
        json_patch::apply(&[json_patch::Operation::Replace { path: "".into(), value: arr(&[]) }], &doc),
        Ok(arr(&[]))
    );
    assert_eq!(
        json_patch::apply(
            &[
                json_patch::Operation::Replace { path: "".into(), value: obj(&[]) },
                json_patch::Operation::Add { path: "/n".into(), value: Json::Num(1.0) },
            ],
            &doc
        ),
        Ok(obj(&[("n", Json::Num(1.0))]))
    );
}

#[test]
fn apply_empty_patch_clones_input() {
    let doc = jdoc().0;
    assert_eq!(json_patch::apply(&[], &doc), Ok(doc.clone()));
}

#[test]
fn apply_decodes_pointer_escapes() {
    let doc = obj(&[("a/b", Json::Num(1.0)), ("~1", Json::Num(2.0))]);
    assert_eq!(
        json_patch::apply(&[json_patch::Operation::Replace { path: "/a~1b".into(), value: Json::Num(9.0) }], &doc),
        Ok(obj(&[("a/b", Json::Num(9.0)), ("~1", Json::Num(2.0))]))
    );
    assert_eq!(
        json_patch::apply(&[json_patch::Operation::Replace { path: "/~01".into(), value: Json::Num(8.0) }], &doc),
        Ok(obj(&[("a/b", Json::Num(1.0)), ("~1", Json::Num(8.0))]))
    );
}

#[test]
fn apply_roundtrip_get() {
    let olds = vec![
        obj(&[("a", Json::Num(1.0)), ("b", arr(&[Json::Bool(true), Json::Null]))]),
        arr(&[obj(&[("x/y", Json::Str("s".into()))])]),
        Json::Str("root".into()),
    ];
    let news = vec![
        obj(&[("a", Json::Num(2.0)), ("c", Json::Num(0.0))]),
        arr(&[obj(&[("x/y", Json::Str("t".into()))]), Json::Num(9.0)]),
        Json::Num(42.0),
    ];
    for (old, new) in olds.into_iter().zip(news) {
        assert_eq!(json_patch::apply(&json_patch::get(&old, &new), &old), Ok(new));
    }
}

// ---------------------------------------------------------------------------
// Formatter + Utils
// ---------------------------------------------------------------------------

#[test]
fn format_primitives_and_nesting() {
    assert_eq!(formatter::format(&Json::Null, 0), "null");
    assert_eq!(formatter::format(&Json::Bool(true), 0), "true");
    assert_eq!(formatter::format(&Json::Num(123.0), 0), "123");
    assert_eq!(formatter::format(&Json::Str("hi".into()), 0), "\"hi\"");
    assert_eq!(formatter::format(&obj(&[("a", Json::Num(1.0)), ("b", arr(&[Json::Num(2.0), Json::Num(3.0)]))]), 0), "{\"a\":1,\"b\":[2,3]}");
}

#[test]
fn format_pretty_indentation() {
    assert_eq!(
        formatter::format(&obj(&[("a", Json::Num(1.0)), ("b", arr(&[Json::Num(2.0), Json::Num(3.0)]))]), 2),
        "{\n  \"a\": 1,\n  \"b\": [\n    2,\n    3\n  ]\n}"
    );
}

#[test]
fn format_json_valid_output() {
    assert_eq!(
        formatter::format_json(&obj(&[("name", Json::Str("Alice".into())), ("age", Json::Num(30.0))]), 0),
        "{\"age\":30,\"name\":\"Alice\"}"
    );
    assert_eq!(
        formatter::format_json(&obj(&[("name", Json::Str("Alice".into())), ("age", Json::Num(30.0))]), 2),
        "{\n  \"age\": 30,\n  \"name\": \"Alice\"\n}"
    );
}

#[test]
fn singleshot_yields_once_then_completes() {
    let mut generator = utils_gen::SingleShotGen::new("hello");
    assert_eq!(generator.next(0), utils_gen::Step::Yield(&"hello"));
    assert_eq!(generator.next(42), utils_gen::Step::Return(42));
    assert_eq!(generator.next(7), utils_gen::Step::Return(7));
}

// ---------------------------------------------------------------------------
// Graph: construction, protocols, snapshots
// ---------------------------------------------------------------------------

use graph::{
    Graph, GraphError, Kind, MutableGraph, Snapshot, View, begin_mutation, build,
    maximum_bipartite_matching, maximum_flow, minimum_cut, mutate,
};

fn directed(nodes: &[&str], edges: &[(usize, usize, i32)]) -> Graph<String, i32> {
    build(Kind::Directed, |m| {
        for n in nodes {
            m.add_node(n.to_string())?;
        }
        for (s, t, d) in edges {
            m.add_edge(*s, *t, *d)?;
        }
        Ok(())
    })
    .unwrap()
}

fn undirected_i32(nodes: &[&str], edges: &[(usize, usize, i32)]) -> Graph<String, i32> {
    build(Kind::Undirected, |m| {
        for n in nodes {
            m.add_node(n.to_string())?;
        }
        for (s, t, d) in edges {
            m.add_edge(*s, *t, *d)?;
        }
        Ok(())
    })
    .unwrap()
}

fn undirected(nodes: &[&str], edges: &[(usize, usize, &'static str)]) -> Graph<String, &'static str> {
    build(Kind::Undirected, |m| {
        for n in nodes {
            m.add_node(n.to_string())?;
        }
        for (s, t, d) in edges {
            m.add_edge(*s, *t, *d)?;
        }
        Ok(())
    })
    .unwrap()
}

#[test]
fn constructs_directed_and_undirected_graphs() {
    let d = Graph::<String, i32>::directed();
    let u = Graph::<String, i32>::undirected();
    assert_eq!((d.kind, u.kind), (Kind::Directed, Kind::Undirected));
    assert!(!d.is_mutable());
    assert_eq!(d.node_count(), 0);
    assert_eq!(u.edge_count(), 0);
    assert_eq!(Graph::<String, i32>::make("directed").unwrap().kind, Kind::Directed);
}

#[test]
fn rejects_invalid_runtime_graph_kinds() {
    assert_eq!(
        Graph::<String, i32>::make("invalid"),
        Err(GraphError("Graph type must be directed or undirected".into()))
    );
}

#[test]
fn stringifies_iterates_in_graph_order() {
    let g = directed(&["A", "B"], &[(0, 1, 1)]);
    assert_eq!(g.to_string(), "Graph(directed, 2, 1)");
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.node_vec(), vec![(0, &"A".to_string()), (1, &"B".to_string())]);
}

#[test]
fn snapshot_roundtrips_sparse_parallel_loops() {
    let g = Graph::from_snapshot(Snapshot {
        kind: Kind::Undirected,
        nodes: vec![
            graph::IndexedNode { index: 2, data: "A".to_string() },
            graph::IndexedNode { index: 5, data: "B".to_string() },
        ],
        edges: vec![
            graph::IndexedEdge { index: 3, source: 5, target: 2, data: "first" },
            graph::IndexedEdge { index: 7, source: 5, target: 2, data: "parallel" },
            graph::IndexedEdge { index: 11, source: 5, target: 5, data: "loop" },
        ],
    })
    .unwrap();
    let snap = g.to_snapshot();
    assert_eq!(snap.nodes.iter().map(|n| n.index).collect::<Vec<_>>(), vec![2, 5]);
    assert_eq!(snap.edges.iter().map(|e| e.index).collect::<Vec<_>>(), vec![3, 7, 11]);
    assert_eq!(graph::neighbors(&g, 2), vec![5]);
    assert_eq!(graph::neighbors(&g, 5), vec![2, 5]);
    assert_eq!(Graph::from_snapshot(g.to_snapshot()).unwrap(), g);
}

#[test]
fn snapshot_clones_never_alias_stored_records() {
    let g = directed(&["A", "B"], &[(0, 1, 1)]);
    let m = begin_mutation(&g);
    m.update_node(0, |_| "updated".to_string()).unwrap();
    let mut snap = m.to_snapshot();
    snap.nodes[0].index = 1;
    snap.nodes[0].data = "exposed".to_string();
    assert_eq!(m.get_node(0), Some("updated".to_string()));
    assert_eq!(graph::neighbors(&m, 0), vec![1]);
}

#[test]
fn mutation_isolates_source_and_finalizes() {
    let source = directed(&["A", "B"], &[(0, 1, 1)]);
    let m = begin_mutation(&source);
    m.add_node("C".to_string()).unwrap();
    let result = m.end_mutation().unwrap();
    assert_eq!(source.node_count(), 2);
    assert_eq!(result.node_count(), 3);
    assert_ne!(result, source);
    assert!(!result.is_mutable());
}

#[test]
fn scoped_mutation_data_orders_share_behavior() {
    let graph = Graph::<String, i32>::directed();
    let first = mutate(&graph, |m| {
        m.add_node("A".to_string())?;
        Ok(())
    })
    .unwrap();
    assert_eq!(first.node_count(), 1);
}

#[test]
fn finalized_handles_reject_with_mutable_error() {
    let m = begin_mutation(&directed(&["A"], &[]));
    m.add_node("B".to_string()).unwrap();
    m.end_mutation().unwrap();
    assert_eq!(
        m.add_node("late".to_string()),
        Err(GraphError("Graph is not mutable".into()))
    );
    assert_eq!(m.end_mutation(), Err(GraphError("Graph is not mutable".into())));
}

#[test]
fn manual_finalization_inside_scope_errors() {
    let out = build(Kind::Directed, |m: &MutableGraph<String, i32>| {
        m.end_mutation()?;
        Ok(())
    });
    assert_eq!(out, Err(GraphError("Graph is not mutable".into())));
}

#[test]
fn every_mutation_entry_rejects_finalized_handles() {
    let m = begin_mutation(&directed(&["A", "B"], &[(0, 1, 1)]));
    m.end_mutation().unwrap();
    assert!(m.add_node("C".to_string()).is_err());
    assert!(m.add_edge(1, 0, 2).is_err());
    assert!(m.update_node(0, |n| n.clone()).is_err());
    assert!(m.update_edge(0, |e| *e).is_err());
    assert!(m.remove_node(0).is_err());
    assert!(m.remove_nodes(&[0]).is_err());
    assert!(m.remove_edge(0).is_err());
    assert!(m.remove_edges(&[0]).is_err());
    assert!(m.map_nodes(|n| n.clone()).is_err());
    assert!(m.map_edges(|e| *e).is_err());
    assert!(m.reverse().is_err());
    assert!(m.end_mutation().is_err());
}

#[test]
fn nested_mutation_inside_transform_rejected() {
    let m = begin_mutation(&directed(&["A", "B"], &[(0, 1, 1)]));
    let mut inner: Option<Result<usize, GraphError>> = None;
    let out = m.update_node(0, |_| {
        inner = Some(m.add_node("C".to_string()));
        "A".to_string()
    });
    assert!(out.is_ok());
    assert_eq!(inner, Some(Err(GraphError("Cannot mutate graph during a transformation".into()))));
    assert_eq!(m.node_count(), 2);
}

#[test]
fn callbacks_observe_fresh_state() {
    let m = begin_mutation(&directed(&["source", "target"], &[(0, 1, 1)]));
    m.update_edge(0, |e| {
        assert_eq!(graph::has_path(&m, 0, 1, graph::Direction::Outgoing), true);
        e + 1
    })
    .unwrap();
    assert_eq!(m.get_edge(0).unwrap().data, 2);
}

#[test]
fn missing_edge_endpoints_error() {
    let m = begin_mutation(&directed(&["A"], &[]));
    assert_eq!(m.add_edge(1, 0, 1), Err(GraphError("Node 1 does not exist".into())));
    assert_eq!(m.add_edge(0, 1, 1), Err(GraphError("Node 1 does not exist".into())));
}

#[test]
fn silent_missing_updates_and_removals() {
    let m = begin_mutation(&directed(&["A", "B"], &[(0, 1, 1)]));
    m.update_edge(99, |_| 3).unwrap();
    m.remove_edge(99).unwrap();
    m.remove_node(99).unwrap();
    assert_eq!(m.edge_count(), 1);
    assert_eq!(m.get_edge(99), None);
    assert_eq!(m.get_node(99), None);
}

#[test]
fn parallel_undirected_edges_and_loops_remove_independently() {
    let g = undirected(&["A", "B"], &[(1, 0, "first"), (0, 1, "second"), (1, 1, "loop")]);
    let m = begin_mutation(&g);
    m.remove_edge(0).unwrap();
    m.remove_edge(2).unwrap();
    m.remove_edge(99).unwrap();
    assert_eq!(m.to_snapshot().edges.iter().map(|e| e.index).collect::<Vec<_>>(), vec![1]);
    assert_eq!(graph::neighbors(&m, 0), vec![1]);
    assert_eq!(graph::neighbors(&m, 1), vec![0]);
}

#[test]
fn node_removal_drops_incident_edges() {
    let g = undirected(&["A", "B", "C"], &[(1, 0, "reverse"), (0, 1, "parallel"), (1, 1, "loop"), (2, 0, "keep")]);
    let m = begin_mutation(&g);
    m.remove_node(1).unwrap();
    let snap = m.to_snapshot();
    assert_eq!(snap.nodes.iter().map(|n| n.index).collect::<Vec<_>>(), vec![0, 2]);
    assert_eq!(snap.edges.len(), 1);
    assert_eq!(snap.edges[0].data, "keep");
}

#[test]
fn bulk_removal_collects_before_draining() {
    let m = begin_mutation(&directed(&["A", "B"], &[(0, 1, 1), (1, 0, 2)]));
    let ids: Vec<usize> = m.to_snapshot().edges.iter().map(|e| e.index).collect();
    m.remove_edges(&ids).unwrap();
    assert_eq!(m.edge_count(), 0);
    let n = begin_mutation(&directed(&["A", "B"], &[]));
    let nids: Vec<usize> = n.to_snapshot().nodes.iter().map(|x| x.index).collect();
    n.remove_nodes(&nids).unwrap();
    assert_eq!(n.node_count(), 0);
}

#[test]
fn bulk_removal_iterables_query_fresh_graph() {
    let m = begin_mutation(&directed(&["A", "B"], &[(0, 1, 1)]));
    assert!(graph::has_path(&m, 0, 1, graph::Direction::Outgoing));
    m.remove_edges(&[0]).unwrap();
    assert!(!graph::has_path(&m, 0, 1, graph::Direction::Outgoing));
    assert!(graph::is_acyclic(&m));
}

#[test]
fn finalization_inside_bulk_drain_rejected() {
    let m = begin_mutation(&directed(&["A", "B"], &[(0, 1, 1)]));
    let out = m.remove_edges(&[0]);
    assert!(out.is_ok());
    assert!(m.is_live());
    assert_eq!(m.edge_count(), 0);
}

#[test]
fn map_preserves_indexes_and_structure() {
    let m = begin_mutation(&directed(&["a", "b"], &[(0, 1, 2)]));
    m.map_nodes(|n| n.to_uppercase()).unwrap();
    m.map_edges(|e| e * 3).unwrap();
    let snap = m.to_snapshot();
    assert_eq!(snap.nodes[0].data, "A");
    assert_eq!(snap.edges[0].data, 6);
}

#[test]
fn filter_nodes_drops_incident_edges() {
    let m = begin_mutation(&directed(&["A", "B", "C"], &[(0, 1, 1), (1, 2, 2), (0, 2, 3)]));
    m.filter_nodes(|n| n != "B").unwrap();
    let snap = m.to_snapshot();
    assert_eq!(snap.nodes.iter().map(|n| n.index).collect::<Vec<_>>(), vec![0, 2]);
    assert_eq!(snap.edges.len(), 1);
    assert_eq!(snap.edges[0].index, 2);
}

// ---------------------------------------------------------------------------
// Graph: queries
// ---------------------------------------------------------------------------

fn query_graph() -> Graph<String, &'static str> {
    undirected(&["A", "B", "C"], &[(0, 1, "first"), (0, 1, "parallel"), (2, 0, "incoming"), (0, 0, "loop"), (0, 2, "last")])
}

#[test]
fn edge_order_multiplicity_and_degrees() {
    let g = directed(&["A", "B", "C"], &[(0, 1, 1), (0, 1, 1), (2, 0, 1), (0, 0, 1), (0, 2, 1)]);
    assert_eq!(graph::incident_edges(&g, 0), vec![0, 1, 2, 3, 4]);
    assert_eq!(graph::outgoing_edges(&g, 0).unwrap(), vec![0, 1, 3, 4]);
    assert_eq!(graph::incoming_edges(&g, 0).unwrap(), vec![2, 3]);
    assert_eq!(graph::edges_between(&g, 0, 1), vec![0, 1]);
    assert_eq!(graph::edges_between(&g, 1, 0), Vec::<usize>::new());
    assert_eq!(graph::out_degree(&g, 0).unwrap(), 4);
    assert_eq!(graph::in_degree(&g, 0).unwrap(), 2);
}

#[test]
fn neighbors_dedup_first_edge_order() {
    let g = directed(&["A", "B", "C"], &[(0, 1, 1), (0, 1, 1), (2, 0, 1), (0, 0, 1), (0, 2, 1)]);
    assert_eq!(graph::neighbors(&g, 0), vec![1, 0, 2]);
    assert_eq!(graph::successors(&g, 0).unwrap(), vec![1, 0, 2]);
    assert_eq!(graph::predecessors(&g, 0).unwrap(), vec![2, 0]);
    assert_eq!(graph::neighbors_directed(&g, 0, graph::Direction::Outgoing).unwrap(), vec![1, 0, 2]);
}

#[test]
fn undirected_orientation_selfloops_degree() {
    let g = undirected_i32(&["A", "B"], &[(1, 0, 1), (0, 1, 2), (0, 0, 3)]);
    assert_eq!(graph::neighbors(&g, 0), vec![1, 0]);
    assert_eq!(graph::neighbors(&g, 1), vec![0]);
    assert_eq!(graph::incident_edges(&g, 0), vec![0, 1, 2]);
    assert_eq!(graph::edges_between(&g, 0, 1), vec![0, 1]);
    assert_eq!(graph::degree(&g, 0).unwrap(), 4);
    assert!(graph::has_edge(&g, 0, 1));
    assert!(graph::has_edge(&g, 1, 0));
}

#[test]
fn directed_membership_without_symmetry() {
    let g = directed(&["A", "B", "C"], &[(0, 1, 1)]);
    assert!(graph::has_edge(&g, 0, 1));
    assert!(!graph::has_edge(&g, 1, 0));
    assert!(!graph::has_edge(&g, 0, 2));
    assert!(!graph::has_edge(&g, 99, 0));
}

#[test]
fn finders_pass_stored_endpoints() {
    let g = undirected(&["A", "B", "C"], &[(2, 0, "match"), (0, 1, "match")]);
    let mut calls: Vec<(String, usize, usize)> = vec![];
    let found = graph::find_edge(&g, |d, s, t| {
        calls.push((d.to_string(), s, t));
        d == &"match"
    });
    assert_eq!(found, Some(0));
    assert_eq!(calls[0], ("match".to_string(), 2, 0));
    assert_eq!(graph::find_node(&g, |n| n == "B"), Some(1));
    assert_eq!(graph::find_nodes(&g, |n| n != "C"), vec![0, 1]);
}

#[test]
fn directed_only_queries_reject_undirected() {
    let g = undirected_i32(&["A", "B"], &[(0, 1, 1)]);
    assert!(graph::successors(&g, 0).is_err());
    assert!(graph::predecessors(&g, 0).is_err());
    assert!(graph::outgoing_edges(&g, 0).is_err());
    assert!(graph::incoming_edges(&g, 0).is_err());
    assert!(graph::neighbors_directed(&g, 0, graph::Direction::Outgoing).is_err());
    let d = directed(&["A", "B"], &[(0, 1, 1)]);
    assert!(graph::degree(&d, 0).is_err());
}

// ---------------------------------------------------------------------------
// Graph: set operations with identity projections
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Node {
    id: &'static str,
    label: &'static str,
}

fn left_graph() -> Graph<Node, &'static str> {
    graph::build(
        graph::Kind::Directed,
        |m| {
            for n in [("a", "A1"), ("b", "B1"), ("c", "C1")] {
                m.add_node(Node { id: n.0, label: n.1 })?;
            }
            m.add_edge(0, 1, "left")?;
            m.add_edge(1, 2, "shared")?;
            Ok(())
        },
    )
    .unwrap()
}

fn right_graph() -> Graph<Node, &'static str> {
    graph::build(
        graph::Kind::Directed,
        |m| {
            for n in [("b", "B2"), ("c", "C2"), ("d", "D2")] {
                m.add_node(Node { id: n.0, label: n.1 })?;
            }
            m.add_edge(0, 1, "shared")?;
            m.add_edge(1, 2, "right")?;
            Ok(())
        },
    )
    .unwrap()
}

fn semantic(g: &Graph<Node, &'static str>) -> (Vec<String>, Vec<String>) {
    let nodes: std::collections::HashMap<usize, &Node> = g.node_vec().into_iter().collect();
    let mut ns: Vec<String> = nodes.values().map(|n| format!("{}:{}", n.id, n.label)).collect();
    ns.sort();
    let mut es: Vec<String> = g
        .edge_vec()
        .into_iter()
        .map(|(_, e)| format!("{}->{}:{}", nodes[&e.source].id, nodes[&e.target].id, e.data))
        .collect();
    es.sort();
    (ns, es)
}

#[test]
fn compose_intersect_difference_by_identity() {
    let id = |n: &Node| n.id;
    let (cn, ce) = semantic(&graph::compose(&left_graph(), &right_graph(), id).unwrap());
    assert_eq!(cn, vec!["a:A1", "b:B2", "c:C2", "d:D2"]);
    assert_eq!(ce, vec!["a->b:left", "b->c:shared", "c->d:right"]);
    let (inn, ine) = semantic(&graph::intersection(&left_graph(), &right_graph(), id).unwrap());
    assert_eq!(inn, vec!["b:B1", "c:C1"]);
    assert_eq!(ine, vec!["b->c:shared"]);
    let (dn, de) = semantic(&graph::difference(&left_graph(), &right_graph(), id).unwrap());
    assert_eq!(dn, vec!["a:A1", "b:B1", "c:C1"]);
    assert_eq!(de, vec!["a->b:left"]);
    let (sn, se) = semantic(&graph::symmetric_difference(&left_graph(), &right_graph(), id).unwrap());
    assert_eq!(sn, vec!["a:A1", "b:B2", "c:C2", "d:D2"]);
    assert_eq!(se, vec!["a->b:left", "c->d:right"]);
}

#[test]
fn undirected_set_identity_ignores_orientation() {
    let first = undirected(&["A", "B"], &[(0, 1, "same")]);
    let second = undirected(&["B", "A"], &[(0, 1, "same")]);
    assert_eq!(graph::intersection(&first, &second, |n: &String| n.clone()).unwrap().edge_count(), 1);
    assert_eq!(graph::difference(&first, &second, |n: &String| n.clone()).unwrap().edge_count(), 0);
}

#[test]
fn set_operations_reject_kind_mismatch() {
    let d = directed(&["A"], &[]);
    let u = undirected_i32(&["A"], &[]);
    assert!(graph::compose(&d, &u, |n: &String| n.clone()).is_err());
    assert!(graph::intersection(&d, &u, |n: &String| n.clone()).is_err());
    assert!(graph::difference(&d, &u, |n: &String| n.clone()).is_err());
    assert!(graph::symmetric_difference(&d, &u, |n: &String| n.clone()).is_err());
    assert!(graph::sum(&d, &u).is_err());
}

fn string_graph(nodes: &[&str], edges: &[(usize, usize, &str)]) -> Graph<String, String> {
    build(Kind::Directed, |m| {
        for n in nodes {
            m.add_node(n.to_string())?;
        }
        for (s, t, d) in edges {
            m.add_edge(*s, *t, d.to_string())?;
        }
        Ok(())
    })
    .unwrap()
}

#[test]
fn complement_skips_self_loops() {
    let g = string_graph(&["A", "B"], &[(0, 1, "existing")]);
    let c = graph::complement(&g, |a: &String, b: &String| format!("{a}-{b}"));
    assert_eq!(c.edge_count(), 1);
    let e = c.edge_vec()[0].1.clone();
    assert_eq!((e.source, e.target, e.data.as_str()), (1, 0, "B-A"));
    let u = graph::build(graph::Kind::Undirected, |m: &graph::MutableGraph<String, String>| {
        for n in ["A", "B", "C"] {
            m.add_node(n.to_string())?;
        }
        m.add_edge(0, 1, "existing".to_string())?;
        Ok(())
    })
    .unwrap();
    let cu = graph::complement(&u, |a: &String, b: &String| format!("{a}-{b}"));
    assert_eq!(cu.edge_count(), 2);
}

fn directed_s(nodes: &[&str], edges: &[(usize, usize, &'static str)]) -> Graph<String, &'static str> {
    build(Kind::Directed, |m| {
        for n in nodes {
            m.add_node(n.to_string())?;
        }
        for (s, t, d) in edges {
            m.add_edge(*s, *t, *d)?;
        }
        Ok(())
    })
    .unwrap()
}

#[test]
fn neighborhood_radius_and_direction() {
    let g = directed_s(&["A", "B", "C", "D"], &[(0, 1, "AB"), (1, 2, "BC"), (2, 1, "CB"), (2, 3, "CD")]);
    let n1 = graph::neighborhood(&g, 1, 1.0, graph::Direction::Outgoing).unwrap();
    assert_eq!(n1.node_vec().iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(n1.edge_count(), 2);
    let ninf = graph::neighborhood(&g, 1, f64::INFINITY, graph::Direction::Outgoing).unwrap();
    assert_eq!(ninf.node_count(), 3);
    assert!(graph::neighborhood(&g, 1, f64::NAN, graph::Direction::Outgoing).is_err());
    assert!(graph::neighborhood(&g, 1, -1.0, graph::Direction::Outgoing).is_err());
    assert!(graph::neighborhood(&g, 1, 0.5, graph::Direction::Outgoing).is_err());
    assert!(graph::neighborhood(&g, 9, 1.0, graph::Direction::Outgoing).is_err());
}

#[test]
fn induced_subgraph_keeps_sparse_indexes() {
    let g = Graph::from_snapshot(graph::Snapshot {
        kind: graph::Kind::Directed,
        nodes: vec![
            graph::IndexedNode { index: 2, data: "A".to_string() },
            graph::IndexedNode { index: 5, data: "B".to_string() },
            graph::IndexedNode { index: 9, data: "C".to_string() },
        ],
        edges: vec![
            graph::IndexedEdge { index: 3, source: 2, target: 5, data: "AB" },
            graph::IndexedEdge { index: 7, source: 5, target: 9, data: "BC" },
            graph::IndexedEdge { index: 11, source: 5, target: 5, data: "loop" },
        ],
        })
    .unwrap();
    let sub = graph::induced_subgraph(&g, &[9, 5, 5]).unwrap();
    assert_eq!(sub.node_vec().iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![5, 9]);
    assert_eq!(sub.edge_vec().iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![7, 11]);
    assert!(graph::induced_subgraph(&g, &[2, 4]).is_err());
    let empty = graph::induced_subgraph(&Graph::<String, i32>::undirected(), &[]).unwrap();
    assert_eq!(empty.kind, graph::Kind::Undirected);
}

#[test]
fn sum_keeps_equal_payloads_disjoint() {
    let first = directed(&["A", "B"], &[(0, 1, 1)]);
    let second = directed(&["A", "B"], &[(0, 1, 2)]);
    let s = graph::sum(&first, &second).unwrap();
    assert_eq!(s.node_vec().iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(s.edge_count(), 2);
}

// ---------------------------------------------------------------------------
// Graph: serialization + flow + matching
// ---------------------------------------------------------------------------

#[test]
fn graphviz_exact_strings() {
    let g = directed(&["A", "B"], &[(0, 1, 1)]);
    assert_eq!(
        graph::to_graphviz(&g, "G", |n: &String| n.clone(), |e: &i32| e.to_string()),
        "digraph \"G\" {\n  \"0\" [label=\"A\"];\n  \"1\" [label=\"B\"];\n  \"0\" -> \"1\" [label=\"1\"];\n}"
    );
    let u = undirected(&["A", "B"], &[(1, 0, "edge")]);
    assert_eq!(
        graph::to_graphviz(&u, "G", |n: &String| n.clone(), |e: &&str| e.to_string()),
        "graph \"G\" {\n  \"0\" [label=\"A\"];\n  \"1\" [label=\"B\"];\n  \"1\" -- \"0\" [label=\"edge\"];\n}"
    );
}

#[test]
#[test]
fn graphviz_escapes_names_and_labels() {
    let label = "C:\\new\n\"line\"";
    let g = directed(&[label, "end"], &[(0, 1, 1)]);
    let out = graph::to_graphviz(&g, "My \"Graph\"", |n: &String| format!("node:{n}"), |_: &i32| "e".to_string());
    assert!(out.starts_with("digraph \"My \\\"Graph\\\"\" {"));
    assert!(out.contains("node:C:\\\\new\\n\\\"line\\\""));
}

#[test]
fn mermaid_exact_strings() {
    let g = directed(&["A", "B"], &[(0, 1, 1)]);
    assert_eq!(
        graph::to_mermaid(&g, "TD", true, |n: &String| n.clone(), |_: &i32| "edge".to_string(), |_| graph::MermaidShape::Rectangle),
        "flowchart TD\n  0[\"A\"]\n  1[\"B\"]\n  0 -->|\"edge\"| 1"
    );
    let u = undirected(&["A", "B"], &[(1, 0, "")]);
    assert_eq!(
        graph::to_mermaid(&u, "TD", false, |n: &String| n.clone(), |_: &&str| "".to_string(), |_| graph::MermaidShape::Rectangle),
        "graph TD\n  0[\"A\"]\n  1[\"B\"]\n  1 --- 0"
    );
}

#[test]
fn mermaid_shapes_and_escapes() {
    let g = directed(&["A"], &[]);
    let shapes = [
        (graph::MermaidShape::Rectangle, "0[\"A\"]"),
        (graph::MermaidShape::Rounded, "0(\"A\")"),
        (graph::MermaidShape::Circle, "0((\"A\"))"),
        (graph::MermaidShape::Diamond, "0{\"A\"}"),
        (graph::MermaidShape::Hexagon, "0{{\"A\"}}"),
        (graph::MermaidShape::Stadium, "0([\"A\"])"),
        (graph::MermaidShape::Subroutine, "0[[\"A\"]]"),
        (graph::MermaidShape::Cylindrical, "0[(\"A\")]"),
    ];
    for (shape, node) in shapes {
        assert_eq!(
            graph::to_mermaid(&g, "TD", true, |n: &String| n.clone(), |_: &i32| "".to_string(), |_| shape),
            format!("flowchart TD\n  {node}")
        );
    }
    let tricky = directed(&["#\"<>&[]{}()|\\", "B\r\n2"], &[(0, 1, 1)]);
    let out = graph::to_mermaid(&tricky, "LR", false, |n: &String| n.clone(), |_: &i32| "w(2)".to_string(), |_| graph::MermaidShape::Rectangle);
    assert!(!out.contains('\r'));
    assert!(out.contains("#35;#quot;#lt;#gt;#amp;#91;#93;#123;#125;#40;#41;#124;#92;"));
    assert!(out.contains("w#40;2#41;"));
}

#[test]
fn maximum_flow_and_minimum_cut_exact() {
    let g = directed(&["source", "target"], &[(0, 1, 3)]);
    let flow = graph::maximum_flow(&g, 0, 1, |e: &i32| *e as f64).unwrap();
    assert_eq!(flow.value, 3.0);
    assert_eq!(flow.flows.get(&0), Some(&3.0));
    assert_eq!(flow.cut, vec![0]);
    let cut = graph::minimum_cut(&g, 0, 1, |e: &i32| *e as f64).unwrap();
    assert_eq!(cut.value, 3.0);
    assert_eq!(cut.edges, vec![0]);
    assert_eq!((cut.source, cut.target), (vec![0], vec![1]));
}

#[test]
fn flow_clrs_conservation_and_cut_value() {
    let caps = [16.0, 13.0, 10.0, 4.0, 12.0, 9.0, 14.0, 7.0, 20.0, 4.0];
    let ends = [[0, 1], [0, 2], [1, 2], [2, 1], [1, 3], [3, 2], [2, 4], [4, 3], [3, 5], [4, 5]];
    let g = graph::build(graph::Kind::Directed, |m: &graph::MutableGraph<(), f64>| {
        for _ in 0..6 {
            m.add_node(())?;
        }
        for (i, e) in ends.iter().enumerate() {
            m.add_edge(e[0] as usize, e[1] as usize, caps[i])?;
        }
        Ok(())
    })
    .unwrap();
    let flow = graph::maximum_flow(&g, 0, 5, |e: &f64| *e).unwrap();
    let cut = graph::minimum_cut(&g, 0, 5, |e: &f64| *e).unwrap();
    assert_eq!(flow.value, 23.0);
    assert_eq!(cut.value, 23.0);
    let mut balance = [0.0; 6];
    for (i, e) in ends.iter().enumerate() {
        let v = flow.flows[&i];
        let (s, t) = (e[0] as usize, e[1] as usize);
        assert!((0.0..=caps[i]).contains(&v));
        balance[s] -= v;
        balance[t] += v;
    }
    assert_eq!(balance, [-23.0, 0.0, 0.0, 0.0, 0.0, 23.0]);
    assert_eq!(cut.edges.iter().map(|e| caps[*e]).sum::<f64>(), 23.0);
}

#[test]
fn flow_reroutes_through_residuals() {
    let g = directed(&["a", "b", "c", "d", "e", "f", "g"], &[(0, 1, 1), (0, 2, 1), (1, 3, 1), (2, 3, 1), (3, 5, 1), (1, 4, 1), (4, 5, 1)]);
    assert_eq!(graph::maximum_flow(&g, 0, 5, |e: &i32| *e as f64).unwrap().value, 2.0);
}

#[test]
fn flow_validation_messages() {
    let g = directed(&["a", "b"], &[]);
    assert_eq!(
        graph::maximum_flow(&g, 0, 2, |e: &i32| *e as f64),
        Err(graph::GraphError("Node 2 does not exist".into()))
    );
    assert_eq!(
        graph::maximum_flow(&g, 0, 0, |e: &i32| *e as f64),
        Err(graph::GraphError("Flow source and target must be different nodes".into()))
    );
    let u = undirected(&["a", "b"], &[(0, 1, "x")]);
    assert_eq!(
        graph::maximum_flow(&u, 0, 1, |_: &&str| 1.0),
        Err(graph::GraphError("Cannot compute flow of undirected graph".into()))
    );
    for cap in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let bad = graph::build(graph::Kind::Directed, |m: &graph::MutableGraph<(), f64>| {
            m.add_node(())?;
            m.add_node(())?;
            m.add_edge(0, 0, cap)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            graph::maximum_flow(&bad, 0, 1, |e: &f64| *e),
            Err(graph::GraphError("Edge 0 capacity must be a finite non-negative number".into()))
        );
    }
    let neg = graph::build(graph::Kind::Directed, |m: &graph::MutableGraph<(), f64>| {
        m.add_node(())?;
        m.add_node(())?;
        m.add_edge(0, 0, -1.0)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        graph::maximum_flow(&neg, 0, 1, |e: &f64| *e),
        Err(graph::GraphError("Edge 0 capacity must be a finite non-negative number".into()))
    );
}

#[test]
fn bipartite_even_odd_disconnected_selfloop() {
    let even = undirected(&["a", "b", "c", "d"], &[(0, 1, "x"), (1, 2, "x"), (2, 3, "x"), (3, 0, "x")]);
    assert_eq!(graph::is_bipartite(&even), Ok(true));
    let odd = undirected(&["a", "b", "c"], &[(0, 1, "x"), (1, 2, "x"), (2, 0, "x")]);
    assert_eq!(graph::is_bipartite(&odd), Ok(false));
    let with_loop = undirected(&["a"], &[(0, 0, "x")]);
    assert_eq!(graph::is_bipartite(&with_loop), Ok(false));
    let d = directed(&["a", "b"], &[(0, 1, 1)]);
    assert!(graph::is_bipartite(&d).is_err());
    assert!(graph::maximum_bipartite_matching(&d).is_err());
}

#[test]
fn bipartite_matching_bruteforce_oracle() {
    for mask in 0..512 {
        let mut adjacency: Vec<Vec<usize>> = vec![vec![], vec![], vec![]];
        let g = graph::build(graph::Kind::Undirected, |m: &graph::MutableGraph<(), ()>| {
            for _ in 0..6 {
                m.add_node(())?;
            }
            for left in 0..3 {
                for right in 0..3 {
                    if mask & (1 << (left * 3 + right)) != 0 {
                        adjacency[left].push(right);
                        m.add_edge(left, right + 3, ())?;
                    }
                }
            }
            Ok(())
        })
        .unwrap();
        let matching = graph::maximum_bipartite_matching(&g).unwrap();
        let best = oracle(&adjacency, 0, 0);
        assert_eq!(matching.len(), best, "mask {mask}");
        let mut lefts = std::collections::HashSet::new();
        let mut rights = std::collections::HashSet::new();
        for mt in &matching {
            assert!((0..3).contains(&mt.left) && (3..6).contains(&mt.right));
            assert!(lefts.insert(mt.left) && rights.insert(mt.right));
            let e = g.edge_vec().into_iter().find(|(i, _)| *i == mt.edge).unwrap().1;
            assert!((e.source == mt.left && e.target == mt.right) || (e.source == mt.right && e.target == mt.left));
        }
    }
}

fn oracle(adjacency: &[Vec<usize>], left: usize, used: u32) -> usize {
    if left == adjacency.len() {
        return 0;
    }
    let mut best = oracle(adjacency, left + 1, used);
    for right in &adjacency[left] {
        if used & (1 << right) == 0 {
            best = best.max(1 + oracle(adjacency, left + 1, used | (1 << right)));
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Graph: traversal, topo, walkers
// ---------------------------------------------------------------------------

fn diamond() -> Graph<i32, i32> {
    graph::build(graph::Kind::Directed, |m: &graph::MutableGraph<i32, i32>| {
        for n in [0, 1, 2, 3] {
            m.add_node(n)?;
        }
        m.add_edge(0, 1, 1)?;
        m.add_edge(0, 2, 2)?;
        m.add_edge(1, 3, 3)?;
        m.add_edge(2, 3, 4)?;
        Ok(())
    })
    .unwrap()
}

#[test]
fn traversal_documented_orders() {
    let g = diamond();
    assert_eq!(graph::dfs(&g, &[0], f64::INFINITY, graph::Direction::Outgoing).unwrap().indices().unwrap(), vec![0, 1, 3, 2]);
    assert_eq!(graph::bfs(&g, &[0], f64::INFINITY, graph::Direction::Outgoing).unwrap().indices().unwrap(), vec![0, 1, 2, 3]);
    assert_eq!(
        graph::dfs_post_order(&g, &[0], f64::INFINITY, graph::Direction::Outgoing).unwrap().indices().unwrap(),
        vec![3, 1, 2, 0]
    );
    assert_eq!(graph::dfs(&g, &[], f64::INFINITY, graph::Direction::Outgoing).unwrap().indices().unwrap(), Vec::<usize>::new());
}

#[test]
fn postorder_parallel_selfloops() {
    let g = graph::build(graph::Kind::Directed, |m: &graph::MutableGraph<i32, i32>| {
        for n in [0, 1, 2, 3, 4] {
            m.add_node(n)?;
        }
        m.add_edge(0, 1, 1)?;
        m.add_edge(0, 1, 2)?;
        m.add_edge(0, 1, 3)?;
        m.add_edge(1, 2, 4)?;
        m.add_edge(1, 3, 5)?;
        m.add_edge(1, 4, 6)?;
        m.add_edge(1, 1, 7)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        graph::dfs_post_order(&g, &[0], f64::INFINITY, graph::Direction::Outgoing).unwrap().indices().unwrap(),
        vec![2, 3, 4, 1, 0]
    );
}

#[test]
fn radius_membership_by_shortest_distance() {
    let g = diamond();
    assert_eq!(
        graph::dfs(&g, &[0], 2.0, graph::Direction::Outgoing).unwrap().indices().unwrap(),
        vec![0, 1, 3, 2]
    );
    assert_eq!(
        graph::bfs(&g, &[2], f64::INFINITY, graph::Direction::Incoming).unwrap().indices().unwrap(),
        vec![2, 0]
    );
    assert_eq!(
        graph::dfs_post_order(&g, &[2], f64::INFINITY, graph::Direction::Incoming).unwrap().indices().unwrap(),
        vec![0, 2]
    );
    let chain = graph::build(graph::Kind::Directed, |m: &graph::MutableGraph<i32, i32>| {
        for n in [0, 1, 2] {
            m.add_node(n)?;
        }
        m.add_edge(0, 1, 1)?;
        m.add_edge(1, 2, 1)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        graph::dfs_post_order(&chain, &[0], 1.0, graph::Direction::Outgoing).unwrap().indices().unwrap(),
        vec![1, 0]
    );
    assert_eq!(
        graph::bfs(&g, &[1], 1.0, graph::Direction::Undirected).unwrap().indices().unwrap(),
        vec![1, 3, 0]
    );
    assert_eq!(
        graph::dfs_post_order(&g, &[0], f64::INFINITY, graph::Direction::Undirected)
            .unwrap()
            .indices()
            .unwrap(),
        vec![2, 3, 1, 0]
    );
}

#[test]
fn radius_validation_errors() {
    let g = diamond();
    for bad in [f64::NAN, f64::NEG_INFINITY, -1.0, 0.5] {
        assert!(graph::dfs(&g, &[0], bad, graph::Direction::Outgoing).is_err());
        assert!(graph::bfs(&g, &[0], bad, graph::Direction::Outgoing).is_err());
        assert!(graph::dfs_post_order(&g, &[0], bad, graph::Direction::Outgoing).is_err());
    }
}

#[test]
fn walkers_snapshot_while_fresh_reiterate() {
    let m = begin_mutation(&directed(&["A", "B", "C"], &[(0, 1, 1), (1, 2, 2)]));
    let d = graph::dfs(&m, &[0], f64::INFINITY, graph::Direction::Outgoing).unwrap();
    let b = graph::bfs(&m, &[0], f64::INFINITY, graph::Direction::Outgoing).unwrap();
    let p = graph::dfs_post_order(&m, &[0], f64::INFINITY, graph::Direction::Outgoing).unwrap();
    let frozen_d = d.indices().unwrap();
    let frozen_b = b.indices().unwrap();
    let frozen_p = p.indices().unwrap();
    m.remove_node(1).unwrap();
    assert_eq!((frozen_d, frozen_b, frozen_p), (vec![0, 1, 2], vec![0, 1, 2], vec![2, 1, 0]));
    assert_eq!(d.indices().unwrap(), vec![0]);
    assert!(graph::dfs(&m, &[0], f64::INFINITY, graph::Direction::Outgoing).is_ok());
}

#[test]
fn missing_starts_revalidate_per_run() {
    let m = begin_mutation(&directed(&["A", "B"], &[(0, 1, 1)]));
    let d = graph::dfs(&m, &[0], f64::INFINITY, graph::Direction::Outgoing).unwrap();
    m.remove_node(0).unwrap();
    assert_eq!(d.indices(), Err(graph::GraphError("Node 0 does not exist".into())));
    assert!(graph::dfs(&m, &[0], f64::INFINITY, graph::Direction::Outgoing).is_err());
}

#[test]
fn topo_prioritized_initials_and_validation() {
    let g = directed(&["A", "B", "C", "D"], &[(0, 1, 1), (2, 3, 1)]);
    assert_eq!(graph::topo(&g, &[2]).unwrap().indices().unwrap(), vec![2, 0, 3, 1]);
    let u = Graph::<String, i32>::undirected();
    match graph::topo(&u, &[]).unwrap().indices() {
        Err(e) => assert_eq!(e.to_string(), "Cannot perform topological sort on undirected graph"),
        Ok(_) => panic!("expected topo error"),
    }
    let cyc = directed(&["a", "b"], &[(0, 1, 1), (1, 0, 1)]);
    assert_eq!(
        graph::topo(&cyc, &[]).unwrap().indices().unwrap_err().to_string(),
        "Cannot perform topological sort on cyclic graph"
    );
    let g2 = directed(&["a", "b"], &[(0, 1, 1)]);
    assert_eq!(
        graph::topo(&g2, &[1]).unwrap().indices().unwrap_err().to_string(),
        "Initial node 1 has incoming edges"
    );
    match graph::topo(&g2, &[2]) {
        Err(e) => assert_eq!(e.to_string(), "Node 2 does not exist"),
        Ok(_) => panic!("expected topo error"),
    }
}

#[test]
fn topo_tracks_live_mutation() {
    let m = begin_mutation(&directed(&["A", "B"], &[]));
    let w = graph::topo(&m, &[]).unwrap();
    assert_eq!(w.indices().unwrap(), vec![0, 1]);
    m.add_edge(0, 1, 1).unwrap();
    m.add_edge(1, 0, 2).unwrap();
    assert_eq!(
        w.indices(),
        Err(graph::GraphError("Cannot perform topological sort on cyclic graph".into()))
    );
}

#[test]
fn walker_custom_visit_repeatable() {
    let w = graph::Walker::custom(vec![(2, "A".to_string()), (5, "B".to_string())]);
    assert_eq!(graph::indices(&w).unwrap(), vec![2, 5]);
    assert_eq!(graph::values(&w).unwrap(), vec!["A".to_string(), "B".to_string()]);
    assert_eq!(w.visit(|i, v| format!("{i}:{v}")).unwrap(), vec!["2:A".to_string(), "5:B".to_string()]);
    assert_eq!(w.entries().unwrap(), w.entries().unwrap());
}

#[test]
fn walker_live_over_mutable_nodes_and_edges() {
    let m = begin_mutation(&directed(&["A", "B"], &[(0, 1, 1)]));
    let nodes = graph::nodes(&m);
    let edges = graph::edges(&m);
    assert_eq!(graph::values(&nodes).unwrap(), vec!["A".to_string(), "B".to_string()]);
    assert_eq!(edges.edge_indices().unwrap(), vec![0]);
    m.add_node("C".to_string()).unwrap();
    assert_eq!(graph::values(&nodes).unwrap().len(), 3);
}

// ---------------------------------------------------------------------------
// Graph: paths, cycles, connectivity, spanning, flow already covered;
// pathfinders, witnesses, partitions, predicates
// ---------------------------------------------------------------------------

fn weighted() -> Graph<i32, i32> {
    graph::build(graph::Kind::Directed, |m: &graph::MutableGraph<i32, i32>| {
        for n in [0, 1, 2, 3] {
            m.add_node(n)?;
        }
        m.add_edge(0, 1, 1)?;
        m.add_edge(0, 2, 4)?;
        m.add_edge(1, 2, 2)?;
        m.add_edge(1, 3, 5)?;
        m.add_edge(2, 3, 1)?;
        Ok(())
    })
    .unwrap()
}

#[test]
fn shortest_paths_agree_with_ground_truth() {
    let g = weighted();
    let expect = graph::PathResult { path: vec![0, 1, 2, 3], edges: vec![0, 2, 4], distance: 4.0, costs: vec![1, 2, 1] };
    assert_eq!(graph::dijkstra(&g, 0, 3, |e: &i32| *e as f64).unwrap(), Some(expect.clone()));
    assert_eq!(graph::astar(&g, 0, 3, |e: &i32| *e as f64, |_| 0.0).unwrap(), Some(expect.clone()));
    assert_eq!(graph::bellman_ford(&g, 0, 3, |e: &i32| *e as f64).unwrap(), Some(expect));
    assert_eq!(graph::dijkstra(&g, 0, 0, |e: &i32| *e as f64).unwrap().unwrap().distance, 0.0);
}

#[test]
fn pathfinder_validation_messages() {
    let g = directed(&["a", "b"], &[(0, 1, 1)]);
    assert_eq!(
        graph::dijkstra(&g, 0, 9, |e: &i32| *e as f64),
        Err(graph::GraphError("Node 9 does not exist".into()))
    );
    let neg = directed(&["a", "b"], &[(0, 1, -1)]);
    // i32 edges cannot hold NaN; negative covered here, float cases below.
    assert_eq!(
        graph::dijkstra(&neg, 0, 1, |e: &i32| *e as f64),
        Err(graph::GraphError("Dijkstra's algorithm requires non-negative edge weights".into()))
    );
    assert_eq!(
        graph::astar(&neg, 0, 1, |e: &i32| *e as f64, |_| 0.0),
        Err(graph::GraphError("A* algorithm requires non-negative edge weights".into()))
    );
    let early = directed(&["a", "b", "c"], &[(0, 1, 1), (0, 2, 2), (2, 1, -5)]);
    assert_eq!(
        graph::dijkstra(&early, 0, 1, |e: &i32| *e as f64),
        Err(graph::GraphError("Dijkstra's algorithm requires non-negative edge weights".into()))
    );
}

#[test]
fn bellman_ford_negative_cycle_route() {
    let g = directed(&["a", "b", "c"], &[(0, 1, 1), (1, 2, -3), (2, 0, 1)]);
    assert_eq!(
        graph::bellman_ford(&g, 0, 2, |e: &i32| *e as f64),
        Err(graph::GraphError("Negative cycle affects path to node 2".into()))
    );
}

#[test]
fn all_shortest_and_simple_enumeration() {
    let g = weighted();
    let all = graph::all_shortest_paths(&g, 0, 3, |e: &i32| *e as f64, f64::INFINITY).unwrap();
    let paths = all.paths().unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].nodes, vec![0, 1, 2, 3]);
    let simple = graph::simple_paths(&g, 0, 3, f64::INFINITY).unwrap();
    let edge_lists: Vec<Vec<usize>> = simple.paths().unwrap().into_iter().map(|p| p.edges).collect();
    assert_eq!(edge_lists, vec![vec![0, 2, 4], vec![0, 3], vec![1, 4]]);
    match graph::simple_paths(&g, 0, 3, -1.0) {
        Err(e) => assert_eq!(e.to_string(), "Path enumeration limit must be a non-negative integer or Infinity"),
        Ok(_) => panic!("expected limit error"),
    }
    assert_eq!(graph::simple_paths(&g, 0, 3, 0.0).unwrap().len().unwrap(), 0);
    assert!(graph::simple_paths(&g, 0, 9, f64::INFINITY).is_err());
}

#[test]
fn all_shortest_validates_weights_per_run() {
    let g = directed(&["a", "b"], &[(0, 1, -1)]);
    match graph::all_shortest_paths(&g, 0, 1, |e: &i32| *e as f64, f64::INFINITY) {
        Err(e) => assert_eq!(e.to_string(), "unreachable"),
        Ok(w) => match w.paths() {
            Err(e) => assert_eq!(e.to_string(), "All shortest paths requires non-negative edge weights"),
            Ok(_) => panic!("expected weights error"),
        },
    }
}

#[test]
fn cycle_witnesses_exact() {
    let c = directed(&["a", "b", "c"], &[(0, 1, 1), (1, 2, 1), (2, 0, 1)]);
    let w = graph::find_cycle(&c).unwrap();
    assert_eq!((w.path, w.edges), (vec![0, 1, 2, 0], vec![0, 1, 2]));
    let s = directed(&["a", "b"], &[(0, 0, 1)]);
    let w2 = graph::find_cycle(&s).unwrap();
    assert_eq!((w2.path, w2.edges), (vec![0, 0], vec![0]));
    let p = directed(&["a", "b"], &[(0, 1, 1), (0, 1, 2), (1, 0, 3)]);
    let w3 = graph::find_cycle(&p).unwrap();
    assert_eq!((w3.path, w3.edges), (vec![0, 1, 0], vec![0, 2]));
    assert!(graph::is_acyclic(&directed(&["a", "b"], &[(0, 1, 1)])));
    assert!(!graph::is_acyclic(&c));
}

#[test]
fn undirected_parallel_cycle_witness() {
    let u = undirected(&["a", "b"], &[(0, 1, "x"), (0, 1, "y")]);
    let w = graph::find_cycle(&u).unwrap();
    assert_eq!((w.path, w.edges), (vec![0, 1, 0], vec![0, 1]));
    let l = undirected(&["a"], &[(0, 0, "x")]);
    let w2 = graph::find_cycle(&l).unwrap();
    assert_eq!((w2.path, w2.edges), (vec![0, 0], vec![0]));
}

#[test]
fn component_partitions_exact() {
    let d = directed(&["a", "b", "c", "d"], &[(0, 1, 1), (1, 0, 1), (2, 3, 1)]);
    assert_eq!(graph::strongly_connected_components(&d).unwrap(), vec![vec![2], vec![3], vec![0, 1]]);
    assert_eq!(graph::weakly_connected_components(&d).unwrap(), vec![vec![0, 1], vec![2, 3]]);
    let u = undirected_i32(&["a", "b", "c", "d"], &[(0, 1, 1), (1, 2, 1), (3, 3, 1)]);
    assert_eq!(graph::connected_components(&u).unwrap(), vec![vec![0, 1, 2], vec![3]]);
    assert!(graph::connected_components(&d).is_err());
    assert!(graph::weakly_connected_components(&u).is_err());
}

#[test]
fn predicate_values_including_empty() {
    let tree = undirected_i32(&["A", "B", "C"], &[(0, 1, 1), (1, 2, 1)]);
    assert!(graph::is_connected(&tree));
    assert!(graph::is_tree(&tree).unwrap());
    let dis = undirected(&["A", "B"], &[]);
    assert!(!graph::is_connected(&dis));
    assert!(graph::is_connected(&Graph::<String, i32>::undirected()));
    assert!(!graph::is_tree(&Graph::<String, i32>::undirected()).unwrap());
    let weak = directed(&["A", "B", "C"], &[(0, 1, 1), (1, 2, 1)]);
    assert!(graph::is_weakly_connected(&weak));
    assert!(!graph::is_strongly_connected(&weak));
    let m = begin_mutation(&weak);
    m.add_edge(2, 0, 1).unwrap();
    let strong = m.end_mutation().unwrap();
    assert!(graph::is_strongly_connected(&strong));
    assert!(graph::is_tree(&tree).is_err() == false);
}

#[test]
fn lowlink_partitions_exact() {
    let u = undirected_i32(&["a", "b", "c", "d"], &[(0, 1, 1), (1, 2, 1), (3, 3, 1)]);
    assert_eq!(graph::articulation_points(&u).unwrap(), vec![1]);
    assert_eq!(graph::bridges(&u).unwrap(), vec![0, 1]);
    assert_eq!(graph::biconnected_components(&u).unwrap(), vec![vec![0, 1], vec![1, 2], vec![3]]);
    let d = directed(&["a"], &[]);
    assert!(graph::articulation_points(&d).is_err());
}

#[test]
fn spanning_tiebreak_and_validation() {
    let g = undirected_i32(&["a", "b", "c"], &[(0, 1, 1), (1, 2, 1), (0, 2, 1)]);
    let msf = graph::minimum_spanning_forest(&g, |e: &i32| *e as f64).unwrap();
    assert_eq!(msf.edge_vec().iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1]);
    let d = directed(&["a", "b"], &[(0, 1, 1)]);
    assert!(graph::minimum_spanning_forest(&d, |e: &i32| *e as f64).is_err());
    let nan = graph::build(graph::Kind::Undirected, |m: &graph::MutableGraph<(), f64>| {
        m.add_node(())?;
        m.add_node(())?;
        m.add_edge(0, 1, f64::NAN)?;
        Ok(())
    })
    .unwrap();
    assert!(graph::minimum_spanning_forest(&nan, |e: &f64| *e).is_err());
}

#[test]
fn finders_equality_hash_unweighted_paths() {
    let g = directed(&["A", "B", "C"], &[(0, 1, 1), (0, 1, 1), (2, 0, 1), (0, 0, 1), (0, 2, 1)]);
    assert_eq!(graph::find_node(&g, |n| n == "B"), Some(1));
    assert_eq!(graph::find_nodes(&g, |n| n != "C"), vec![0, 1]);
    assert_eq!(graph::find_edge(&g, |_, _, _| true), Some(0));
    assert_eq!(graph::find_edges(&g, |d, _, _| *d == 1).len(), 5);
    assert_eq!(graph::get_node(&g, 1), Some("B".to_string()));
    assert_eq!(graph::get_node(&g, 99), None);
    assert_eq!(g, directed(&["A", "B", "C"], &[(0, 1, 1), (0, 1, 1), (2, 0, 1), (0, 0, 1), (0, 2, 1)]));
    let mut dist: Vec<(usize, usize)> = graph::unweighted_distances(&g, 0).unwrap().into_iter().collect();
    dist.sort();
    assert_eq!(dist, vec![(0, 0), (1, 1), (2, 1)]);
    assert!(graph::has_path(&g, 0, 2, graph::Direction::Outgoing));
    assert!(graph::has_path(&g, 2, 1, graph::Direction::Outgoing));
    assert!(!graph::has_path(&g, 1, 2, graph::Direction::Outgoing));
    assert!(graph::has_path(&g, 1, 2, graph::Direction::Undirected));
}

#[test]
fn reverse_mutable_edges() {
    let m = begin_mutation(&directed(&["A", "B", "C"], &[(0, 1, 1), (1, 2, 2)]));
    m.reverse().unwrap();
    m.add_edge(0, 1, 3).unwrap();
    let snap = m.to_snapshot();
    assert_eq!(snap.edges.iter().map(|e| (e.source, e.target)).collect::<Vec<_>>(), vec![(1, 0), (2, 1), (0, 1)]);
    assert_eq!(graph::neighbors(&m, 2), vec![1]);
    assert!(graph::has_edge(&m, 1, 0));
    assert!(graph::has_edge(&m, 0, 1));
    let u = begin_mutation(&undirected(&["A", "B"], &[(1, 0, "x")]));
    u.reverse().unwrap();
    assert_eq!(u.to_snapshot().edges[0].source, 1);
}

// ---------------------------------------------------------------------------
// Graph: floyd, spanning, reduction, matching exact, validation, snapshots
// ---------------------------------------------------------------------------

#[test]
fn floyd_multihop_preserves_payloads() {
    let g = graph::build(graph::Kind::Directed, |m: &graph::MutableGraph<String, Option<i32>>| {
        for n in ["A", "B", "C"] {
            m.add_node(n.to_string())?;
        }
        m.add_edge(0, 1, None)?;
        m.add_edge(1, 2, None)?;
        m.add_edge(0, 2, Some(10))?;
        Ok(())
    })
    .unwrap();
    let r = graph::floyd_warshall(&g, |e: &Option<i32>| match e {
        None => 1.0,
        Some(w) => *w as f64,
    })
    .unwrap();
    assert_eq!(r.distances[&0][&2], 2.0);
    assert_eq!(r.paths[&0][&2], vec![0, 1, 2]);
    assert_eq!(r.edges[&0][&2], vec![0, 1]);
    assert_eq!(r.costs[&0][&2], vec![None, None]);
}

#[test]
fn floyd_negative_cycle_names_node() {
    let g = directed(&["a", "b", "c"], &[(0, 1, 1), (1, 2, -3), (2, 0, 1)]);
    assert_eq!(
        graph::floyd_warshall(&g, |e: &i32| *e as f64),
        Err(graph::GraphError("Negative cycle detected involving node 0".into()))
    );
}

#[test]
fn infinity_weights_are_unreachable() {
    let g = weighted();
    assert_eq!(graph::dijkstra(&g, 3, 0, |_: &i32| f64::INFINITY).unwrap(), None);
    assert_eq!(graph::bellman_ford(&g, 3, 0, |_: &i32| f64::INFINITY).unwrap(), None);
}

#[test]
fn bellman_ford_negative_edges_ok() {
    let g = directed(&["a", "b", "c"], &[(0, 1, 1), (1, 2, -3), (0, 2, 10)]);
    let r = graph::bellman_ford(&g, 0, 2, |e: &i32| *e as f64).unwrap().unwrap();
    assert_eq!((r.path, r.distance), (vec![0, 1, 2], -2.0));
}

#[test]
fn spanning_sparse_and_parity() {
    let g = undirected_i32(&["a", "b", "c", "d", "e"], &[(0, 1, 2), (0, 3, 1), (1, 2, 3), (1, 4, 4), (3, 4, 5)]);
    let msf = graph::minimum_spanning_forest(&g, |e: &i32| *e as f64).unwrap();
    assert_eq!(msf.edge_vec().iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(msf.node_vec().iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
}

#[test]
fn transitive_reduction_drops_implied() {
    let g = directed(&["a", "b", "c"], &[(0, 1, 1), (1, 2, 1), (0, 2, 1)]);
    let r = graph::transitive_reduction(&g).unwrap();
    assert_eq!(r.edge_vec().iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1]);
    let cyc = directed(&["a", "b"], &[(0, 1, 1), (1, 0, 1)]);
    assert_eq!(
        graph::transitive_reduction(&cyc),
        Err(graph::GraphError("Cannot transitively reduce cyclic graph".into()))
    );
    let u = undirected_i32(&["a", "b"], &[(0, 1, 1)]);
    assert_eq!(
        graph::transitive_reduction(&u),
        Err(graph::GraphError("Cannot transitively reduce undirected graph".into()))
    );
}

#[test]
fn matching_deterministic_pairs() {
    let g = graph::build(graph::Kind::Undirected, |m: &graph::MutableGraph<String, i32>| {
        for n in ["a", "b", "c", "d", "e", "f"] {
            m.add_node(n.to_string())?;
        }
        m.add_edge(0, 3, 1)?;
        m.add_edge(0, 4, 2)?;
        m.add_edge(1, 3, 3)?;
        m.add_edge(1, 5, 4)?;
        m.add_edge(2, 4, 5)?;
        Ok(())
    })
    .unwrap();
    let mt = graph::maximum_bipartite_matching(&g).unwrap();
    assert_eq!(
        mt.iter().map(|m| (m.left, m.right, m.edge)).collect::<Vec<_>>(),
        vec![(0, 3, 0), (1, 5, 3), (2, 4, 4)]
    );
    let p = undirected(&["a", "b"], &[(0, 1, "x"), (0, 1, "y")]);
    let mp = graph::maximum_bipartite_matching(&p).unwrap();
    assert_eq!(mp.len(), 1);
    assert_eq!(mp[0].edge, 0);
    let odd = undirected(&["a", "b", "c"], &[(0, 1, "x"), (1, 2, "x"), (2, 0, "x")]);
    assert_eq!(
        graph::maximum_bipartite_matching(&odd),
        Err(graph::GraphError("Cannot find bipartite matching of non-bipartite graph".into()))
    );
}

#[test]
fn topo_no_initials_fifo_order() {
    let g = directed(&["A", "B", "C", "D"], &[(0, 1, 1), (2, 3, 1)]);
    assert_eq!(graph::topo(&g, &[]).unwrap().indices().unwrap(), vec![0, 2, 1, 3]);
}

#[test]
fn snapshot_validation_errors() {
    let dup_nodes = graph::Snapshot {
        kind: graph::Kind::Directed,
        nodes: vec![
            graph::IndexedNode { index: 1, data: "A".to_string() },
            graph::IndexedNode { index: 1, data: "B".to_string() },
        ],
        edges: vec![],
    };
    assert_eq!(
        Graph::<String, i32>::from_snapshot(dup_nodes),
        Err(graph::GraphError("Node indexes must be strictly increasing".into()))
    );
    let dup_edges = graph::Snapshot {
        kind: graph::Kind::Directed,
        nodes: vec![graph::IndexedNode { index: 0, data: "A".to_string() }],
        edges: vec![
            graph::IndexedEdge { index: 0, source: 0, target: 0, data: 1 },
            graph::IndexedEdge { index: 0, source: 0, target: 0, data: 2 },
        ],
    };
    assert_eq!(
        Graph::<String, i32>::from_snapshot(dup_edges),
        Err(graph::GraphError("Edge indexes must be strictly increasing".into()))
    );
    assert_eq!(Graph::<String, i32>::make("sideways").unwrap_err().to_string(), "Graph type must be directed or undirected");
}

#[test]
fn equality_and_hash_are_structural() {
    let a = directed(&["A", "B"], &[(0, 1, 1)]);
    let b = directed(&["A", "B"], &[(0, 1, 1)]);
    let c = directed(&["A", "B"], &[(0, 1, 2)]);
    assert_eq!(a, b);
    assert_ne!(a, c);
    let h = |g: &Graph<String, i32>| {
        use std::hash::{Hash, Hasher};
        let mut s = std::collections::hash_map::DefaultHasher::new();
        g.hash(&mut s);
        s.finish()
    };
    assert_eq!(h(&a), h(&b));
    assert_ne!(h(&a), h(&c));
}

#[test]
fn formatter_tab_indent() {
    let doc = obj(&[("a", arr(&[Json::Num(1.0)]))]);
    assert_eq!(formatter::format(&doc, 1), "{\n \"a\": [\n  1\n ]\n}".replace("\n", "
"));
}

#[test]
fn filter_edges_keep_nodes() {
    let m = begin_mutation(&directed(&["A", "B", "C"], &[(0, 1, 1), (1, 2, 2), (2, 0, 3)]));
    m.filter_map_edges(|e| if *e % 2 == 0 { None } else { Some(e * 10) }).unwrap();
    let snap = m.to_snapshot();
    assert_eq!(snap.nodes.len(), 3);
    assert_eq!(snap.edges.iter().map(|e| e.data).collect::<Vec<_>>(), vec![10, 30]);
    m.filter_edges(|e| *e > 15).unwrap();
    assert_eq!(m.edge_count(), 1);
}

#[test]
fn unescape_token_value_change() {
    assert_ne!(json_pointer::unescape_token("a~1b"), "a~1b");
    assert_eq!(json_pointer::unescape_token("plain"), "plain");
}

#[test]
fn mutable_handles_share_identity() {
    let m = begin_mutation(&directed(&["A"], &[]));
    let n = m.clone();
    m.add_node("B".to_string()).unwrap();
    assert_eq!(n.node_count(), 2);
    let g = m.end_mutation().unwrap();
    assert!(m.add_node("C".to_string()).is_err());
    assert_eq!(g.node_count(), 2);
}

#[test]
fn option_payloads_structural() {
    let m = begin_mutation(&directed(&["A", "B"], &[(0, 1, 1)]));
    let g = m.end_mutation().unwrap();
    assert_eq!(graph::get_node(&g, 0), Some("A".to_string()));
    let h = directed(&["A", "B"], &[(0, 1, 1)]);
    assert_eq!(g, h);
}
