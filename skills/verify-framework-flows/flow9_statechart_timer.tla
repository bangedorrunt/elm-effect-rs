---- MODULE flow9_statechart_timer ----
EXTENDS Naturals

VARIABLES machine_state, timer_status, clock_ticks, target_reached, done

vars == <<machine_state, timer_status, clock_ticks, target_reached, done>>

Init ==
    /\ machine_state = "Waiting"
    /\ timer_status = "Active"
    /\ clock_ticks = 0
    /\ target_reached = "None"
    /\ done = FALSE

VirtualClockAdvances ==
    /\ machine_state = "Waiting"
    /\ timer_status = "Active"
    /\ clock_ticks < 2
    /\ clock_ticks' = clock_ticks + 1
    /\ UNCHANGED <<machine_state, timer_status, target_reached, done>>

TimerFiresOnDwellMet ==
    /\ machine_state = "Waiting"
    /\ timer_status = "Active"
    /\ clock_ticks = 2
    /\ machine_state' = "TimedOut"
    /\ timer_status' = "Fired"
    /\ target_reached' = "TimeoutTarget"
    /\ done' = TRUE
    /\ UNCHANGED clock_ticks

EarlyExternalEventArrives ==
    /\ machine_state = "Waiting"
    /\ timer_status = "Active"
    /\ clock_ticks < 2
    /\ machine_state' = "Interrupted"
    /\ timer_status' = "Cancelled"
    /\ target_reached' = "ExternalTarget"
    /\ done' = TRUE
    /\ UNCHANGED clock_ticks

StaleTimerDiscarded ==
    /\ machine_state \in {"Interrupted", "TimedOut"}
    /\ timer_status \in {"Cancelled", "Fired"}
    /\ done
    /\ UNCHANGED vars

TerminalStep ==
    /\ done
    /\ UNCHANGED vars

Next ==
    \/ VirtualClockAdvances
    \/ TimerFiresOnDwellMet
    \/ EarlyExternalEventArrives
    \/ StaleTimerDiscarded
    \/ TerminalStep

TimerInvalidatedOnEarlyExit ==
    machine_state = "Interrupted" => timer_status = "Cancelled"

NoStaleTimerWakeup ==
    target_reached = "TimeoutTarget" => clock_ticks = 2

ExclusiveTargetResolution ==
    target_reached \in {"None", "TimeoutTarget", "ExternalTarget"}

NoDeadlock ==
    ~done => machine_state = "Waiting"

==============================
