---- MODULE flow2_concurrency ----
EXTENDS Naturals

VARIABLES scope_state, fiber_state, deadline_hit, finalizers_ran, straggler_named, done

vars == <<scope_state, fiber_state, deadline_hit, finalizers_ran, straggler_named, done>>

Init ==
    /\ scope_state = "ScopeOpen"
    /\ fiber_state = "Created"
    /\ deadline_hit = FALSE
    /\ finalizers_ran = 0
    /\ straggler_named = FALSE
    /\ done = FALSE

ForkChildFiber ==
    /\ scope_state = "ScopeOpen"
    /\ fiber_state = "Created"
    /\ fiber_state' = "Running"
    /\ UNCHANGED <<scope_state, deadline_hit, finalizers_ran, straggler_named, done>>

FiberNormalComplete ==
    /\ fiber_state = "Running"
    /\ ~deadline_hit
    /\ fiber_state' = "Completed"
    /\ UNCHANGED <<scope_state, deadline_hit, finalizers_ran, straggler_named, done>>

ScopeCloseNormal ==
    /\ scope_state = "ScopeOpen"
    /\ fiber_state = "Completed"
    /\ scope_state' = "Closing"
    /\ UNCHANGED <<fiber_state, deadline_hit, finalizers_ran, straggler_named, done>>

DeadlineTrip ==
    /\ fiber_state = "Running"
    /\ ~deadline_hit
    /\ deadline_hit' = TRUE
    /\ UNCHANGED <<scope_state, fiber_state, finalizers_ran, straggler_named, done>>

InterruptOnDeadline ==
    /\ deadline_hit
    /\ fiber_state = "Running"
    /\ fiber_state' = "InterruptSignalled"
    /\ scope_state' = "Closing"
    /\ UNCHANGED <<deadline_hit, finalizers_ran, straggler_named, done>>

FiberResponsiveInterruptYield ==
    /\ fiber_state = "InterruptSignalled"
    /\ fiber_state' = "Interrupted"
    /\ UNCHANGED <<scope_state, deadline_hit, finalizers_ran, straggler_named, done>>

FiberUnresponsiveStraggler ==
    /\ fiber_state = "InterruptSignalled"
    /\ fiber_state' = "StragglerQuarantined"
    /\ straggler_named' = TRUE
    /\ UNCHANGED <<scope_state, deadline_hit, finalizers_ran, done>>

RunLIFOFinalizers ==
    /\ scope_state = "Closing"
    /\ fiber_state \in {"Interrupted", "StragglerQuarantined", "Completed"}
    /\ finalizers_ran < 2
    /\ finalizers_ran' = finalizers_ran + 1
    /\ UNCHANGED <<scope_state, fiber_state, deadline_hit, straggler_named, done>>

FinishScopeClose ==
    /\ scope_state = "Closing"
    /\ finalizers_ran = 2
    /\ scope_state' = "ScopeClosed"
    /\ done' = TRUE
    /\ UNCHANGED <<fiber_state, deadline_hit, finalizers_ran, straggler_named>>

TerminalDone ==
    /\ done
    /\ UNCHANGED vars

Next ==
    \/ ForkChildFiber
    \/ FiberNormalComplete
    \/ ScopeCloseNormal
    \/ DeadlineTrip
    \/ InterruptOnDeadline
    \/ FiberResponsiveInterruptYield
    \/ FiberUnresponsiveStraggler
    \/ RunLIFOFinalizers
    \/ FinishScopeClose
    \/ TerminalDone

NoHangingOnStraggler ==
    scope_state = "ScopeClosed" => (deadline_hit => (fiber_state = "Interrupted" \/ straggler_named))

AllFinalizersRunBeforeClose ==
    scope_state = "ScopeClosed" => finalizers_ran = 2

==============================
