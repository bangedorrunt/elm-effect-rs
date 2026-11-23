use crate::machine::Machine;
use crate::transition::Transition;
use crate::types::StateNodeType;
use std::collections::BTreeMap;
use std::fmt::Debug;

impl<S, E, C, CMD> Machine<S, E, C, CMD>
where
    S: Ord + Clone + Debug + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
    CMD: Clone + Send + Sync + 'static,
{
    /// Render the chart as a Mermaid `stateDiagram-v2`.
    ///
    /// Hierarchy becomes nested `state` blocks, so a reader sees compound and
    /// parallel structure. Edge labels come from [`Transition::label`](crate::Transition::label);
    /// an unlabelled event edge prints `on`, because a matcher is a closure
    /// and carries no name.
    #[must_use]
    pub fn to_mermaid(&self) -> String {
        let mut out = String::from("stateDiagram-v2\n");
        out.push_str(&format!(
            "    [*] --> {}\n",
            mermaid_id(&self.initial_state)
        ));
        for node in self.nodes.values() {
            if node.parent.is_none() {
                self.render_mermaid(&node.id, 1, &mut out);
            }
        }
        out
    }

    fn render_mermaid(&self, id: &S, indent: usize, out: &mut String) {
        let pad = "    ".repeat(indent);
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        match node.node_type {
            StateNodeType::Compound | StateNodeType::Parallel => {
                out.push_str(&format!("{pad}state {} {{\n", mermaid_id(id)));
                if let Some(initial) = &node.initial {
                    out.push_str(&format!("{pad}    [*] --> {}\n", mermaid_id(initial)));
                }
                for child in self.children(id) {
                    self.render_mermaid(child, indent + 1, out);
                }
                self.render_mermaid_edges(id, indent + 1, out);
                out.push_str(&format!("{pad}}}\n"));
            }
            StateNodeType::HistoryShallow | StateNodeType::HistoryDeep => {
                out.push_str(&format!("{pad}state {} <<history>>\n", mermaid_id(id)));
            }
            StateNodeType::Atomic | StateNodeType::Final => {
                self.render_mermaid_edges(id, indent, out);
            }
            StateNodeType::Choice => {
                out.push_str(&format!("{pad}state {} <<choice>>\n", mermaid_id(id)));
                self.render_mermaid_edges(id, indent, out);
            }
        }
    }

    fn render_mermaid_edges(&self, id: &S, indent: usize, out: &mut String) {
        let pad = "    ".repeat(indent);
        let Some(node) = self.nodes.get(id) else {
            return;
        };
        for transition in &node.transitions {
            if transition.is_targetless() {
                continue;
            }
            for target in transition.target_states() {
                out.push_str(&format!(
                    "{pad}{} --> {} : {}\n",
                    mermaid_id(id),
                    mermaid_id(target),
                    transition.edge_label().unwrap_or("on")
                ));
            }
        }
        for transition in &node.always {
            if transition.is_targetless() {
                continue;
            }
            for target in transition.target_states() {
                out.push_str(&format!(
                    "{pad}{} --> {} : always\n",
                    mermaid_id(id),
                    mermaid_id(target)
                ));
            }
        }
        for (delay, transition) in &node.after {
            if transition.is_targetless() {
                continue;
            }
            for target in transition.target_states() {
                out.push_str(&format!(
                    "{pad}{} --> {} : after {}\n",
                    mermaid_id(id),
                    mermaid_id(target),
                    delay.label()
                ));
            }
        }
        for transition in &node.on_done {
            if transition.is_targetless() {
                continue;
            }
            for target in transition.target_states() {
                out.push_str(&format!(
                    "{pad}{} --> {} : done\n",
                    mermaid_id(id),
                    mermaid_id(target)
                ));
            }
        }
    }
}

/// A Mermaid node id. `Debug` is the only rendering every state key has.
fn mermaid_id<S: Debug>(state: &S) -> String {
    format!("{state:?}")
}

