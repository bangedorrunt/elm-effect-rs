---- MODULE schedule_state ----
EXTENDS Naturals

MaxRetries == 2

VARIABLES phase, attempts, remaining, last

vars == <<phase, attempts, remaining, last>>

Init ==
    /\ phase = "Ready"
    /\ attempts = 0
    /\ remaining = MaxRetries
    /\ last = "init"

Start ==
    /\ phase = "Ready"
    /\ phase' = "Running"
    /\ attempts' = 1
    /\ last' = "start"
    /\ UNCHANGED remaining

Succeed ==
    /\ phase = "Running"
    /\ phase' = "Done"
    /\ last' = "success"
    /\ UNCHANGED <<attempts, remaining>>

FailWithBudget ==
    /\ phase = "Running"
    /\ remaining > 0
    /\ phase' = "Waiting"
    /\ remaining' = remaining - 1
    /\ last' = "wait"
    /\ UNCHANGED attempts

FailAtBudget ==
    /\ phase = "Running"
    /\ remaining = 0
    /\ phase' = "Done"
    /\ last' = "stop"
    /\ UNCHANGED <<attempts, remaining>>

Wake ==
    /\ phase = "Waiting"
    /\ phase' = "Running"
    /\ attempts' = attempts + 1
    /\ last' = "wake"
    /\ UNCHANGED remaining

DoneStutter ==
    /\ phase = "Done"
    /\ UNCHANGED vars

Next ==
    \/ Start
    \/ Succeed
    \/ FailWithBudget
    \/ FailAtBudget
    \/ Wake
    \/ DoneStutter

AttemptsBounded == attempts <= MaxRetries + 1
WaitingWithinBudget == phase = "Waiting" => remaining <= MaxRetries
StopIsFinal == phase = "Done" => last \in {"success", "stop"}
WakeIsARetry == last = "wake" => attempts > 1

==============================
