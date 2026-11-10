use crate::action::ActionQueue;
use crate::configuration::Configuration;
use crate::state_node::StateNode;
use crate::transition::Transition;
use crate::types::{MachineState, StateNodeType, StepOutcome, TimerFired, TimerIntent};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

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
    /// inside the source keeps the source active. Everything else exits below
    /// the deepest shared ancestor.
    fn transition_domain(&self, source: &S, target: &S) -> Option<S> {
        if target == source {
            return self.parent_of(source);
        }
        if self.is_strict_descendant(target, source) {
            return self
                .parallel_target_region(source, target)
                .or_else(|| Some(source.clone()));
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
                matches!(active.as_slice(), [child] if self.node_done(config, child))
            }
            StateNodeType::Parallel => {
                let children = self.children(id);
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
                for child in self.children(id) {
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
                if let Some(fallback) = node.initial.clone() {
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
                let found = if transient {
                    node.always
                        .iter()
                        .find(|transition| transition.guard_passes(ctx, event))
                } else {
                    node.transitions
                        .iter()
                        .find(|transition| transition.is_enabled(ctx, event))
                        .or_else(|| {
                            node.after
                                .iter()
                                .map(|(_, transition)| transition)
                                .find(|transition| transition.is_enabled(ctx, event))
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
                    && self.node(id).is_some_and(|node| !node.on_done.is_empty())
                    && self.node_done(&state.configuration, id)
            })
            .collect();
        candidates.sort_by(|a, b| self.depth(b).cmp(&self.depth(a)).then_with(|| a.cmp(b)));

        for id in candidates {
            if let Some(node) = self.node(&id) {
                for transition in &node.on_done {
                    if transition.guard_passes(&state.context, event) {
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

    /// Record history for every compound state this step exits.
    fn record_history(
        &self,
        before: &Configuration<S>,
        exited: &BTreeSet<S>,
        history: &mut BTreeMap<S, Configuration<S>>,
    ) {
        for node_id in exited {
            if self.node_type(node_id) != StateNodeType::Compound {
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
        for (duration, _) in &node.after {
            out.timers.push(TimerIntent::Start {
                state: id.clone(),
                duration: *duration,
            });
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
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        if entered.contains(id) {
            return;
        }
        self.enter_self(id, state, entered, out);
        let Some(node) = self.node(id) else {
            return;
        };
        match node.node_type {
            StateNodeType::Compound => {
                if let Some(initial) = node.initial.clone() {
                    self.enter_target(&initial, state, entered, out);
                }
            }
            StateNodeType::Parallel => {
                for child in self.children(id) {
                    self.enter_target(&child, state, entered, out);
                }
            }
            _ => {}
        }
    }

    /// Enter a target, resolving a history node to its record or fallback.
    fn enter_target(
        &self,
        id: &S,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
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
                                    self.enter_node(&cursor, state, entered, out);
                                }
                            }
                        } else {
                            self.restore_deep(&recorded, id, state, entered, out);
                        }
                    }
                    None => {
                        if let Some(fallback) = node.initial.clone()
                            && fallback != *id
                        {
                            self.enter_target(&fallback, state, entered, out);
                        }
                    }
                }
            }
            _ => self.enter_node(id, state, entered, out),
        }
    }

    /// Re-enter a remembered leaf set, running entry actions from the top down.
    fn restore_deep(
        &self,
        recorded: &Configuration<S>,
        history_id: &S,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
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
            self.enter_self(&node, state, entered, out);
        }
    }

    /// Enter the path from the transition domain down to the target.
    fn enter_from_domain(
        &self,
        target: &S,
        domain: Option<&S>,
        state: &mut MachineState<S, C>,
        entered: &mut BTreeSet<S>,
        out: &mut Outbox<'_, S, CMD, E>,
    ) {
        let mut path = self.path_from_root(target);
        if let Some(domain) = domain
            && let Some(position) = path.iter().position(|node| node == domain)
        {
            path.drain(..=position);
        }
        let Some((last, ancestors)) = path.split_last() else {
            return;
        };
        for ancestor in ancestors {
            self.enter_self(ancestor, state, entered, out);
        }
        self.enter_target(last, state, entered, out);
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
            let domain = self.transition_domain(&selection.source, &selection.transition.target);
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
        ordered.sort_by(|a, b| self.depth(b).cmp(&self.depth(a)).then_with(|| a.cmp(b)));
        for id in &ordered {
            if let Some(node) = self.node(id) {
                for action in &node.exit_actions {
                    action(&mut state.context, out.enq);
                }
                for (duration, _) in &node.after {
                    out.timers.push(TimerIntent::Cancel {
                        state: id.clone(),
                        duration: *duration,
                    });
                }
            }
        }
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
            self.enter_from_domain(
                &selection.transition.target,
                domain.as_ref(),
                state,
                &mut entered,
                out,
            );
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
        state: MachineState<S, C>,
        mut enq: ActionQueue<CMD, E>,
        timers: Vec<TimerIntent<S>>,
        internal: VecDeque<E>,
        transitioned: bool,
    ) -> StepOutcome<S, C, CMD, E> {
        let current_state = state
            .configuration
            .primary()
            .cloned()
            .unwrap_or_else(|| self.initial_state.clone());
        let done = self.root_done(&state.configuration);
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
        let mut state = MachineState::new(Configuration::new(), context.clone());
        let mut enq = ActionQueue::new();
        let mut timers = Vec::new();
        let mut internal = VecDeque::new();
        {
            let mut out = Outbox {
                enq: &mut enq,
                timers: &mut timers,
                internal: &mut internal,
            };
            let mut entered = BTreeSet::new();
            self.enter_node(&self.initial_state, &mut state, &mut entered, &mut out);
            out.collect_raised();
        }
        self.outcome(state, enq, timers, internal, false)
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
        self.outcome(working, enq, timers, internal, transitioned)
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
        let mut enq = ActionQueue::new();
        let mut timers = Vec::new();
        let mut internal = VecDeque::new();
        let transitioned = {
            let mut out = Outbox {
                enq: &mut enq,
                timers: &mut timers,
                internal: &mut internal,
            };
            let active = self.active_nodes(&working.configuration);
            let selection = active.contains(&timer.state).then(|| {
                self.node(&timer.state).and_then(|node| {
                    node.after
                        .iter()
                        .find(|(duration, _)| *duration == timer.duration)
                        .map(|(_, transition)| transition.clone())
                })
            });
            let taken = match selection.flatten() {
                Some(transition) if transition.guard_passes(&working.context, event) => {
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
        self.outcome(working, enq, timers, internal, transitioned)
    }
}

/// Builder for constructing a [`Machine`].
pub struct MachineBuilder<S: Ord + Clone, E, C: Clone, CMD> {
    nodes: BTreeMap<S, StateNode<S, E, C, CMD>>,
    initial_state: S,
    initial_context: C,
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
        }
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
        }
    }
}
