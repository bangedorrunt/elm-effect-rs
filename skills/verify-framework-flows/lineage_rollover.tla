---- MODULE lineage_rollover ----
EXTENDS Naturals

MaxEpoch == 2

VARIABLES phase, epoch, next_epoch, prev_terminal, next_terminal, history

vars == <<phase, epoch, next_epoch, prev_terminal, next_terminal, history>>

Init ==
    /\ phase = "Running"
    /\ epoch = 1
    /\ next_epoch = 2
    /\ prev_terminal = FALSE
    /\ next_terminal = FALSE
    /\ history = 1

ContinueAsNew ==
    /\ phase = "Running"
    /\ epoch < MaxEpoch
    /\ ~next_terminal
    /\ phase' = "Closing"
    /\ next_epoch' = epoch + 1
    /\ UNCHANGED <<epoch, prev_terminal, next_terminal, history>>

SettleNextEarly ==
    /\ phase = "Running"
    /\ ~next_terminal
    /\ next_terminal' = TRUE
    /\ UNCHANGED <<phase, epoch, next_epoch, prev_terminal, history>>

ReattachSettledNext ==
    /\ phase = "Running"
    /\ next_terminal
    /\ phase' = "Done"
    /\ prev_terminal' = TRUE
    /\ UNCHANGED <<epoch, next_epoch, next_terminal, history>>

ClosePrev ==
    /\ phase = "Closing"
    /\ phase' = "StartingNext"
    /\ prev_terminal' = TRUE
    /\ UNCHANGED <<epoch, next_epoch, next_terminal, history>>

StartNext ==
    /\ phase = "StartingNext"
    /\ prev_terminal
    /\ phase' = "Running"
    /\ epoch' = next_epoch
    /\ next_epoch' = next_epoch + 1
    /\ history' = 1
    /\ UNCHANGED <<prev_terminal, next_terminal>>

CrashDuringRollover ==
    /\ phase = "Closing"
    /\ phase' = "Crashed"
    /\ UNCHANGED <<epoch, next_epoch, prev_terminal, next_terminal, history>>

RecoverRollover ==
    /\ phase = "Crashed"
    /\ phase' = "Closing"
    /\ UNCHANGED <<epoch, next_epoch, prev_terminal, next_terminal, history>>

FinalEpochStutter ==
    /\ phase = "Running"
    /\ epoch = MaxEpoch
    /\ UNCHANGED vars

DoneStutter ==
    /\ phase = "Done"
    /\ UNCHANGED vars

Next ==
    \/ ContinueAsNew
    \/ SettleNextEarly
    \/ ReattachSettledNext
    \/ ClosePrev
    \/ StartNext
    \/ CrashDuringRollover
    \/ RecoverRollover
    \/ FinalEpochStutter
    \/ DoneStutter

EpochPositive == epoch >= 1
EpochBounded == epoch <= MaxEpoch
NextEpochAdjacent == phase \in {"Closing", "StartingNext", "Crashed"} => next_epoch = epoch + 1
PrevClosedBeforeNext == phase = "StartingNext" => prev_terminal
RunningAfterRolloverClosedPrev == phase = "Running" /\ epoch > 1 => prev_terminal
HistoryBounded == history <= 1
DoneHasClosedPrev == phase = "Done" => prev_terminal
SettledNextStillClosesPrev == next_terminal /\ phase = "Done" => prev_terminal
CrashKeepsNextAdjacent == phase = "Crashed" => next_epoch = epoch + 1

==============================
