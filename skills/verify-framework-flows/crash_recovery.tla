---- MODULE crash_recovery ----
EXTENDS Naturals

VARIABLES phase, message, intent, provider, outcome, pending, in_flight, runs, replays

vars == <<phase, message, intent, provider, outcome, pending, in_flight, runs, replays>>

Init ==
    /\ phase = "Boot"
    /\ message = FALSE
    /\ intent = FALSE
    /\ provider = FALSE
    /\ outcome = FALSE
    /\ pending = FALSE
    /\ in_flight = FALSE
    /\ runs = 0
    /\ replays = 0

AppendMessage ==
    /\ phase = "Boot"
    /\ phase' = "Message"
    /\ message' = TRUE
    /\ UNCHANGED <<intent, provider, outcome, pending, in_flight, runs, replays>>

AppendIntent ==
    /\ phase = "Message"
    /\ phase' = "Intent"
    /\ intent' = TRUE
    /\ UNCHANGED <<message, provider, outcome, pending, in_flight, runs, replays>>

RunProvider ==
    /\ phase = "Intent"
    /\ phase' = "Run"
    /\ provider' = TRUE
    /\ runs' = runs + 1
    /\ UNCHANGED <<message, intent, outcome, pending, in_flight, replays>>

AppendOutcome ==
    /\ phase = "Run"
    /\ phase' = "Done"
    /\ outcome' = TRUE
    /\ UNCHANGED <<message, intent, provider, pending, in_flight, runs, replays>>

CrashAfterMessage ==
    /\ phase = "Message"
    /\ phase' = "Crashed"
    /\ UNCHANGED <<message, intent, provider, outcome, pending, in_flight, runs, replays>>

CrashAfterIntent ==
    /\ phase = "Intent"
    /\ phase' = "Crashed"
    /\ UNCHANGED <<message, intent, provider, outcome, pending, in_flight, runs, replays>>

CrashAfterRun ==
    /\ phase = "Run"
    /\ phase' = "Crashed"
    /\ UNCHANGED <<message, intent, provider, outcome, pending, in_flight, runs, replays>>

Recover ==
    /\ phase = "Crashed"
    /\ phase' = "Recovered"
    /\ pending' = (message /\ ~provider)
    /\ in_flight' = (intent /\ provider /\ ~outcome)
    /\ UNCHANGED <<message, intent, provider, outcome, runs, replays>>

ResumePending ==
    /\ phase = "Recovered"
    /\ pending
    /\ phase' = "Done"
    /\ pending' = FALSE
    /\ intent' = TRUE
    /\ provider' = TRUE
    /\ runs' = runs + 1
    /\ UNCHANGED <<message, outcome, in_flight, replays>>

ReconcileInFlight ==
    /\ phase = "Recovered"
    /\ in_flight
    /\ phase' = "Done"
    /\ in_flight' = FALSE
    /\ replays' = replays + 1
    /\ UNCHANGED <<message, intent, provider, outcome, pending, runs>>

Finish ==
    /\ phase = "Recovered"
    /\ ~pending
    /\ ~in_flight
    /\ phase' = "Done"
    /\ UNCHANGED <<message, intent, provider, outcome, pending, in_flight, runs, replays>>

DoneStutter ==
    /\ phase = "Done"
    /\ UNCHANGED vars

Next ==
    \/ AppendMessage
    \/ AppendIntent
    \/ RunProvider
    \/ AppendOutcome
    \/ CrashAfterMessage
    \/ CrashAfterIntent
    \/ CrashAfterRun
    \/ Recover
    \/ ResumePending
    \/ ReconcileInFlight
    \/ Finish
    \/ DoneStutter

ProviderNeedsIntent == provider => intent
OutcomeNeedsProvider == outcome => provider
PendingNeverRan == pending => ~provider
InFlightNotReplayed == in_flight => replays = 0
AtMostOneRun == runs <= 1
AtMostOneReconcile == replays <= 1
CrashedHasMessage == phase = "Crashed" => message
RecoveredClassifies == phase = "Recovered" => pending \/ in_flight \/ outcome
DoneSettled == phase = "Done" => outcome \/ runs = 1 \/ replays = 1

==============================
