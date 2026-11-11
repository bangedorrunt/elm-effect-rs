use crate::action::ActionQueue;
use crate::configuration::Configuration;
use crate::state_node::{LifecycleFn, StateNode};
use crate::transition::Transition;
use crate::types::{ArmedTimer, MachineState, StateNodeType, StepOutcome, TimerFired, TimerIntent};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

/// The micro-step bound that turns a non-terminating `always` loop into a
/// stopped machine instead of a hung one (ADR-0007 law L2).
const MAX_MICRO_STEPS: usize = 100;

/// One transition chosen for this macro-step, and the node that declares it.
#[derive(Clone)]
struct Selected<S, E, C, CMD> {
    source: S,
    transition: Transition<S, E, C, CMD>,
}

/// What the entry and exit machinery writes out during one macro-step.
struct Outbox<'a, S, CMD, E> {
    enq: &'a mut ActionQueue<CMD, E>,
    timers: &'a mut Vec<TimerIntent<S>>,
    internal: &'a mut VecDeque<E>,
}

impl<S, CMD, E> Outbox<'_, S, CMD, E> {
    /// Move events the actions raised into the internal queue of this macro-step.
    fn collect_raised(&mut self) {
        self.internal.extend(self.enq.take_raised());
    }
}

/// A compiled Harel statechart machine.
#[derive(Clone)]
pub struct Machine<S: Ord + Clone, E, C: Clone, CMD> {
    pub(crate) nodes: BTreeMap<S, StateNode<S, E, C, CMD>>,
    pub(crate) initial_state: S,
    pub(crate) initial_context: C,
    pub(crate) root_parallel: bool,
    pub(crate) root_entry_actions: Vec<LifecycleFn<C, CMD, E>>,
    pub(crate) root_exit_actions: Vec<LifecycleFn<C, CMD, E>>,
}

