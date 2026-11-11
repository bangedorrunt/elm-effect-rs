---- MODULE park_plan ----
EXTENDS Naturals

MaxAsked == 4
MaxCeiling == 4
MaxNow == 4

VARIABLES phase, asked_s, ceiling_s, now, decision, rows

vars == <<phase, asked_s, ceiling_s, now, decision, rows>>

Decide(asked, ceiling, current) ==
    IF ceiling > 0 /\ asked > ceiling THEN "RefusedOverCeiling"
    ELSE IF current >= asked THEN "ResumeNow"
    ELSE "Wait"

Init ==
    /\ phase = "Evaluate"
    /\ asked_s \in 0..MaxAsked
    /\ ceiling_s \in 0..MaxCeiling
    /\ now \in 0..MaxNow
    /\ decision = "None"
    /\ rows = 0

Evaluate ==
    /\ phase = "Evaluate"
    /\ decision' = Decide(asked_s, ceiling_s, now)
    /\ phase' = "Decided"
    /\ UNCHANGED <<asked_s, ceiling_s, now, rows>>

Upsert ==
    /\ phase = "Decided"
    /\ decision = "Wait"
    /\ rows = 0
    /\ rows' = 1
    /\ phase' = "Done"
    /\ UNCHANGED <<asked_s, ceiling_s, now, decision>>

Finish ==
    /\ phase = "Decided"
    /\ decision /= "Wait"
    /\ phase' = "Done"
    /\ UNCHANGED <<asked_s, ceiling_s, now, decision, rows>>

DoneStutter ==
    /\ phase = "Done"
    /\ UNCHANGED vars

Next ==
    \/ Evaluate
    \/ Upsert
    \/ Finish
    \/ DoneStutter

RefusalNeedsCeiling == decision = "RefusedOverCeiling" => ceiling_s > 0 /\ asked_s > ceiling_s
WaitHasFutureDeadline == decision = "Wait" => now < asked_s
ResumeReachedDeadline == decision = "ResumeNow" => now >= asked_s
DecisionIsTotal == phase \in {"Decided", "Done"} => decision \in {"Wait", "ResumeNow", "RefusedOverCeiling"}
AtMostOneRow == rows <= 1
WaitHasOneRow == phase = "Done" /\ decision = "Wait" => rows = 1
RefusalHasNoRow == decision = "RefusedOverCeiling" => rows = 0

==============================
