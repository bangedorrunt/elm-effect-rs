//! hi5 tail slice: pathfinding algorithm cells.
//! Ports packages/effect/test/Pathfinding.test.ts (Dijkstra, Bellman-Ford,
//! A*), left uncovered by the area beads. Module loads via `#[path]`
//! like port_misc.rs: no facade touch.
#![cfg(feature = "tokio")]

#[path = "../src/graph.rs"]
mod graph;

fn maze() -> graph::Graph<i32, i32> {
    graph::build(
        graph::Kind::Directed,
        |m: &graph::MutableGraph<i32, i32>| {
            for n in [0, 1, 2, 3, 4] {
                m.add_node(n)?;
            }
            m.add_edge(0, 1, 10)?;
            m.add_edge(0, 2, 1)?;
            m.add_edge(2, 1, 1)?;
            m.add_edge(1, 3, 1)?;
            m.add_edge(2, 3, 9)?;
            Ok(())
        },
    )
    .expect("fixture builds")
}

// Pathfinding.test.ts:27 — Dijkstra finds the cheapest path, skipping the
// expensive direct edge for the two-hop detour.
#[test]
fn dijkstra_finds_cheapest_detour() {
    let g = maze();
    let found = graph::dijkstra(&g, 0, 3, |e: &i32| *e as f64)
        .expect("no validation error")
        .expect("a path exists");
    assert_eq!(found.path, vec![0, 2, 1, 3]);
    assert_eq!(found.distance, 3.0);
    assert_eq!(
        graph::dijkstra(&g, 3, 4, |e: &i32| *e as f64).expect("valid query"),
        None,
        "isolated node has no path"
    );
}

// Pathfinding.test.ts:40 — Bellman-Ford agrees on the cheapest path.
#[test]
fn bellman_ford_finds_cheapest_detour() {
    let g = maze();
    let found = graph::bellman_ford(&g, 0, 3, |e: &i32| *e as f64)
        .expect("no validation error")
        .expect("a path exists");
    assert_eq!(found.path, vec![0, 2, 1, 3]);
    assert_eq!(found.distance, 3.0);
}

// Pathfinding.test.ts:53 — A* with an admissible heuristic agrees too.
#[test]
fn astar_finds_cheapest_detour() {
    let g = maze();
    let found = graph::astar(&g, 0, 3, |e: &i32| *e as f64, |_| 0.0)
        .expect("no validation error")
        .expect("a path exists");
    assert_eq!(found.path, vec![0, 2, 1, 3]);
    assert_eq!(found.distance, 3.0);
}