impl<S, E, C, CMD> Machine<S, E, C, CMD>
where
    S: Ord + Clone + std::fmt::Debug + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: Clone + Send + Sync + 'static,
    CMD: Clone + Send + Sync + 'static,
{
    /// Start building a machine with the specified root initial state and context.
    pub fn builder(initial_state: S, initial_context: C) -> MachineBuilder<S, E, C, CMD> {
        MachineBuilder::new(initial_state, initial_context)
    }

    /// Access the initial root state.
    #[must_use]
    pub fn initial_state(&self) -> &S {
        &self.initial_state
    }

    /// Access the initial extended context.
    #[must_use]
    pub fn initial_context(&self) -> &C {
        &self.initial_context
    }

    /// The configuration the machine starts in, with every compound and
    /// parallel node expanded to its leaves.
    #[must_use]
    pub fn initial_configuration(&self) -> Configuration<S> {
        if self.root_parallel {
            let mut config = Configuration::new();
            let mut visited = BTreeSet::new();
            for child in self.top_level_children() {
                self.expand_into(&child, &BTreeMap::new(), &mut visited, &mut config);
            }
            return config;
        }
        self.resolve_configuration(&self.initial_state)
    }

    /// Expand a state into the leaves entered when it is the target.
    #[must_use]
    pub fn resolve_configuration(&self, state: &S) -> Configuration<S> {
        let mut config = Configuration::new();
        let mut visited = BTreeSet::new();
        self.expand_into(state, &BTreeMap::new(), &mut visited, &mut config);
        config
    }

    /// Resolve a state to its primary active leaf, following initial sub-states.
    #[must_use]
    pub fn resolve_leaf(&self, state: &S) -> S {
        self.resolve_configuration(state)
            .primary()
            .cloned()
            .unwrap_or_else(|| state.clone())
    }

    fn node(&self, id: &S) -> Option<&StateNode<S, E, C, CMD>> {
        self.nodes.get(id)
    }

    fn node_type(&self, id: &S) -> StateNodeType {
        self.nodes
            .get(id)
            .map_or(StateNodeType::Atomic, |node| node.node_type)
    }

    fn is_history(&self, id: &S) -> bool {
        matches!(
            self.node_type(id),
            StateNodeType::HistoryShallow | StateNodeType::HistoryDeep
        )
    }

    fn parent_of(&self, id: &S) -> Option<S> {
        self.nodes.get(id).and_then(|node| node.parent.clone())
    }

    /// `[state, parent, grandparent, ...]`.
    fn ancestry(&self, state: &S) -> Vec<S> {
        let mut path = Vec::new();
        let mut cursor = Some(state.clone());
        while let Some(current) = cursor {
            path.push(current.clone());
            cursor = self.parent_of(&current);
        }
        path
    }

    /// `[root, ..., parent, state]`.
    fn path_from_root(&self, state: &S) -> Vec<S> {
        let mut path = self.ancestry(state);
        path.reverse();
        path
    }

    /// Distance from a top-level node, counted in nodes.
    fn depth(&self, state: &S) -> usize {
        self.ancestry(state).len()
    }

    /// Direct children, in deterministic identifier order.
    pub(crate) fn children(&self, id: &S) -> Vec<S> {
        self.nodes
            .values()
            .filter(|node| node.parent.as_ref() == Some(id))
            .map(|node| node.id.clone())
            .collect()
    }

    /// Nodes directly under the implicit root.
    fn top_level_children(&self) -> Vec<S> {
        self.nodes
            .values()
            .filter(|node| node.parent.is_none())
            .map(|node| node.id.clone())
            .collect()
    }

    fn is_strict_descendant(&self, child: &S, ancestor: &S) -> bool {
        let mut cursor = self.parent_of(child);
        while let Some(current) = cursor {
            if &current == ancestor {
                return true;
            }
            cursor = self.parent_of(&current);
        }
        false
    }

    /// Deepest shared ancestor. `None` is the implicit machine root.
    fn lca(&self, first: &S, second: &S) -> Option<S> {
        let first_path = self.path_from_root(first);
        let second_path = self.path_from_root(second);
        let mut common = None;
        for (a, b) in first_path.iter().zip(second_path.iter()) {
            if a == b {
                common = Some(a.clone());
            } else {
                break;
            }
        }
        common
    }

    /// The states that stay active across this transition (ADR-0007 law L1).
    ///
    /// A target equal to the source exits and re-enters it. A target strictly
    /// inside the source keeps the source active. A target strictly above the
    /// source exits and re-enters that ancestor. Everything else exits below
    /// the deepest shared ancestor.
    fn transition_domain_single(&self, source: &S, target: &S, reenter: bool) -> Option<S> {
        if target == source {
            return if reenter {
                self.parent_of(source)
            } else {
                Some(source.clone())
            };
        }
        if self.is_strict_descendant(target, source) {
            if reenter {
                return self.parent_of(source);
            }
            return self
                .parallel_target_region(source, target)
                .or_else(|| Some(source.clone()));
        }
        if self.is_strict_descendant(source, target) {
            return self.parent_of(target);
        }
        let domain = self.lca(source, target);
        domain
            .as_ref()
            .and_then(|ancestor| self.parallel_target_region(ancestor, target))
            .or(domain)
    }

    /// The target's region when the transition domain is a parallel node.
    ///
    /// XState keeps the other regions active when a transition reaches into
    /// one region, including when the source leaf is in a different region.
    fn parallel_target_region(&self, ancestor: &S, target: &S) -> Option<S> {
        if self.node_type(ancestor) != StateNodeType::Parallel {
            return None;
        }
        let path = self.path_from_root(target);
        let position = path.iter().position(|node| node == ancestor)?;
        let region = path.get(position + 1)?;
        (region != target).then(|| region.clone())
    }

    /// Resolve history targets to the states they actually enter, following
    /// XState's effective-target rule: a recorded deep history contributes its
    /// leaves, shallow history its immediate children, and an unvisited history
    /// contributes its fallback.
    fn effective_targets(&self, targets: &[S], history: &BTreeMap<S, Configuration<S>>) -> Vec<S> {
        let mut effective = Vec::new();
        let mut seen = BTreeSet::new();
        for target in targets {
            if !self.is_history(target) {
                if seen.insert(target.clone()) {
                    effective.push(target.clone());
                }
                continue;
            }
            let recorded = history.get(target).filter(|recorded| !recorded.is_empty());
            if let Some(recorded) = recorded {
                let boundary = self.parent_of(target);
                for leaf in recorded.leaves() {
                    let node = if self.node_type(target) == StateNodeType::HistoryShallow {
                        let mut cursor = leaf.clone();
                        while let Some(parent) = self.parent_of(&cursor) {
                            if Some(&parent) == boundary.as_ref() {
                                break;
                            }
                            cursor = parent;
                        }
                        cursor
                    } else {
                        leaf.clone()
                    };
                    if seen.insert(node.clone()) {
                        effective.push(node);
                    }
                }
            } else if let Some(node) = self.node(target) {
                for fallback in node.initial_targets.clone() {
                    if seen.insert(fallback.clone()) {
                        effective.push(fallback);
                    }
                }
            }
        }
        effective
    }

    /// Keep a parallel domain only as wide as the region that contains every
    /// target, so sibling regions are not reset by a region-local transition.
    fn narrow_parallel_domain(&self, domain: Option<S>, targets: &[S]) -> Option<S> {
        let mut narrowed = domain?;
        while self.node_type(&narrowed) == StateNodeType::Parallel {
            let Some(region) = self.children(&narrowed).into_iter().find(|child| {
                targets
                    .iter()
                    .all(|target| target == child || self.is_strict_descendant(target, child))
            }) else {
                break;
            };
            narrowed = region;
        }
        Some(narrowed)
    }

    /// The transition domain for a target set. A single target keeps the
    /// documented self/descendant/ancestor rules; several targets take the
    /// deepest proper common ancestor of the source and every target.
    fn transition_domain(
        &self,
        source: &S,
        targets: &[S],
        history: &BTreeMap<S, Configuration<S>>,
        reenter: bool,
    ) -> Option<S> {
        if targets.len() == 1 {
            return self.transition_domain_single(source, &targets[0], reenter);
        }
        let effective = self.effective_targets(targets, history);
        if effective.is_empty() {
            return None;
        }
        if effective.len() == 1 {
            return self.transition_domain_single(source, &effective[0], reenter);
        }
        if effective
            .iter()
            .all(|target| target == source || self.is_strict_descendant(target, source))
        {
            if reenter {
                return self.parent_of(source);
            }
            return self.narrow_parallel_domain(Some(source.clone()), &effective);
        }

        let source_path = self.path_from_root(source);
        let mut common = None;
        for (position, node) in source_path.iter().enumerate() {
            let shared = effective.iter().all(|target| {
                let target_path = self.path_from_root(target);
                target_path.get(position) == Some(node) && target_path.len() > position + 1
            });
            if !shared {
                break;
            }
            common = Some(node.clone());
        }
        if reenter {
            return common;
        }
        self.narrow_parallel_domain(common, &effective)
    }

    /// Every active node, leaves and their ancestors.
    fn active_nodes(&self, config: &Configuration<S>) -> BTreeSet<S> {
        let mut nodes = BTreeSet::new();
        for leaf in config.leaves() {
            for ancestor in self.ancestry(leaf) {
                nodes.insert(ancestor);
            }
        }
        nodes
    }

    fn has_active_under(&self, config: &Configuration<S>, id: &S) -> bool {
        config
            .leaves()
            .any(|leaf| leaf == id || self.is_strict_descendant(leaf, id))
    }

    /// Whether the whole chart finished: the declared root completed, or a
    /// top-level final state is active.
    ///
    /// The two rules cover both builder shapes: a machine whose root is one
    /// compound or parallel node, and an XState schema whose top-level states
    /// are siblings and one of them is `"type": "final"`.
    fn root_done(&self, config: &Configuration<S>) -> bool {
        if self.root_parallel {
            let children: Vec<S> = self
                .top_level_children()
                .into_iter()
                .filter(|child| !self.is_history(child))
                .collect();
            return !children.is_empty()
                && children.iter().all(|child| self.node_done(config, child));
        }
        self.node_done(config, &self.initial_state)
            || config.leaves().any(|leaf| {
                self.node_type(leaf) == StateNodeType::Final && self.parent_of(leaf).is_none()
            })
    }

    fn node_done(&self, config: &Configuration<S>, id: &S) -> bool {
        let Some(node) = self.node(id) else {
            return false;
        };
        match node.node_type {
            StateNodeType::Final => self.has_active_under(config, id),
            StateNodeType::Atomic | StateNodeType::HistoryShallow | StateNodeType::HistoryDeep => {
                false
            }
            StateNodeType::Compound => {
                let active: Vec<S> = self
                    .children(id)
                    .into_iter()
                    .filter(|child| self.has_active_under(config, child))
                    .collect();
                matches!(active.as_slice(), [child] if self.node_type(child) == StateNodeType::Final)
            }
            StateNodeType::Parallel => {
                let children: Vec<S> = self
                    .children(id)
                    .into_iter()
                    .filter(|child| !self.is_history(child))
                    .collect();
                !children.is_empty() && children.iter().all(|child| self.node_done(config, child))
            }
        }
    }

    /// Pure entry expansion used for initial configuration and diagram queries.
    fn expand_into(
        &self,
        id: &S,
        history: &BTreeMap<S, Configuration<S>>,
        visited: &mut BTreeSet<S>,
        out: &mut Configuration<S>,
    ) {
        if !visited.insert(id.clone()) {
            return;
        }
        let Some(node) = self.node(id) else {
            out.insert(id.clone());
            return;
        };
        match node.node_type {
            StateNodeType::Atomic | StateNodeType::Final => {
                out.insert(id.clone());
            }
            StateNodeType::Compound => {
                if let Some(initial) = node.initial.clone() {
                    self.expand_into(&initial, history, visited, out);
                }
            }
            StateNodeType::Parallel => {
                for child in self
                    .children(id)
                    .into_iter()
                    .filter(|child| !self.is_history(child))
                {
                    self.expand_into(&child, history, visited, out);
                }
            }
            StateNodeType::HistoryShallow | StateNodeType::HistoryDeep => {
                if let Some(recorded) = history.get(id).filter(|recorded| !recorded.is_empty()) {
                    if node.node_type == StateNodeType::HistoryShallow {
                        let boundary = self.parent_of(id);
                        let mut seen = BTreeSet::new();
                        for leaf in recorded.leaves() {
                            let mut cursor = leaf.clone();
                            while let Some(parent) = self.parent_of(&cursor) {
                                if Some(&parent) == boundary.as_ref() {
                                    break;
                                }
                                cursor = parent;
                            }
                            if seen.insert(cursor.clone()) {
                                self.expand_into(&cursor, history, visited, out);
                            }
                        }
                    } else {
                        for leaf in recorded.leaves() {
                            out.insert(leaf.clone());
                        }
                    }
                    return;
                }
                for fallback in node.initial_targets.clone() {
                    self.expand_into(&fallback, history, visited, out);
                }
            }
        }
    }

    /// Choose at most one transition per active region.
    ///
    /// The walk starts at the leaf and climbs, so the nearest handler wins
    /// (law L3). One selection per source node is kept, so a transition on a
    /// shared ancestor is taken once. When a deeper source also selected a
    /// transition, the ancestor's selection is preempted.
    fn select(
        &self,
        config: &Configuration<S>,
        ctx: &C,
        event: &E,
        transient: bool,
    ) -> Vec<Selected<S, E, C, CMD>> {
        let mut selected: Vec<Selected<S, E, C, CMD>> = Vec::new();
        let mut chosen: BTreeSet<S> = BTreeSet::new();

        for leaf in config.leaves() {
            for ancestor in self.ancestry(leaf) {
                if chosen.contains(&ancestor) {
                    break;
                }
                let Some(node) = self.node(&ancestor) else {
                    continue;
                };
                let found = if self.node_type(&ancestor) == StateNodeType::Final {
                    None
                } else if transient {
                    node.always
                        .iter()
                        .find(|transition| transition.guard_passes_in(config, ctx, event))
                } else {
                    let mut candidates: Vec<&Transition<S, E, C, CMD>> = node
                        .transitions
                        .iter()
                        .filter(|transition| transition.is_enabled(config, ctx, event))
                        .collect();
                    candidates.sort_by_key(|transition| transition.selection_priority_for(event));
                    candidates.first().copied().or_else(|| {
                        node.after
                            .iter()
                            .map(|(_, transition)| transition)
                            .find(|transition| transition.is_enabled(config, ctx, event))
                    })
                };
                if let Some(transition) = found {
                    chosen.insert(ancestor.clone());
                    selected.push(Selected {
                        source: ancestor.clone(),
                        transition: transition.clone(),
                    });
                    break;
                }
            }
        }

        let sources: Vec<S> = selected.iter().map(|item| item.source.clone()).collect();
        selected.retain(|item| {
            !sources.iter().any(|other| {
                other != &item.source && self.is_strict_descendant(other, &item.source)
            })
        });
        selected
    }

    /// The deepest completed node with an enabled `on_done` transition.
    fn select_done(
        &self,
        state: &MachineState<S, C>,
        event: &E,
        handled: &BTreeSet<S>,
    ) -> Option<Selected<S, E, C, CMD>> {
        let mut candidates: Vec<S> = self
            .active_nodes(&state.configuration)
            .into_iter()
            .filter(|id| {
                !handled.contains(id)
                    && self.node_type(id) != StateNodeType::Final
                    && self.node(id).is_some_and(|node| !node.on_done.is_empty())
                    && self.node_done(&state.configuration, id)
            })
            .collect();
        candidates.sort_by(|a, b| self.depth(b).cmp(&self.depth(a)).then_with(|| a.cmp(b)));

        for id in candidates {
            if let Some(node) = self.node(&id) {
                for transition in &node.on_done {
                    if transition.guard_passes_in(&state.configuration, &state.context, event) {
                        return Some(Selected {
                            source: id,
                            transition: transition.clone(),
                        });
                    }
                }
            }
        }
        None
    }

    /// Run the exit side of a node sequence already ordered child-to-parent.
    fn run_exit_actions(
        &self,
        ordered: &[S],
        state: &mut MachineState<S, C>,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        for id in ordered {
            if let Some(node) = self.node(id) {
                for action in &node.exit_actions {
                    action(&mut state.context, out.enq);
                }
                let cancelled: Vec<_> = state
                    .armed
                    .iter()
                    .filter(|timer| timer.state == *id)
                    .map(|timer| timer.duration)
                    .collect();
                state.armed.retain(|timer| timer.state != *id);
                for duration in cancelled {
                    out.timers.push(TimerIntent::Cancel {
                        state: id.clone(),
                        duration,
                    });
                }
            }
        }
    }

    /// Exit every active node and then the implicit machine root when the
    /// chart completes. The configuration stays readable as the final
    /// snapshot; only lifecycle actions and timers run.
    fn finalize_done(&self, state: &mut MachineState<S, C>, out: &mut Outbox<'_, S, CMD, E>) {
        let mut ordered: Vec<S> = self
            .active_nodes(&state.configuration)
            .into_iter()
            .collect();
        ordered.sort_by_key(|state| std::cmp::Reverse(self.path_from_root(state)));
        self.run_exit_actions(&ordered, state, out);
        for action in &self.root_exit_actions {
            action(&mut state.context, out.enq);
        }
        out.collect_raised();
    }

    /// Record history for every compound or parallel state this step exits.
    fn record_history(
        &self,
        before: &Configuration<S>,
        exited: &BTreeSet<S>,
        history: &mut BTreeMap<S, Configuration<S>>,
    ) {
        for node_id in exited {
            if !matches!(
                self.node_type(node_id),
                StateNodeType::Compound | StateNodeType::Parallel
            ) {
                continue;
            }
            for child in self.children(node_id) {
                if !matches!(
                    self.node_type(&child),
                    StateNodeType::HistoryShallow | StateNodeType::HistoryDeep
                ) {
                    continue;
                }
                let recorded = Configuration::from_leaves(
                    before
                        .leaves()
                        .filter(|leaf| self.is_strict_descendant(leaf, node_id))
                        .cloned(),
                );
                if !recorded.is_empty() {
                    history.insert(child, recorded);
                }
            }
        }
    }

    /// Put one leaf into the configuration and run its entry side effects.
    ///
    /// `entered` is the per-macro-step guard: it holds the nodes that were
    /// already active before entry started, so a node that stayed active does
    /// not run its entry actions twice.
    fn enter_self(
        &self,
        id: &S,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
        event: Option<&E>,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        if !entered.insert(id.clone()) {
            return;
        }
        let Some(node) = self.node(id) else {
            return;
        };
        for action in &node.entry_actions {
            action(&mut state.context, out.enq);
        }
        for hook in &node.entry_hooks {
            hook(&mut state.context, out.enq, event);
        }
        if node.node_type != StateNodeType::Final {
            for (index, (delay, _)) in node.after.iter().enumerate() {
                if let Some(duration) = delay.resolve(&state.context, event) {
                    out.timers.push(TimerIntent::Start {
                        state: id.clone(),
                        duration,
                    });
                    state.armed.push(ArmedTimer {
                        state: id.clone(),
                        duration,
                        transition: index,
                    });
                }
            }
        }
        if matches!(node.node_type, StateNodeType::Atomic | StateNodeType::Final) {
            state.configuration.insert(id.clone());
        }
    }

    /// Enter a node and everything its type expands to.
    fn enter_node(
        &self,
        id: &S,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
        event: Option<&E>,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        if entered.contains(id) {
            return;
        }
        self.enter_self(id, state, entered, event, out);
        let Some(node) = self.node(id) else {
            return;
        };
        match node.node_type {
            StateNodeType::Compound => {
                if let Some(initial) = node.initial.clone() {
                    self.enter_target(&initial, state, entered, event, out);
                }
            }
            StateNodeType::Parallel => {
                for child in self
                    .children(id)
                    .into_iter()
                    .filter(|child| !self.is_history(child))
                {
                    self.enter_target(&child, state, entered, event, out);
                }
            }
            _ => {}
        }
    }

    /// Enter the implicit root's initial regions: every top-level child for a
    /// parallel root, or the declared initial state otherwise.
    fn enter_root(
        &self,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
        event: Option<&E>,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        if self.root_parallel {
            for child in self
                .top_level_children()
                .into_iter()
                .filter(|child| !self.is_history(child))
            {
                self.enter_target(&child, state, entered, event, out);
            }
        } else {
            self.enter_target(&self.initial_state, state, entered, event, out);
        }
    }

    /// Enter a target, resolving a history node to its record or fallback.
    fn enter_target(
        &self,
        id: &S,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
        event: Option<&E>,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        let Some(node) = self.node(id) else {
            return;
        };
        match node.node_type {
            StateNodeType::HistoryShallow | StateNodeType::HistoryDeep => {
                let recorded = state.history.get(id).cloned().filter(|r| !r.is_empty());
                match recorded {
                    Some(recorded) => {
                        if node.node_type == StateNodeType::HistoryShallow {
                            let boundary = self.parent_of(id);
                            let mut seen = BTreeSet::new();
                            for leaf in recorded.leaves() {
                                let mut cursor = leaf.clone();
                                while let Some(parent) = self.parent_of(&cursor) {
                                    if Some(&parent) == boundary.as_ref() {
                                        break;
                                    }
                                    cursor = parent;
                                }
                                if seen.insert(cursor.clone()) {
                                    self.enter_node(&cursor, state, entered, event, out);
                                }
                            }
                        } else {
                            self.restore_deep(&recorded, id, state, entered, event, out);
                        }
                    }
                    None => {
                        let boundary = self.parent_of(id);
                        for fallback in node.initial_targets.clone() {
                            if fallback == *id {
                                continue;
                            }
                            self.enter_from_domain(
                                &fallback,
                                boundary.as_ref(),
                                state,
                                entered,
                                event,
                                out,
                            );
                        }
                    }
                }
            }
            _ => self.enter_node(id, state, entered, event, out),
        }
    }

    /// Re-enter a remembered leaf set, running entry actions from the top down.
    fn restore_deep(
        &self,
        recorded: &Configuration<S>,
        history_id: &S,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
        event: Option<&E>,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        let boundary = self.parent_of(history_id);
        let mut nodes = BTreeSet::new();
        for leaf in recorded.leaves() {
            for ancestor in self.ancestry(leaf) {
                if Some(&ancestor) == boundary.as_ref() {
                    break;
                }
                nodes.insert(ancestor);
            }
        }
        let mut ordered: Vec<S> = nodes.into_iter().collect();
        ordered.sort_by(|a, b| self.depth(a).cmp(&self.depth(b)).then_with(|| a.cmp(b)));
        for node in ordered {
            self.enter_self(&node, state, entered, event, out);
        }
    }

    /// Enter the path from the transition domain down to the target.
    fn enter_from_domain(
        &self,
        target: &S,
        domain: Option<&S>,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
        event: Option<&E>,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        let mut path = self.path_from_root(target);
        if let Some(domain) = domain
            && let Some(position) = path.iter().position(|node| node == domain)
        {
            path.drain(..=position);
        }
        let Some((last, ancestors)) = path.split_last() else {
            if domain == Some(target) {
                self.enter_default_descendants(target, state, entered, event, out);
            }
            return;
        };
        for ancestor in ancestors {
            self.enter_self(ancestor, state, entered, event, out);
        }
        self.enter_target(last, state, entered, event, out);
    }

    /// Re-enter the default children of a state that stays active across a
    /// non-reentering self-transition.
    fn enter_default_descendants(
        &self,
        id: &S,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
        event: Option<&E>,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        let Some(node) = self.node(id) else {
            return;
        };
        match node.node_type {
            StateNodeType::Compound => {
                if let Some(initial) = node.initial.clone() {
                    self.enter_target(&initial, state, entered, event, out);
                }
            }
            StateNodeType::Parallel => {
                for child in self
                    .children(id)
                    .into_iter()
                    .filter(|child| !self.is_history(child))
                {
                    self.enter_target(&child, state, entered, event, out);
                }
            }
            _ => {}
        }
    }

    /// Apply the chosen transitions in law L1 order: exit, actions, entry.
    fn apply(
        &self,
        state: &mut MachineState<S, C>,
        selections: &[Selected<S, E, C, CMD>],
        event: &E,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        if selections.is_empty() {
            return;
        }
        let before = state.configuration.clone();
        let active = self.active_nodes(&before);

        let mut exiting: BTreeSet<S> = BTreeSet::new();
        let mut domains: Vec<Option<S>> = Vec::with_capacity(selections.len());
        for selection in selections {
            if selection.transition.is_targetless() {
                domains.push(None);
                continue;
            }
            let domain = self.transition_domain(
                &selection.source,
                selection.transition.target_states(),
                &state.history,
                selection.transition.is_reentering(),
            );
            for node in &active {
                let below = match &domain {
                    Some(domain) => self.is_strict_descendant(node, domain),
                    None => true,
                };
                if below {
                    exiting.insert(node.clone());
                }
            }
            domains.push(domain);
        }

        self.record_history(&before, &exiting, &mut state.history);

        let mut ordered: Vec<S> = exiting.into_iter().collect();
        ordered.sort_by_key(|state| std::cmp::Reverse(self.path_from_root(state)));
        self.run_exit_actions(&ordered, state, out);
        state.configuration = Configuration::from_leaves(
            before
                .leaves()
                .filter(|leaf| !ordered.iter().any(|node| node == *leaf))
                .cloned(),
        );

        for selection in selections {
            for action in &selection.transition.actions {
                action(&mut state.context, event, out.enq);
            }
        }

        let mut entered = self.active_nodes(&state.configuration);
        for (selection, domain) in selections.iter().zip(domains) {
            if selection.transition.is_targetless() {
                continue;
            }
            for target in selection.transition.target_states() {
                self.enter_from_domain(
                    target,
                    domain.as_ref(),
                    state,
                    &mut entered,
                    Some(event),
                    out,
                );
            }
        }
        out.collect_raised();
    }

    /// Run eventless, completion, and raised-event micro-steps to quiescence.
    fn settle(
        &self,
        state: &mut MachineState<S, C>,
        event: &E,
        out: &mut Outbox<'_, S, CMD, E>,
    ) -> bool {
        let mut taken = false;
        let mut handled_done: BTreeSet<S> = BTreeSet::new();

        for _ in 0..MAX_MICRO_STEPS {
            let transient = self.select(&state.configuration, &state.context, event, true);
            if !transient.is_empty() {
                self.apply(state, &transient, event, out);
                taken = true;
                continue;
            }

            if let Some(done) = self.select_done(state, event, &handled_done) {
                handled_done.insert(done.source.clone());
                self.apply(state, std::slice::from_ref(&done), event, out);
                taken = true;
                continue;
            }

            let Some(raised) = out.internal.pop_front() else {
                break;
            };
            let selections = self.select(&state.configuration, &state.context, &raised, false);
            if !selections.is_empty() {
                self.apply(state, &selections, &raised, out);
                taken = true;
            }
        }
        taken
    }

    fn outcome(
        &self,
        mut state: MachineState<S, C>,
        mut enq: ActionQueue<CMD, E>,
        mut timers: Vec<TimerIntent<S>>,
        mut internal: VecDeque<E>,
        transitioned: bool,
        was_done: bool,
    ) -> StepOutcome<S, C, CMD, E> {
        let done = self.root_done(&state.configuration);
        if done && !was_done {
            let mut out = Outbox {
                enq: &mut enq,
                timers: &mut timers,
                internal: &mut internal,
            };
            self.finalize_done(&mut state, &mut out);
        }
        let current_state = state
            .configuration
            .primary()
            .cloned()
            .unwrap_or_else(|| self.initial_state.clone());
        StepOutcome {
            current_state,
            configuration: state.configuration,
            history: state.history,
            context: state.context,
            commands: enq.take_commands(),
            raised_events: internal.into_iter().collect(),
            emitted: enq.take_emitted(),
            directives: enq.take_directives(),
            timers,
            armed: state.armed,
            transitioned,
            done,
        }
    }

    /// Enter the machine's initial configuration.
    ///
    /// This is the pure start edge: it runs the entry actions of every state
    /// entered, arms their `after` delays as [`TimerIntent::Start`] values, and
    /// returns the resulting model. Eventless transitions wait for the first
    /// [`step`](Self::step), because a guard and a transition action both read
    /// the event, and no event exists yet.
    pub fn enter(&self, context: &C) -> StepOutcome<S, C, CMD, E> {
        self.enter_inner(context, None)
    }

    /// Enter like [`enter`](Self::enter), but make `init` the event that entry
    /// actions and delay functions observe. No eventless transitions settle.
    pub fn enter_with_event(&self, context: &C, init: &E) -> StepOutcome<S, C, CMD, E> {
        self.enter_inner(context, Some(init))
    }

    fn enter_inner(&self, context: &C, init: Option<&E>) -> StepOutcome<S, C, CMD, E> {
        let mut state = MachineState::new(Configuration::new(), context.clone());
        let mut enq = ActionQueue::new();
        let mut timers = Vec::new();
        let mut internal = VecDeque::new();
        {
            for action in &self.root_entry_actions {
                action(&mut state.context, &mut enq);
            }
            let mut out = Outbox {
                enq: &mut enq,
                timers: &mut timers,
                internal: &mut internal,
            };
            let mut entered = BTreeSet::new();
            self.enter_root(&mut state, &mut entered, init, &mut out);
            out.collect_raised();
        }
        self.outcome(state, enq, timers, internal, false, false)
    }

    /// Enter the machine and settle its initial macrostep.
    ///
    /// Entry actions may raise events, and active states may declare `always`
    /// transitions. Both run before `start` returns, with `init` available to
    /// guards and actions. [`enter`](Self::enter) remains the unsettled entry
    /// edge for hosts that need to supply their own first event.
    pub fn start(&self, context: &C, init: &E) -> StepOutcome<S, C, CMD, E> {
        let mut state = MachineState::new(Configuration::new(), context.clone());
        let mut enq = ActionQueue::new();
        let mut timers = Vec::new();
        let mut internal = VecDeque::new();
        let transitioned = {
            for action in &self.root_entry_actions {
                action(&mut state.context, &mut enq);
            }
            let mut out = Outbox {
                enq: &mut enq,
                timers: &mut timers,
                internal: &mut internal,
            };
            let mut entered = BTreeSet::new();
            self.enter_root(&mut state, &mut entered, Some(init), &mut out);
            out.collect_raised();
            self.settle(&mut state, init, &mut out)
        };
        self.outcome(state, enq, timers, internal, transitioned, false)
    }

    /// Process an event through the statechart, following hierarchy bubbling
    /// and law L4 eventless loops.
    pub fn step(&self, current: &S, context: &C, event: &E) -> StepOutcome<S, C, CMD, E> {
        let state = MachineState::new(self.resolve_configuration(current), context.clone());
        self.step_state(&state, event)
    }

    /// Process an event from a full run state, including recorded history.
    pub fn step_state(&self, state: &MachineState<S, C>, event: &E) -> StepOutcome<S, C, CMD, E> {
        let mut working = state.clone();
        if working.configuration.is_empty() {
            working.configuration = self.initial_configuration();
        }
        let was_done = self.root_done(&working.configuration);
        let mut enq = ActionQueue::new();
        let mut timers = Vec::new();
        let mut internal = VecDeque::new();
        let transitioned = {
            let mut out = Outbox {
                enq: &mut enq,
                timers: &mut timers,
                internal: &mut internal,
            };
            let selections = self.select(&working.configuration, &working.context, event, false);
            let taken = !selections.is_empty();
            if taken {
                self.apply(&mut working, &selections, event, &mut out);
            }
            self.settle(&mut working, event, &mut out) || taken
        };
        self.outcome(working, enq, timers, internal, transitioned, was_done)
    }

    /// Timer intents for the delays armed by the active states, in entry order.
    /// A host resuming from a snapshot arms these before stepping.
    #[must_use]
    pub fn timers_for_state(&self, state: &MachineState<S, C>) -> Vec<TimerIntent<S>> {
        let active = self.active_nodes(&state.configuration);
        if !state.armed.is_empty() {
            return state
                .armed
                .iter()
                .filter(|timer| active.contains(&timer.state))
                .map(|timer| TimerIntent::Start {
                    state: timer.state.clone(),
                    duration: timer.duration,
                })
                .collect();
        }
        let mut timers = Vec::new();
        for id in active {
            if let Some(node) = self.node(&id) {
                for (delay, _) in &node.after {
                    if let Some(duration) = delay.resolve(&state.context, None) {
                        timers.push(TimerIntent::Start {
                            state: id.clone(),
                            duration,
                        });
                    }
                }
            }
        }
        timers
    }

    /// Rebuild runtime timer bookkeeping from a restored configuration and
    /// context. Serialized armed timers are authoritative, including durations
    /// originally resolved from an entry event. Missing context-only delays are
    /// reconstructed; an event-dependent delay with no serialized entry cannot
    /// be reconstructed and stays unarmed.
    pub fn rearm_timers(&self, state: &mut MachineState<S, C>) {
        let active = self.active_nodes(&state.configuration);
        state.armed.retain(|timer| {
            active.contains(&timer.state)
                && self
                    .node(&timer.state)
                    .is_some_and(|node| timer.transition < node.after.len())
        });
        for id in active {
            let Some(node) = self.node(&id) else {
                continue;
            };
            for (index, (delay, _)) in node.after.iter().enumerate() {
                if state
                    .armed
                    .iter()
                    .any(|timer| timer.state == id && timer.transition == index)
                {
                    continue;
                }
                if let Some(duration) = delay.resolve(&state.context, None) {
                    state.armed.push(ArmedTimer {
                        state: id.clone(),
                        duration,
                        transition: index,
                    });
                }
            }
        }
    }

    /// Take the `after` transition of `timer.state` when its delay elapsed.
    ///
    /// A timer that fires for a state no longer active is a no-op, which is
    /// how a preempted delay is cancelled without a cancel race.
    pub fn step_timer(
        &self,
        state: &MachineState<S, C>,
        timer: &TimerFired<S>,
        event: &E,
    ) -> StepOutcome<S, C, CMD, E> {
        let mut working = state.clone();
        if working.configuration.is_empty() {
            working.configuration = self.initial_configuration();
        }
        let was_done = self.root_done(&working.configuration);
        let mut enq = ActionQueue::new();
        let mut timers = Vec::new();
        let mut internal = VecDeque::new();
        let transitioned = {
            let mut out = Outbox {
                enq: &mut enq,
                timers: &mut timers,
                internal: &mut internal,
            };
            let armed = working
                .armed
                .iter()
                .position(|armed| armed.state == timer.state && armed.duration == timer.duration)
                .map(|position| working.armed.remove(position));
            let active = self.active_nodes(&working.configuration);
            let selection = active.contains(&timer.state).then(|| {
                self.node(&timer.state).and_then(|node| match armed {
                    Some(armed) => node
                        .after
                        .get(armed.transition)
                        .map(|(_, transition)| transition.clone()),
                    None => node
                        .after
                        .iter()
                        .find(|(delay, _)| {
                            delay.resolve(&working.context, Some(event)) == Some(timer.duration)
                        })
                        .map(|(_, transition)| transition.clone()),
                })
            });
            let taken = match selection.flatten() {
                Some(transition)
                    if transition.guard_passes_in(
                        &working.configuration,
                        &working.context,
                        event,
                    ) =>
                {
                    let selected = Selected {
                        source: timer.state.clone(),
                        transition,
                    };
                    self.apply(
                        &mut working,
                        std::slice::from_ref(&selected),
                        event,
                        &mut out,
                    );
                    true
                }
                _ => false,
            };
            self.settle(&mut working, event, &mut out) || taken
        };
        self.outcome(working, enq, timers, internal, transitioned, was_done)
    }
}

