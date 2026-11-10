---- MODULE flow1_elm_loop ----
EXTENDS Naturals

VARIABLES phase, journal_logged, cmds_pending, epoch_valid, model_updated, done

vars == <<phase, journal_logged, cmds_pending, epoch_valid, model_updated, done>>

Init ==
    /\ phase = "Idle"
    /\ journal_logged = FALSE
    /\ cmds_pending = 0
    /\ epoch_valid = TRUE
    /\ model_updated = FALSE
    /\ done = FALSE

ReceiveMessage ==
    /\ phase = "Idle"
    /\ ~done
    /\ phase' = "PreJournal"
    /\ UNCHANGED <<journal_logged, cmds_pending, epoch_valid, model_updated, done>>

JournalAppendSuccess ==
    /\ phase = "PreJournal"
    /\ epoch_valid
    /\ journal_logged' = TRUE
    /\ phase' = "PureReduce"
    /\ UNCHANGED <<cmds_pending, epoch_valid, model_updated, done>>

JournalAppendStaleEpoch ==
    /\ phase = "PreJournal"
    /\ ~epoch_valid
    /\ phase' = "Fenced"
    /\ done' = TRUE
    /\ UNCHANGED <<journal_logged, cmds_pending, epoch_valid, model_updated>>

PureReduceStep ==
    /\ phase = "PureReduce"
    /\ journal_logged
    /\ model_updated' = TRUE
    /\ cmds_pending' = 2
    /\ phase' = "WindowDispatch"
    /\ UNCHANGED <<journal_logged, epoch_valid, done>>

WindowExecuteCommand ==
    /\ phase = "WindowDispatch"
    /\ cmds_pending > 0
    /\ cmds_pending' = cmds_pending - 1
    /\ IF cmds_pending' = 0
       THEN phase' = "InterpretFeedback"
       ELSE phase' = "WindowDispatch"
    /\ UNCHANGED <<journal_logged, epoch_valid, model_updated, done>>

FeedOutcomeBackToLoop ==
    /\ phase = "InterpretFeedback"
    /\ phase' = "Completed"
    /\ done' = TRUE
    /\ UNCHANGED <<journal_logged, cmds_pending, epoch_valid, model_updated>>

FencingEvent ==
    /\ phase \in {"Idle", "PreJournal"}
    /\ epoch_valid
    /\ epoch_valid' = FALSE
    /\ UNCHANGED <<phase, journal_logged, cmds_pending, model_updated, done>>

TerminalDone ==
    /\ done
    /\ UNCHANGED vars

Next ==
    \/ ReceiveMessage
    \/ JournalAppendSuccess
    \/ JournalAppendStaleEpoch
    \/ PureReduceStep
    \/ WindowExecuteCommand
    \/ FeedOutcomeBackToLoop
    \/ FencingEvent
    \/ TerminalDone

NoReduceWithoutJournal ==
    phase = "PureReduce" => journal_logged

NoDispatchWhenFenced ==
    phase = "Fenced" => cmds_pending = 0

==============================
