use crate::action::ActionQueue;
use crate::configuration::Configuration;
use std::sync::Arc;

/// A pure guard over context and the event being processed.
pub type GuardFn<C, E> = Arc<dyn Fn(&C, &E) -> bool + Send + Sync>;
type ConfigurationGuardFn<S> = Arc<dyn Fn(&Configuration<S>) -> bool + Send + Sync>;
/// An action that may read the event, mutate context, and enqueue work.
pub type ActionFn<C, E, CMD> = Arc<dyn Fn(&mut C, &E, &mut ActionQueue<CMD, E>) + Send + Sync>;
/// A predicate that decides whether an event selects this transition.
pub type EventMatcherFn<E> = Arc<dyn Fn(&E) -> bool + Send + Sync>;
type SelectionPriorityFn<E> = Arc<dyn Fn(&E) -> (u8, usize) + Send + Sync>;

/// Declarative state transition rule.
#[derive(Clone)]
pub struct Transition<S, E, C, CMD> {
    pub(crate) target: S,
    pub(crate) targets: Vec<S>,
    pub(crate) targetless: bool,
    pub(crate) reenter: bool,
    pub(crate) matcher: Option<EventMatcherFn<E>>,
    selection_priority: Option<SelectionPriorityFn<E>>,
    pub(crate) guard: Option<GuardFn<C, E>>,
    configuration_guard: Option<ConfigurationGuardFn<S>>,
    pub(crate) actions: Vec<ActionFn<C, E, CMD>>,
    pub(crate) label: Option<String>,
}

impl<S: Clone + 'static, E: 'static, C: 'static, CMD: Send + Sync + 'static>
    Transition<S, E, C, CMD>
{
    /// Create a new transition directed to `target`.
    pub fn target(target: S) -> Self {
        let targets = vec![target.clone()];
        Self {
            target,
            targets,
            targetless: false,
            reenter: false,
            matcher: None,
            selection_priority: None,
            guard: None,
            configuration_guard: None,
            actions: Vec::new(),
            label: None,
        }
    }

    /// Direct this transition at several states, as XState does with a target
    /// array. The first target is the primary destination.
    pub fn targets(mut self, targets: impl IntoIterator<Item = S>) -> Self {
        let targets: Vec<S> = targets.into_iter().collect();
        if let Some(first) = targets.first() {
            self.target = first.clone();
        }
        self.targets = targets;
        self
    }

    /// Mark this transition targetless. It runs its actions and leaves the
    /// active configuration untouched; the stored target is a placeholder.
    pub fn targetless(mut self) -> Self {
        self.targetless = true;
        self
    }

    /// Exit and re-enter the source when the target is the source itself or a
    /// descendant. XState defaults to `false`, which preserves the source.
    pub fn reenter(mut self) -> Self {
        self.reenter = true;
        self
    }

    /// Set re-entry from a schema value.
    pub fn reenter_with(mut self, reenter: bool) -> Self {
        self.reenter = reenter;
        self
    }

    /// Whether this transition exits and re-enters its source.
    #[must_use]
    pub fn is_reentering(&self) -> bool {
        self.reenter
    }

    /// Match specific events that trigger this transition.
    pub fn on(mut self, predicate: impl Fn(&E) -> bool + Send + Sync + 'static) -> Self {
        self.matcher = Some(Arc::new(predicate));
        self
    }

    /// Rank matching transitions before declaration order, as XState does for
    /// exact and wildcard event descriptors. A lower rank is selected first.
    pub(crate) fn with_selection_priority(
        mut self,
        priority: impl Fn(&E) -> (u8, usize) + Send + Sync + 'static,
    ) -> Self {
        self.selection_priority = Some(Arc::new(priority));
        self
    }

    pub(crate) fn selection_priority_for(&self, event: &E) -> (u8, usize) {
        self.selection_priority
            .as_ref()
            .map_or((0, 0), |priority| priority(event))
    }

    /// Give the edge the name a diagram should print.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Add a conditional guard that must evaluate to `true` for the transition to take effect.
    pub fn guard(mut self, predicate: impl Fn(&C, &E) -> bool + Send + Sync + 'static) -> Self {
        self.guard = Some(Arc::new(predicate));
        self
    }

    /// Add a guard over the configuration active when the transition is selected.
    pub fn configuration_guard(
        mut self,
        predicate: impl Fn(&Configuration<S>) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.configuration_guard = Some(Arc::new(predicate));
        self
    }

    /// Mutate extended context or enqueue actions during transition execution.
    pub fn action(
        mut self,
        action: impl Fn(&mut C, &E, &mut ActionQueue<CMD, E>) + Send + Sync + 'static,
    ) -> Self {
        self.actions.push(Arc::new(action));
        self
    }

    /// Convenience helper to mutate context directly (XState `assign`).
    pub fn assign(mut self, updater: impl Fn(&mut C, &E) + Send + Sync + 'static) -> Self {
        self.actions.push(Arc::new(move |ctx, event, _| {
            updater(ctx, event);
        }));
        self
    }

    /// Convenience helper to dispatch a command during the transition.
    pub fn exec(mut self, cmd: CMD) -> Self
    where
        CMD: Clone,
    {
        self.actions.push(Arc::new(move |_, _, enq| {
            enq.exec(cmd.clone());
        }));
        self
    }

    /// Whether the event matches this transition.
    #[must_use]
    pub fn matches(&self, event: &E) -> bool {
        self.matcher.as_ref().is_none_or(|matcher| matcher(event))
    }

    /// Whether the guard admits this event and context.
    #[must_use]
    pub fn guard_passes(&self, ctx: &C, event: &E) -> bool {
        self.guard.as_ref().is_none_or(|guard| guard(ctx, event))
    }

    /// Whether both the context guard and configuration guard pass.
    #[must_use]
    pub(crate) fn guard_passes_in(
        &self,
        configuration: &Configuration<S>,
        ctx: &C,
        event: &E,
    ) -> bool {
        self.configuration_guard
            .as_ref()
            .is_none_or(|guard| guard(configuration))
            && self.guard_passes(ctx, event)
    }

    /// Whether this transition is enabled in the given configuration.
    #[must_use]
    pub fn is_enabled(&self, configuration: &Configuration<S>, ctx: &C, event: &E) -> bool {
        self.matches(event) && self.guard_passes_in(configuration, ctx, event)
    }

    /// Every state this transition enters, in declaration order.
    #[must_use]
    pub fn target_states(&self) -> &[S] {
        &self.targets
    }

    /// The first state this transition enters. A targetless transition carries
    /// its declaring state here as a placeholder and never enters it.
    #[must_use]
    pub fn destination(&self) -> &S {
        self.targets.first().unwrap_or(&self.target)
    }

    /// Whether this transition runs without exiting or entering any state.
    #[must_use]
    pub fn is_targetless(&self) -> bool {
        self.targetless
    }

    /// The diagram label, when one was set.
    #[must_use]
    pub fn edge_label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}
