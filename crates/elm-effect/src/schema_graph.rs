//! `Schema.Graph`: the schema of an immutable [`Graph`].
//!
//! The pin's wire form is one object — the kind, the active nodes with their
//! stable indexes, and the active edges with theirs (`graphEncodedSchema`).
//! Decoding hands the snapshot to [`Graph::from_snapshot`], which already
//! enforces the pin's own rules (strictly increasing indexes, every endpoint
//! live), so the codec walks the wire and lets the graph refuse rather than
//! repeating those checks here.
//!
//! The value side is this crate's [`Graph`], so the declaration id is what
//! makes a persisted schema read back as `Schema.Graph(kind, node, edge)`
//! instead of as the object it encodes through.

use std::collections::BTreeMap;

use elm_effect_schema::{Schema, SchemaError, Unknown};

use crate::graph::{Graph, IndexedEdge, IndexedNode, Kind, Snapshot};

/// `Schema.Graph(kind, node, edge)`: a graph whose node payloads decode through
/// `node` and edge payloads through `edge`.
pub fn graph<N, E>(kind: Kind, node: Schema<N>, edge: Schema<E>) -> Schema<Graph<N, E>>
where
    N: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    let decode_node = node.clone();
    let decode_edge = edge.clone();
    let encode_node = node.clone();
    let encode_edge = edge.clone();
    let code = format!(
        "Schema.Graph({:?}, {}, {})",
        kind.name(),
        node.to_code(),
        edge.to_code()
    );
    let ty = format!(
        "Graph.Graph<{}, {}, {:?}>",
        node.to_type(),
        edge.to_type(),
        kind.name()
    );
    Schema::<Graph<N, E>>::try_convert(
        Schema::<Unknown>::any(),
        move |wire: Unknown| decode(wire, kind, &decode_node, &decode_edge),
        move |graph: &Graph<N, E>| encode(graph, kind, &encode_node, &encode_edge),
    )
    .with_declaration(
        "effect/schema/Graph",
        elm_effect_schema::serde_json::Value::String(kind.name().to_string()),
        vec![Schema::as_unknown(node), Schema::as_unknown(edge)],
    )
    .with_code(&code, &ty)
}

/// Read the pin's encoded object back into a graph.
fn decode<N, E>(
    wire: Unknown,
    kind: Kind,
    node: &Schema<N>,
    edge: &Schema<E>,
) -> Result<Graph<N, E>, SchemaError>
where
    N: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    let wire_kind = wire.kind();
    let Unknown::Object(mut fields) = wire else {
        return Err(refused("$", "an encoded Graph", wire_kind));
    };
    match fields.remove("type") {
        Some(Unknown::Text(found)) if found == kind.name() => {}
        Some(other) => {
            return Err(SchemaError::TypeMismatchDynamic {
                path: "$.type".to_string(),
                expected: format!("the {} graph kind", kind.name()),
                got: other.kind(),
            });
        }
        None => return Err(missing("$.type")),
    }

    let nodes = array(fields.remove("nodes"), "$.nodes")?;
    let mut decoded_nodes = Vec::with_capacity(nodes.len());
    for (position, element) in nodes.into_iter().enumerate() {
        let element_kind = element.kind();
        let Unknown::Object(mut fields) = element else {
            return Err(refused(
                &format!("$.nodes[{position}]"),
                "a node object",
                element_kind,
            ));
        };
        let index = index_of(
            fields.remove("index"),
            &format!("$.nodes[{position}].index"),
        )?;
        let data = node
            .decode(match fields.remove("data") {
                Some(value) => value,
                None => return Err(missing(&format!("$.nodes[{position}].data"))),
            })
            .map_err(|error| error.at(&format!("$.nodes[{position}].data")))?;
        decoded_nodes.push(IndexedNode { index, data });
    }

    let edges = array(fields.remove("edges"), "$.edges")?;
    let mut decoded_edges = Vec::with_capacity(edges.len());
    for (position, element) in edges.into_iter().enumerate() {
        let element_kind = element.kind();
        let Unknown::Object(mut fields) = element else {
            return Err(refused(
                &format!("$.edges[{position}]"),
                "an edge object",
                element_kind,
            ));
        };
        let index = index_of(
            fields.remove("index"),
            &format!("$.edges[{position}].index"),
        )?;
        let source = index_of(
            fields.remove("source"),
            &format!("$.edges[{position}].source"),
        )?;
        let target = index_of(
            fields.remove("target"),
            &format!("$.edges[{position}].target"),
        )?;
        let data = edge
            .decode(match fields.remove("data") {
                Some(value) => value,
                None => return Err(missing(&format!("$.edges[{position}].data"))),
            })
            .map_err(|error| error.at(&format!("$.edges[{position}].data")))?;
        decoded_edges.push(IndexedEdge {
            index,
            source,
            target,
            data,
        });
    }

    // The graph itself owns the index and endpoint rules, so its refusal is
    // reported as the refinement it is rather than re-stated at a guessed path.
    Graph::from_snapshot(Snapshot {
        kind,
        nodes: decoded_nodes,
        edges: decoded_edges,
    })
    .map_err(|error| SchemaError::FilterFailed {
        path: "$".to_string(),
        message: error.0,
    })
}

