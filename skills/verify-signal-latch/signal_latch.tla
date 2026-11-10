---- MODULE signal_latch ----
EXTENDS Naturals

VARIABLES runner_pc, phase, signals, outcome, signal_sent, cancel_sent, cancelled_in_flight

vars == <<runner_pc, phase, signals, outcome, signal_sent, cancel_sent, cancelled_in_flight>>

Init ==
    /\ runner_pc = "CheckPre"
    /\ phase = "Running"
    /\ signals = 0
    /\ outcome = "None"
    /\ signal_sent = FALSE
    /\ cancel_sent = FALSE
    /\ cancelled_in_flight = FALSE

RunnerCheckPre ==
    /\ runner_pc = "CheckPre"
    /\ IF phase = "Cancelled"
       THEN /\ outcome' = "TerminalError"
            /\ runner_pc' = "Done"
            /\ UNCHANGED <<phase, signals, signal_sent, cancel_sent, cancelled_in_flight>>
       ELSE IF signals > 0
            THEN /\ outcome' = "Signalled"
                 /\ signals' = 0
                 /\ runner_pc' = "Done"
                 /\ UNCHANGED <<phase, signal_sent, cancel_sent, cancelled_in_flight>>
            ELSE /\ runner_pc' = "CommitPark"
                 /\ UNCHANGED <<phase, signals, outcome, signal_sent, cancel_sent, cancelled_in_flight>>

RunnerCommitPark ==
    /\ runner_pc = "CommitPark"
    /\ IF phase = "Cancelled"
       THEN /\ outcome' = "TerminalError"
            /\ runner_pc' = "Done"
            /\ UNCHANGED <<phase, signals, signal_sent, cancel_sent, cancelled_in_flight>>
       ELSE /\ phase' = "Parked"
            /\ runner_pc' = "GapCheck"
            /\ UNCHANGED <<signals, outcome, signal_sent, cancel_sent, cancelled_in_flight>>

RunnerGapCheck ==
    /\ runner_pc = "GapCheck"
    /\ IF signals = 0
       THEN /\ outcome' = "Parked"
            /\ runner_pc' = "Done"
            /\ UNCHANGED <<phase, signals, signal_sent, cancel_sent, cancelled_in_flight>>
       ELSE /\ IF phase = "Cancelled"
               THEN /\ outcome' = "TerminalError"
                    /\ runner_pc' = "Done"
                    /\ UNCHANGED <<phase, signals, signal_sent, cancel_sent, cancelled_in_flight>>
               ELSE /\ phase' = "Running"
                    /\ signals' = 0
                    /\ outcome' = "Signalled"
                    /\ runner_pc' = "Done"
                    /\ UNCHANGED <<signal_sent, cancel_sent, cancelled_in_flight>>

SignallerSignal ==
    /\ ~signal_sent
    /\ runner_pc /= "Done"
    /\ signal_sent' = TRUE
    /\ IF phase = "Cancelled"
       THEN UNCHANGED <<phase, signals>>
       ELSE /\ signals' = 1
            /\ IF phase = "Parked"
               THEN phase' = "Running"
               ELSE UNCHANGED phase
    /\ UNCHANGED <<runner_pc, outcome, cancel_sent, cancelled_in_flight>>

CancellerCancel ==
    /\ ~cancel_sent
    /\ runner_pc /= "Done"
    /\ cancel_sent' = TRUE
    /\ phase' = "Cancelled"
    /\ cancelled_in_flight' = TRUE
    /\ UNCHANGED <<runner_pc, signals, outcome, signal_sent>>

TerminalDone ==
    /\ runner_pc = "Done"
    /\ UNCHANGED vars

Next ==
    \/ RunnerCheckPre
    \/ RunnerCommitPark
    \/ RunnerGapCheck
    \/ SignallerSignal
    \/ CancellerCancel
    \/ TerminalDone

NoResurrectOnSignal ==
    cancelled_in_flight => outcome /= "Signalled"

NoLostWakeup ==
    (signal_sent /\ ~cancelled_in_flight) => outcome /= "Parked"

LiveWhenSignalled ==
    outcome = "Signalled" => phase = "Running"

==============================
