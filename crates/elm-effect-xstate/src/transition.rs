use crate::action::ActionQueue;
use std::sync::Arc;

pub type GuardFn<C, E> = Arc<dyn Fn(&C, &E) -> bool + Send + Sync>;
pub type ActionFn<C, E, CMD> = Arc<dyn Fn(&mut C, &E, &mut ActionQueue<CMD, E>) + Send + Sync>;
pub type EventMatcherFn<E> = Arc<dyn Fn(&E) -> bool + Send + Sync>;

/// Declarative state transition rule.
#[derive(Clone)]
pub struct Transition<S, E, C, CMD> {
    pub(crate) target: S,
    pub(crate) matcher: Option<EventMatcherFn<E>>,
    pub(crate) guard: Option<GuardFn<C, E>>,
    pub(crate) actions: Vec<ActionFn<C, E, CMD>>,
}

impl<S: Clone, E: 'static, C: 'static, CMD: Send + Sync + 'static> Transition<S, E, C, CMD> {
    /// Create a new transition directed to `target`.
    pub fn target(target: S) -> Self {
        Self {
            target,
            matcher: None,
            guard: None,
            actions: Vec::new(),
        }
    }

    /// Match specific events that trigger this transition.
    pub fn on(mut self, predicate: impl Fn(&E) -> bool + Send + Sync + 'static) -> Self {
        self.matcher = Some(Arc::new(predicate));
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

    /// Evaluate whether this transition is enabled for `event` and `ctx`.
    #[must_use]
    pub fn is_enabled(&self, ctx: &C, event: &E) -> bool {
        if let Some(ref matcher) = self.matcher
            && !matcher(event)
        {
            return false;
        }
        if let Some(ref guard) = self.guard
            && !guard(ctx, event)
        {
            return false;
        }
        true
    }
}