impl<S, E, C, CMD> Machine<S, E, C, CMD>
where
    S: Ord + Clone + Debug + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
    CMD: Clone + Send + Sync + 'static,
{
    /// Render the chart as a TLA+ module named `Statechart`.
    ///
    /// The module models topology only: `Init`, `Next`, and the state set.
    /// Guards and context are Rust closures and cannot be translated, so the
    /// model over-approximates them (every edge is always enabled).
    #[must_use]
    pub fn to_tla(&self) -> String {
        self.to_tla_named("Statechart")
    }

    /// Render the chart as a TLA+ module with an explicit module name.
    ///
    /// ```text
    /// ---- MODULE Traffic ----
    /// EXTENDS Naturals
    /// VARIABLES state
    /// States == {"Green", "Yellow"}
    /// Init == state = "Green"
    /// Next == \/ state = "Green" /\ state' = "Yellow"
    /// ====
    /// ```
    #[must_use]
    pub fn to_tla_named(&self, module: &str) -> String {
        let mut states: Vec<String> = self
            .nodes
            .keys()
            .map(tla_state)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        states.sort();
        let mut out = format!(
            "---- MODULE {module} ----\nEXTENDS Naturals\n\nVARIABLES state\n\nStates == {{{}}}\n\nInit == state = {}\n\nNext ==\n",
            states.join(", "),
            tla_state(&self.initial_state)
        );
        let mut edges = Vec::new();
        for node in self.nodes.values() {
            let source = tla_state(&node.id);
            for transition in &node.transitions {
                if !transition.is_targetless() {
                    for target in transition.target_states() {
                        edges.push(tla_edge(&source, &tla_state(target)));
                    }
                }
            }
            for transition in &node.always {
                if !transition.is_targetless() {
                    for target in transition.target_states() {
                        edges.push(tla_edge(&source, &tla_state(target)));
                    }
                }
            }
            for (_, transition) in &node.after {
                if !transition.is_targetless() {
                    for target in transition.target_states() {
                        edges.push(tla_edge(&source, &tla_state(target)));
                    }
                }
            }
            for transition in &node.on_done {
                if !transition.is_targetless() {
                    for target in transition.target_states() {
                        edges.push(tla_edge(&source, &tla_state(target)));
                    }
                }
            }
        }
        if edges.is_empty() {
            out.push_str("    UNCHANGED state\n");
        } else {
            out.push_str("    ");
            out.push_str(&edges.join("\n    "));
            out.push('\n');
        }
        out.push_str("====\n");
        out
    }

    /// Export the machine topology as an XState v6-compatible MachineJSON object
    /// suitable for Stately Studio and Stately Inspector (ADR-0008 Law L11).
    #[must_use]
    pub fn to_machine_json(&self) -> serde_json::Value {
        let mut root = serde_json::Map::new();
        if self.root_parallel {
            root.insert("type".into(), serde_json::Value::String("parallel".into()));
        } else {
            root.insert(
                "initial".into(),
                serde_json::Value::String(json_state_id(&self.initial_state)),
            );
        }
        let mut states = serde_json::Map::new();
        for node in self.nodes.values() {
            if node.parent.is_none() {
                states.insert(
                    json_state_id(&node.id),
                    self.render_machine_json_node(&node.id),
                );
            }
        }
        root.insert("states".into(), serde_json::Value::Object(states));
        serde_json::Value::Object(root)
    }

    fn render_machine_json_node(&self, id: &S) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        let Some(node) = self.nodes.get(id) else {
            return serde_json::Value::Object(map);
        };
        match node.node_type {
            StateNodeType::Atomic => {}
            StateNodeType::Compound => {
                if let Some(initial) = &node.initial {
                    map.insert(
                        "initial".into(),
                        serde_json::Value::String(json_state_id(initial)),
                    );
                }
            }
            StateNodeType::Parallel => {
                map.insert("type".into(), serde_json::Value::String("parallel".into()));
            }
            StateNodeType::Final => {
                map.insert("type".into(), serde_json::Value::String("final".into()));
            }
            StateNodeType::Choice => {
                map.insert("type".into(), serde_json::Value::String("choice".into()));
            }
            StateNodeType::HistoryShallow | StateNodeType::HistoryDeep => {
                map.insert("type".into(), serde_json::Value::String("history".into()));
                let depth = if node.node_type == StateNodeType::HistoryDeep {
                    "deep"
                } else {
                    "shallow"
                };
                map.insert("history".into(), serde_json::Value::String(depth.into()));
                if node.initial_targets.len() == 1 {
                    map.insert(
                        "target".into(),
                        serde_json::Value::String(json_state_id(&node.initial_targets[0])),
                    );
                } else if node.initial_targets.len() > 1 {
                    map.insert(
                        "target".into(),
                        serde_json::Value::Array(
                            node.initial_targets
                                .iter()
                                .map(|target| serde_json::Value::String(json_state_id(target)))
                                .collect(),
                        ),
                    );
                }
            }
        }

        if !node.transitions.is_empty() {
            let mut by_event: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
            for transition in &node.transitions {
                let key = transition
                    .event_key()
                    .or_else(|| transition.edge_label())
                    .unwrap_or("EVENT")
                    .to_string();
                by_event
                    .entry(key)
                    .or_default()
                    .push(transition_to_json(transition));
            }
            let mut on_map = serde_json::Map::new();
            for (key, mut list) in by_event {
                let val = if list.len() == 1 {
                    list.remove(0)
                } else {
                    serde_json::Value::Array(list)
                };
                on_map.insert(key, val);
            }
            map.insert("on".into(), serde_json::Value::Object(on_map));
        }

        if !node.always.is_empty() {
            map.insert(
                "always".into(),
                serde_json::Value::Array(node.always.iter().map(transition_to_json).collect()),
            );
        }

        if !node.after.is_empty() {
            let mut by_delay: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
            for (delay, transition) in &node.after {
                let label = delay.label();
                let key = label.strip_suffix("ms").unwrap_or(&label).to_string();
                by_delay
                    .entry(key)
                    .or_default()
                    .push(transition_to_json(transition));
            }
            let mut after_map = serde_json::Map::new();
            for (key, mut list) in by_delay {
                let val = if list.len() == 1 {
                    list.remove(0)
                } else {
                    serde_json::Value::Array(list)
                };
                after_map.insert(key, val);
            }
            map.insert("after".into(), serde_json::Value::Object(after_map));
        }

        if !node.on_done.is_empty() {
            let val = if node.on_done.len() == 1 {
                transition_to_json(&node.on_done[0])
            } else {
                serde_json::Value::Array(node.on_done.iter().map(transition_to_json).collect())
            };
            map.insert("onDone".into(), val);
        }

        let children = self.children(id);
        if !children.is_empty() {
            let mut states_map = serde_json::Map::new();
            for child in children {
                states_map.insert(json_state_id(child), self.render_machine_json_node(child));
            }
            map.insert("states".into(), serde_json::Value::Object(states_map));
        }

        serde_json::Value::Object(map)
    }
}

