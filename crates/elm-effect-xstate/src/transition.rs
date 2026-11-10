use crate::action::ActionQueue;
use std::sync::Arc;

/// A pure guard over context and the event being processed.
pub type GuardFn<C, E> = Arc<dyn Fn(&C, &E) -> bool + Send + Sync>;
/// An action that may read the event, mutate context, and enqueue work.
pub type ActionFn<C, E, CMD> = Arc<dyn Fn(&mut C, &E, &mut ActionQueue<CMD, E>) + Send + Sync>;
/// A predicate that decides whether an event selects this transition.
pub type EventMatcherFn<E> = Arc<dyn Fn(&E) -> bool + Send + Sync>;

/// Declarative state transition rule.
#[derive(Clone)]
pub struct Transition<S, E, C, CMD> {
    pub(crate) target: S,
    pub(crate) matcher: Option<EventMatcherFn<E>>,
    pub(crate) guard: Option<GuardFn<C, E>>,
    pub(crate) actions: Vec<ActionFn<C, E, CMD>>,
    pub(crate) label: Option<String>,
}

impl<S: Clone, E: 'static, C: 'static, CMD: Send + Sync + 'static> Transition<S, E, C, CMD> {
    /// Create a new transition directed to `target`.
    pub fn target(target: S) -> Self {
        Self {
            target,
            matcher: None,
            guard: None,
            actions: Vec::new(),
            label: None,
        }
    }

    /// Match specific events that trigger this transition.
    pub fn on(mut self, predicate: impl Fn(&E) -> bool + Send + Sync + 'static) -> Self {
        self.matcher = Some(Arc::new(predicate));
        self
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

    /// Whether this transition is enabled: the event matches and the guard passes.
    #[must_use]
    pub fn is_enabled(&self, ctx: &C, event: &E) -> bool {
        self.matches(event) && self.guard_passes(ctx, event)
    }

    /// The state this transition enters.
    #[must_use]
    pub fn destination(&self) -> &S {
        &self.target
    }

    /// The diagram label, when one was set.
    #[must_use]
    pub fn edge_label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}
