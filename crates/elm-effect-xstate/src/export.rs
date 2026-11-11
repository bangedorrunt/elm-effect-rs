use crate::machine::Machine;
use crate::types::StateNodeType;
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
                    self.render_mermaid(&child, indent + 1, out);
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
