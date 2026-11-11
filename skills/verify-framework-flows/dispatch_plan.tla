---- MODULE dispatch_plan ----
EXTENDS Naturals

States == {"Prepared", "InFlight", "Accepted", "Settled", "Failed", "Cancelled", "Unknown", "Parked", "ReconcilePending"}
Plans == {"DispatchNew", "RetrySameKey", "ReconcileFirst", "RefuseBlindRetry", "DuplicateTerminal", "ResumeParked"}

VARIABLES phase, state, idempotent, query, lease_live, plan

vars == <<phase, state, idempotent, query, lease_live, plan>>

Decide(s, idem, q, live) ==
    IF s = "Prepared" THEN "DispatchNew"
    ELSE IF s \in {"Settled", "Failed", "Cancelled"} THEN "DuplicateTerminal"
    ELSE IF s = "Parked" THEN "ResumeParked"
    ELSE IF s = "ReconcilePending" THEN "RefuseBlindRetry"
    ELSE IF live THEN "RefuseBlindRetry"
    ELSE IF q THEN "ReconcileFirst"
    ELSE IF idem THEN "RetrySameKey"
    ELSE "RefuseBlindRetry"

Init ==
    /\ phase = "Choose"
    /\ state \in States
    /\ idempotent \in BOOLEAN
    /\ query \in BOOLEAN
    /\ lease_live \in BOOLEAN
    /\ plan = "None"

Choose ==
    /\ phase = "Choose"
    /\ plan' = Decide(state, idempotent, query, lease_live)
    /\ phase' = "Planned"
    /\ UNCHANGED <<state, idempotent, query, lease_live>>

Observe ==
    /\ phase = "Planned"
    /\ phase' = "Done"
    /\ UNCHANGED <<state, idempotent, query, lease_live, plan>>

DoneStutter ==
    /\ phase = "Done"
    /\ UNCHANGED vars

Next ==
    \/ Choose
    \/ Observe
    \/ DoneStutter

Decided == phase \in {"Planned", "Done"}

PlanIsTotal == Decided => plan \in Plans
PreparedDispatchesNew == Decided /\ state = "Prepared" => plan = "DispatchNew"
TerminalsDuplicate == Decided /\ state \in {"Settled", "Failed", "Cancelled"} => plan = "DuplicateTerminal"
ParkedResumes == Decided /\ state = "Parked" => plan = "ResumeParked"
ReconcileRefuses == Decided /\ state = "ReconcilePending" => plan = "RefuseBlindRetry"
LiveLeaseRefuses == Decided /\ lease_live /\ state \in {"InFlight", "Accepted", "Unknown"} => plan = "RefuseBlindRetry"
QueryBeatsIdempotency == Decided /\ ~lease_live /\ query /\ state \in {"InFlight", "Accepted", "Unknown"} => plan = "ReconcileFirst"
IdempotentRetriesSameKey == Decided /\ ~lease_live /\ ~query /\ idempotent /\ state \in {"InFlight", "Accepted", "Unknown"} => plan = "RetrySameKey"
NoContractRefuses == Decided /\ ~lease_live /\ ~query /\ ~idempotent /\ state \in {"InFlight", "Accepted", "Unknown"} => plan = "RefuseBlindRetry"

==============================
