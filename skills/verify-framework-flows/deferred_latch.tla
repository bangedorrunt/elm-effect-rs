---- MODULE deferred_latch ----
EXTENDS Naturals

MaxWaiters == 2

VARIABLES phase, value, waiters, body_runs, completed

vars == <<phase, value, waiters, body_runs, completed>>

Init ==
    /\ phase = "Pending"
    /\ value = 0
    /\ waiters = 0
    /\ body_runs = 0
    /\ completed = FALSE

Complete ==
    /\ phase = "Pending"
    /\ phase' = "Done"
    /\ value' = 1
    /\ completed' = TRUE
    /\ body_runs' = 1
    /\ UNCHANGED waiters

CompleteAgain ==
    /\ phase = "Done"
    /\ UNCHANGED vars

Await ==
    /\ waiters < MaxWaiters
    /\ waiters' = waiters + 1
    /\ UNCHANGED <<phase, value, body_runs, completed>>

ReadDone ==
    /\ phase = "Done"
    /\ UNCHANGED vars

Next ==
    \/ Complete
    \/ CompleteAgain
    \/ Await
    \/ ReadDone

BodyRunsOnce == body_runs <= 1
CompletionPermanent == completed => phase = "Done"
ValueStable == phase = "Done" => value = 1
WaitersBounded == waiters <= MaxWaiters
CompletionNeedsBody == completed => body_runs = 1

==============================