fn transition_to_json<
    S: Clone + Debug + 'static,
    E: 'static,
    C: 'static,
    CMD: Send + Sync + 'static,
>(
    transition: &Transition<S, E, C, CMD>,
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if !transition.is_targetless() {
        let targets = transition.target_states();
        if targets.len() == 1 {
            obj.insert(
                "target".into(),
                serde_json::Value::String(json_state_id(&targets[0])),
            );
        } else if targets.len() > 1 {
            obj.insert(
                "target".into(),
                serde_json::Value::Array(
                    targets
                        .iter()
                        .map(|target| serde_json::Value::String(json_state_id(target)))
                        .collect(),
                ),
            );
        }
    }
    if transition.is_reentering() {
        obj.insert("reenter".into(), serde_json::Value::Bool(true));
    }
    serde_json::Value::Object(obj)
}

fn json_state_id<S: Debug>(state: &S) -> String {
    let rendered = format!("{state:?}");
    rendered
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .unwrap_or(&rendered)
        .to_string()
}

fn tla_edge(source: &str, target: &str) -> String {
    format!("\\/ state = {source} /\\ state' = {target}")
}

/// A TLA+ string constant. `Debug` quotes strings already; enums do not.
fn tla_state<S: Debug>(state: &S) -> String {
    let rendered = format!("{state:?}");
    let text = rendered
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .unwrap_or(&rendered);
    format!("\"{}\"", text.replace('"', "'"))
}
