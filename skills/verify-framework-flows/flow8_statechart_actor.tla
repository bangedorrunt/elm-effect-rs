---- MODULE flow8_statechart_actor ----
EXTENDS Naturals

VARIABLES parent_actor, child_actor, child_scope, msgs_pending, finalizer_run, done

vars == <<parent_actor, child_actor, child_scope, msgs_pending, finalizer_run, done>>

Init ==
    /\ parent_actor = "Running"
    /\ child_actor = "Unspawned"
    /\ child_scope = "Closed"
    /\ msgs_pending = 0
    /\ finalizer_run = FALSE
    /\ done = FALSE

SpawnChildActor ==
    /\ parent_actor = "Running"
    /\ child_actor = "Unspawned"
    /\ child_actor' = "Running"
    /\ child_scope' = "Open"
    /\ UNCHANGED <<parent_actor, msgs_pending, finalizer_run, done>>

SendMessageToChild ==
    /\ parent_actor = "Running"
    /\ child_actor = "Running"
    /\ msgs_pending < 2
    /\ msgs_pending' = msgs_pending + 1
    /\ UNCHANGED <<parent_actor, child_actor, child_scope, finalizer_run, done>>

ChildProcessMessage ==
    /\ child_actor = "Running"
    /\ msgs_pending > 0
    /\ msgs_pending' = msgs_pending - 1
    /\ UNCHANGED <<parent_actor, child_actor, child_scope, finalizer_run, done>>

ParentInitiateStop ==
    /\ parent_actor = "Running"
    /\ child_actor = "Running"
    /\ parent_actor' = "Stopping"
    /\ child_scope' = "Closing"
    /\ UNCHANGED <<child_actor, msgs_pending, finalizer_run, done>>

ChildExecuteFinalizerAndClose ==
    /\ parent_actor = "Stopping"
    /\ child_scope = "Closing"
    /\ finalizer_run' = TRUE
    /\ child_actor' = "Stopped"
    /\ child_scope' = "Closed"
    /\ msgs_pending' = 0
    /\ UNCHANGED <<parent_actor, done>>

ParentCompleteStop ==
    /\ parent_actor = "Stopping"
    /\ child_scope = "Closed"
    /\ child_actor = "Stopped"
    /\ parent_actor' = "Stopped"
    /\ done' = TRUE
    /\ UNCHANGED <<child_actor, child_scope, msgs_pending, finalizer_run>>

TerminalStep ==
    /\ done
    /\ UNCHANGED vars

Next ==
    \/ SpawnChildActor
    \/ SendMessageToChild
    \/ ChildProcessMessage
    \/ ParentInitiateStop
    \/ ChildExecuteFinalizerAndClose
    \/ ParentCompleteStop
    \/ TerminalStep

NoLeakedChildrenOnParentExit ==
    parent_actor = "Stopped" => (child_actor = "Stopped" /\ child_scope = "Closed")

FinalizersSettledBeforeClosed ==
    child_actor = "Stopped" => finalizer_run

NoDeadlock ==
    ~done => parent_actor \in {"Running", "Stopping"}

==============================