/// Write the pin's encoded object.
fn encode<N, E>(
    graph: &Graph<N, E>,
    kind: Kind,
    node: &Schema<N>,
    edge: &Schema<E>,
) -> Result<Unknown, SchemaError>
where
    N: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    let snapshot = graph.to_snapshot();
    // The kind is part of the schema, not of the value, so a graph of the other
    // kind has no representation here; the pin cannot reach this because its
    // value type carries the kind.
    if snapshot.kind != kind {
        return Err(SchemaError::EncodingFailed {
            path: "$".to_string(),
            expected: format!("an immutable {} Graph", kind.name()),
        });
    }
    let nodes = snapshot
        .nodes
        .iter()
        .map(|indexed| {
            Ok(Unknown::Object(BTreeMap::from([
                ("index".to_string(), Unknown::Number(indexed.index as f64)),
                ("data".to_string(), node.try_encode(&indexed.data)?),
            ])))
        })
        .collect::<Result<Vec<Unknown>, SchemaError>>()?;
    let edges = snapshot
        .edges
        .iter()
        .map(|indexed| {
            Ok(Unknown::Object(BTreeMap::from([
                ("index".to_string(), Unknown::Number(indexed.index as f64)),
                ("source".to_string(), Unknown::Number(indexed.source as f64)),
                ("target".to_string(), Unknown::Number(indexed.target as f64)),
                ("data".to_string(), edge.try_encode(&indexed.data)?),
            ])))
        })
        .collect::<Result<Vec<Unknown>, SchemaError>>()?;
    Ok(Unknown::Object(BTreeMap::from([
        ("type".to_string(), Unknown::Text(kind.name().to_string())),
        ("nodes".to_string(), Unknown::Array(nodes)),
        ("edges".to_string(), Unknown::Array(edges)),
    ])))
}

/// A required array field.
fn array(value: Option<Unknown>, path: &str) -> Result<Vec<Unknown>, SchemaError> {
    match value {
        Some(Unknown::Array(elements)) => Ok(elements),
        Some(other) => Err(refused(path, "an array", other.kind())),
        None => Err(missing(path)),
    }
}

/// A stable index: `Schema.Natural`, then the platform's own width. The
/// cast saturates rather than wrapping, so a count too large to hold is
/// refused here instead of collapsing two indexes onto one node.
fn index_of(value: Option<Unknown>, path: &str) -> Result<usize, SchemaError> {
    let value = value.ok_or_else(|| missing(path))?;
    let number = Schema::<f64>::natural()
        .decode(value)
        .map_err(|error| error.at(path))?;
    if number > usize::MAX as f64 {
        return Err(SchemaError::TypeMismatchDynamic {
            path: path.to_string(),
            expected: "an index this platform can hold".to_string(),
            got: "a larger number",
        });
    }
    Ok(number as usize)
}

fn refused(path: &str, expected: &'static str, got: &'static str) -> SchemaError {
    SchemaError::TypeMismatch {
        path: path.to_string(),
        expected,
        got,
    }
}

fn missing(path: &str) -> SchemaError {
    SchemaError::TypeMismatch {
        path: path.to_string(),
        expected: "a value",
        got: "missing",
    }
}
