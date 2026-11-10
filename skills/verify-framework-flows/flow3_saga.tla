---- MODULE flow3_saga ----
EXTENDS Naturals

VARIABLES saga_phase, steps_completed, steps_compensated, current_gen, lease_holder, done

vars == <<saga_phase, steps_completed, steps_compensated, current_gen, lease_holder, done>>

Init ==
    /\ saga_phase = "Prepared"
    /\ steps_completed = 0
    /\ steps_compensated = 0
    /\ current_gen = 1
    /\ lease_holder = "None"
    /\ done = FALSE

ClaimLease ==
    /\ saga_phase = "Prepared"
    /\ lease_holder = "None"
    /\ lease_holder' = "AgentA"
    /\ saga_phase' = "Executing"
    /\ UNCHANGED <<steps_completed, steps_compensated, current_gen, done>>

ExecuteStep1Success ==
    /\ saga_phase = "Executing"
    /\ lease_holder = "AgentA"
    /\ steps_completed = 0
    /\ steps_completed' = 1
    /\ UNCHANGED <<saga_phase, steps_compensated, current_gen, lease_holder, done>>

ExecuteStep2Failure ==
    /\ saga_phase = "Executing"
    /\ lease_holder = "AgentA"
    /\ steps_completed = 1
    /\ saga_phase' = "Compensating"
    /\ UNCHANGED <<steps_completed, steps_compensated, current_gen, lease_holder, done>>

CompensateStep1 ==
    /\ saga_phase = "Compensating"
    /\ lease_holder = "AgentA"
    /\ steps_compensated = 0
    /\ steps_compensated' = 1
    /\ UNCHANGED <<saga_phase, steps_completed, current_gen, lease_holder, done>>

SettleSagaFailure ==
    /\ saga_phase = "Compensating"
    /\ steps_compensated = steps_completed
    /\ saga_phase' = "SettledFailed"
    /\ done' = TRUE
    /\ UNCHANGED <<steps_completed, steps_compensated, current_gen, lease_holder>>

StaleOwnerSettleAttempt ==
    /\ saga_phase = "Compensating"
    /\ UNCHANGED vars

TerminalDone ==
    /\ done
    /\ UNCHANGED vars

Next ==
    \/ ClaimLease
    \/ ExecuteStep1Success
    \/ ExecuteStep2Failure
    \/ CompensateStep1
    \/ SettleSagaFailure
    \/ StaleOwnerSettleAttempt
    \/ TerminalDone

RollbackMatchesCompleted ==
    saga_phase = "SettledFailed" => steps_compensated = steps_completed

NoExcessCompensation ==
    steps_compensated <= steps_completed

==============================
