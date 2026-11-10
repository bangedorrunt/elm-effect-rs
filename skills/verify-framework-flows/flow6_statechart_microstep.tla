---- MODULE flow6_statechart_microstep ----
EXTENDS Naturals

VARIABLES phase, active_state, context_val, microstep_count, exited_before_entered, done

vars == <<phase, active_state, context_val, microstep_count, exited_before_entered, done>>

Init ==
    /\ phase = "Idle"
    /\ active_state = "Off"
    /\ context_val = 0
    /\ microstep_count = 0
    /\ exited_before_entered = TRUE
    /\ done = FALSE

ReceiveTurnOn ==
    /\ phase = "Idle"
    /\ active_state = "Off"
    /\ ~done
    /\ phase' = "ExitSource"
    /\ exited_before_entered' = TRUE
    /\ UNCHANGED <<active_state, context_val, microstep_count, done>>

ExitSourceState ==
    /\ phase = "ExitSource"
    /\ active_state = "Off"
    /\ phase' = "TransitionAction"
    /\ UNCHANGED <<active_state, context_val, microstep_count, exited_before_entered, done>>

ExecuteTransitionActions ==
    /\ phase = "TransitionAction"
    /\ context_val' = context_val + 1
    /\ phase' = "EnterTarget"
    /\ UNCHANGED <<active_state, microstep_count, exited_before_entered, done>>

EnterTargetState ==
    /\ phase = "EnterTarget"
    /\ active_state' = "Starting"
    /\ phase' = "CheckAlways"
    /\ UNCHANGED <<context_val, microstep_count, exited_before_entered, done>>

CascadeAlwaysMicrostep ==
    /\ phase = "CheckAlways"
    /\ active_state = "Starting"
    /\ microstep_count < 2
    /\ microstep_count' = microstep_count + 1
    /\ context_val' = context_val + 10
    /\ active_state' = "Running_Idle"
    /\ phase' = "CheckAlways"
    /\ UNCHANGED <<exited_before_entered, done>>

SettleQuiescence ==
    /\ phase = "CheckAlways"
    /\ active_state = "Running_Idle"
    /\ phase' = "Quiescent"
    /\ done' = TRUE
    /\ UNCHANGED <<active_state, context_val, microstep_count, exited_before_entered>>

TerminalStep ==
    /\ done
    /\ UNCHANGED vars

Next ==
    \/ ReceiveTurnOn
    \/ ExitSourceState
    \/ ExecuteTransitionActions
    \/ EnterTargetState
    \/ CascadeAlwaysMicrostep
    \/ SettleQuiescence
    \/ TerminalStep

QuiescenceReached ==
    microstep_count <= 2

StrictOrderExitedBeforeEntered ==
    exited_before_entered = TRUE

ValidActiveConfiguration ==
    active_state \in {"Off", "Starting", "Running_Idle"}

NoDeadlock ==
    ~done => phase \in {"Idle", "ExitSource", "TransitionAction", "EnterTarget", "CheckAlways"}

==============================