/// Builder for constructing a [`Machine`].
pub struct MachineBuilder<S: Ord + Clone, E, C: Clone, CMD> {
    nodes: BTreeMap<S, StateNode<S, E, C, CMD>>,
    initial_state: S,
    initial_context: C,
    root_parallel: bool,
    root_entry_actions: Vec<LifecycleFn<C, CMD, E>>,
    root_exit_actions: Vec<LifecycleFn<C, CMD, E>>,
}

impl<S, E, C, CMD> MachineBuilder<S, E, C, CMD>
where
    S: Ord + Clone + std::fmt::Debug + Send + Sync + 'static,
    E: 'static,
    C: Clone + 'static,
    CMD: Send + Sync + 'static,
{
    /// Initialize an empty builder.
    pub fn new(initial_state: S, initial_context: C) -> Self {
        Self {
            nodes: BTreeMap::new(),
            initial_state,
            initial_context,
            root_parallel: false,
            root_entry_actions: Vec::new(),
            root_exit_actions: Vec::new(),
        }
    }

    /// Treat every top-level node as a region of an implicit parallel root.
    /// The declared initial state is then only a placeholder.
    pub fn root_parallel(mut self) -> Self {
        self.root_parallel = true;
        self
    }

    /// Register an action that runs when the machine root is entered, before
    /// the initial state's own entry actions.
    pub fn root_entry(
        mut self,
        action: impl Fn(&mut C, &mut ActionQueue<CMD, E>) + Send + Sync + 'static,
    ) -> Self {
        self.root_entry_actions.push(Arc::new(action));
        self
    }

    /// Register an action that runs when the machine root is exited after
    /// completion, after every active state's exit actions.
    pub fn root_exit(
        mut self,
        action: impl Fn(&mut C, &mut ActionQueue<CMD, E>) + Send + Sync + 'static,
    ) -> Self {
        self.root_exit_actions.push(Arc::new(action));
        self
    }

    /// Add an atomic state node configured through a closure.
    pub fn state(
        mut self,
        id: S,
        configure: impl FnOnce(StateNode<S, E, C, CMD>) -> StateNode<S, E, C, CMD>,
    ) -> Self {
        let node = configure(StateNode::atomic(id));
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a compound state node configured through a closure.
    pub fn compound(
        mut self,
        id: S,
        initial: S,
        configure: impl FnOnce(StateNode<S, E, C, CMD>) -> StateNode<S, E, C, CMD>,
    ) -> Self {
        let node = configure(StateNode::compound(id, initial));
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a parallel state node configured through a closure. Every child
    /// with this node as parent is entered together.
    pub fn parallel(
        mut self,
        id: S,
        configure: impl FnOnce(StateNode<S, E, C, CMD>) -> StateNode<S, E, C, CMD>,
    ) -> Self {
        let node = configure(StateNode::parallel(id));
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a shallow history node configured through a closure.
    pub fn history(
        mut self,
        id: S,
        fallback: S,
        configure: impl FnOnce(StateNode<S, E, C, CMD>) -> StateNode<S, E, C, CMD>,
    ) -> Self {
        let node = configure(StateNode::history(id, false, fallback));
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a deep history node configured through a closure.
    pub fn deep_history(
        mut self,
        id: S,
        fallback: S,
        configure: impl FnOnce(StateNode<S, E, C, CMD>) -> StateNode<S, E, C, CMD>,
    ) -> Self {
        let node = configure(StateNode::history(id, true, fallback));
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a final terminal state node configured through a closure.
    pub fn final_state(
        mut self,
        id: S,
        configure: impl FnOnce(StateNode<S, E, C, CMD>) -> StateNode<S, E, C, CMD>,
    ) -> Self {
        let node = configure(StateNode::final_state(id));
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Add a pre-constructed state node directly.
    pub fn node(mut self, node: StateNode<S, E, C, CMD>) -> Self {
        self.nodes.insert(node.id.clone(), node);
        self
    }

    /// Build the finalized [`Machine`].
    #[must_use]
    pub fn build(self) -> Machine<S, E, C, CMD> {
        Machine {
            nodes: self.nodes,
            initial_state: self.initial_state,
            initial_context: self.initial_context,
            root_parallel: self.root_parallel,
            root_entry_actions: self.root_entry_actions,
            root_exit_actions: self.root_exit_actions,
        }
    }
}
