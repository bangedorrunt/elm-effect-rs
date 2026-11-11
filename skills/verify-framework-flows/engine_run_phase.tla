---- MODULE engine_run_phase ----
EXTENDS Naturals

MaxParks == 2

VARIABLES phase, parked, settled, failed, cancelled, generation, park_count

vars == <<phase, parked, settled, failed, cancelled, generation, park_count>>

Init ==
    /\ phase = "New"
    /\ parked = FALSE
    /\ settled = FALSE
    /\ failed = FALSE
    /\ cancelled = FALSE
    /\ generation = 0
    /\ park_count = 0

Start ==
    /\ phase = "New"
    /\ phase' = "Running"
    /\ generation' = 1
    /\ UNCHANGED <<parked, settled, failed, cancelled, park_count>>

Park ==
    /\ phase = "Running"
    /\ park_count < MaxParks
    /\ phase' = "Parked"
    /\ parked' = TRUE
    /\ park_count' = park_count + 1
    /\ generation' = generation + 1
    /\ UNCHANGED <<settled, failed, cancelled>>

Wake ==
    /\ phase = "Parked"
    /\ phase' = "Running"
    /\ parked' = FALSE
    /\ generation' = generation + 1
    /\ UNCHANGED <<settled, failed, cancelled, park_count>>

Settle ==
    /\ phase = "Running"
    /\ phase' = "Settled"
    /\ settled' = TRUE
    /\ UNCHANGED <<parked, failed, cancelled, generation, park_count>>

Fail ==
    /\ phase = "Running"
    /\ phase' = "Failed"
    /\ failed' = TRUE
    /\ UNCHANGED <<parked, settled, cancelled, generation, park_count>>

CancelRunning ==
    /\ phase = "Running"
    /\ phase' = "Cancelled"
    /\ cancelled' = TRUE
    /\ UNCHANGED <<parked, settled, failed, generation, park_count>>

CancelParked ==
    /\ phase = "Parked"
    /\ phase' = "Cancelled"
    /\ cancelled' = TRUE
    /\ parked' = FALSE
    /\ UNCHANGED <<settled, failed, generation, park_count>>

DoneStutter ==
    /\ phase \in {"Settled", "Failed", "Cancelled"}
    /\ UNCHANGED vars

Next ==
    \/ Start
    \/ Park
    \/ Wake
    \/ Settle
    \/ Fail
    \/ CancelRunning
    \/ CancelParked
    \/ DoneStutter

ExclusiveTerminals ==
    /\ ~(settled /\ failed)
    /\ ~(settled /\ cancelled)
    /\ ~(failed /\ cancelled)

RunningClear == phase = "Running" => ~settled /\ ~failed /\ ~cancelled
ParkedClear == phase = "Parked" => parked /\ ~settled /\ ~failed /\ ~cancelled
TerminalPhase == settled => phase = "Settled"
FailedPhase == failed => phase = "Failed"
CancelledPhase == cancelled => phase = "Cancelled"
ParkedResumable == phase = "Parked" => ~cancelled
GenerationBounded == generation <= 2 * MaxParks + 1
ParkCountBounded == park_count <= MaxParks
GenerationLaw ==
    /\ (phase = "Running" => generation = 2 * park_count + 1)
    /\ (phase = "Parked" => generation = 2 * park_count)

==============================
