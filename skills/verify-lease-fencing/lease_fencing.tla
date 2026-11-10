---- MODULE lease_fencing ----
EXTENDS Naturals

VARIABLES clock, journal_owner, journal_deadline, journal_gen, journal_state,
          workerA_state, workerA_gen, workerA_deadline,
          workerB_state, workerB_gen, workerB_deadline,
          split_brain_detected

vars == <<clock, journal_owner, journal_deadline, journal_gen, journal_state,
          workerA_state, workerA_gen, workerA_deadline,
          workerB_state, workerB_gen, workerB_deadline,
          split_brain_detected>>

Init ==
    /\ clock = 0
    /\ journal_owner = "None"
    /\ journal_deadline = 0
    /\ journal_gen = 1
    /\ journal_state = "InFlight"
    /\ workerA_state = "Unclaimed"
    /\ workerA_gen = 0
    /\ workerA_deadline = 0
    /\ workerB_state = "Unclaimed"
    /\ workerB_gen = 0
    /\ workerB_deadline = 0
    /\ split_brain_detected = FALSE

WorkerAClaim ==
    /\ workerA_state = "Unclaimed"
    /\ journal_state = "InFlight"
    /\ (journal_owner = "None" \/ clock >= journal_deadline)
    /\ journal_owner' = "WorkerA"
    /\ journal_gen' = journal_gen + 1
    /\ journal_deadline' = clock + 2
    /\ workerA_state' = "Holding"
    /\ workerA_gen' = journal_gen'
    /\ workerA_deadline' = clock + 2
    /\ UNCHANGED <<clock, journal_state, workerB_state, workerB_gen, workerB_deadline, split_brain_detected>>

WorkerAHeartbeat ==
    /\ workerA_state = "Holding"
    /\ journal_owner = "WorkerA"
    /\ workerA_gen = journal_gen
    /\ clock < journal_deadline
    /\ clock < 4
    /\ journal_deadline' = clock + 2
    /\ workerA_deadline' = clock + 2
    /\ UNCHANGED <<clock, journal_owner, journal_gen, journal_state,
                  workerA_state, workerA_gen,
                  workerB_state, workerB_gen, workerB_deadline, split_brain_detected>>

WorkerAStall ==
    /\ workerA_state = "Holding"
    /\ workerA_state' = "Stalled"
    /\ UNCHANGED <<clock, journal_owner, journal_deadline, journal_gen, journal_state,
                  workerA_gen, workerA_deadline,
                  workerB_state, workerB_gen, workerB_deadline, split_brain_detected>>

AdvanceClock ==
    /\ clock < 4
    /\ clock' = clock + 1
    /\ UNCHANGED <<journal_owner, journal_deadline, journal_gen, journal_state,
                  workerA_state, workerA_gen, workerA_deadline,
                  workerB_state, workerB_gen, workerB_deadline, split_brain_detected>>

WorkerBReclaimAfterExpiry ==
    /\ workerB_state = "Unclaimed"
    /\ journal_state = "InFlight"
    /\ journal_owner = "WorkerA"
    /\ clock >= journal_deadline
    /\ journal_owner' = "WorkerB"
    /\ journal_gen' = journal_gen + 1
    /\ journal_deadline' = clock + 2
    /\ workerB_state' = "Holding"
    /\ workerB_gen' = journal_gen'
    /\ workerB_deadline' = clock + 2
    /\ UNCHANGED <<clock, journal_state, workerA_state, workerA_gen, workerA_deadline, split_brain_detected>>

WorkerAAttemptSettle ==
    /\ workerA_state \in {"Holding", "Stalled"}
    /\ journal_state = "InFlight"
    /\ IF workerA_gen = journal_gen
       THEN /\ journal_state' = "Settled"
            /\ workerA_state' = "Committed"
            /\ IF workerB_state = "Committed"
               THEN split_brain_detected' = TRUE
               ELSE UNCHANGED split_brain_detected
            /\ UNCHANGED <<journal_owner, journal_deadline, journal_gen, workerA_gen, workerA_deadline,
                          workerB_state, workerB_gen, workerB_deadline>>
       ELSE /\ workerA_state' = "Fenced"
            /\ UNCHANGED <<journal_owner, journal_deadline, journal_gen, journal_state,
                          workerA_gen, workerA_deadline,
                          workerB_state, workerB_gen, workerB_deadline, split_brain_detected>>
    /\ UNCHANGED clock

WorkerBAttemptSettle ==
    /\ workerB_state = "Holding"
    /\ journal_state = "InFlight"
    /\ IF workerB_gen = journal_gen
       THEN /\ journal_state' = "Settled"
            /\ workerB_state' = "Committed"
            /\ IF workerA_state = "Committed"
               THEN split_brain_detected' = TRUE
               ELSE UNCHANGED split_brain_detected
            /\ UNCHANGED <<journal_owner, journal_deadline, journal_gen, workerA_state, workerA_gen, workerA_deadline,
                          workerB_gen, workerB_deadline>>
       ELSE /\ workerB_state' = "Fenced"
            /\ UNCHANGED <<journal_owner, journal_deadline, journal_gen, journal_state,
                          workerA_state, workerA_gen, workerA_deadline,
                          workerB_state, workerB_gen, workerB_deadline, split_brain_detected>>
    /\ UNCHANGED clock

TerminalDone ==
    /\ (journal_state = "Settled" \/ clock >= 4)
    /\ UNCHANGED vars

Next ==
    \/ WorkerAClaim
    \/ WorkerAHeartbeat
    \/ WorkerAStall
    \/ AdvanceClock
    \/ WorkerBReclaimAfterExpiry
    \/ WorkerAAttemptSettle
    \/ WorkerBAttemptSettle
    \/ TerminalDone

NoSplitBrain ==
    ~split_brain_detected /\ ~(workerA_state = "Committed" /\ workerB_state = "Committed")

MutualExclusionActiveLease ==
    ~((workerA_state = "Holding" /\ clock < workerA_deadline) /\
      (workerB_state = "Holding" /\ clock < workerB_deadline))

StaleWorkerFenced ==
    (workerA_gen < journal_gen => workerA_state /= "Committed") /\
    (workerB_gen < journal_gen => workerB_state /= "Committed")

MonotonicGeneration ==
    journal_gen >= 1

==============================
