---- MODULE flow7_statechart_parallel ----
EXTENDS Naturals

VARIABLES region1_state, region2_state, parent_state, on_done_emitted, done

vars == <<region1_state, region2_state, parent_state, on_done_emitted, done>>

Init ==
    /\ region1_state = "AudioPlaying"
    /\ region2_state = "VideoPlaying"
    /\ parent_state = "MediaActive"
    /\ on_done_emitted = FALSE
    /\ done = FALSE

AdvanceRegion1Mute ==
    /\ parent_state = "MediaActive"
    /\ region1_state = "AudioPlaying"
    /\ region1_state' = "AudioMuted"
    /\ UNCHANGED <<region2_state, parent_state, on_done_emitted, done>>

AdvanceRegion1Finish ==
    /\ parent_state = "MediaActive"
    /\ region1_state \in {"AudioPlaying", "AudioMuted"}
    /\ region1_state' = "AudioFinal"
    /\ UNCHANGED <<region2_state, parent_state, on_done_emitted, done>>

AdvanceRegion2Finish ==
    /\ parent_state = "MediaActive"
    /\ region2_state = "VideoPlaying"
    /\ region2_state' = "VideoFinal"
    /\ UNCHANGED <<region1_state, parent_state, on_done_emitted, done>>

SynchronizedJoinComplete ==
    /\ parent_state = "MediaActive"
    /\ region1_state = "AudioFinal"
    /\ region2_state = "VideoFinal"
    /\ on_done_emitted' = TRUE
    /\ parent_state' = "PlaybackComplete"
    /\ region1_state' = "None"
    /\ region2_state' = "None"
    /\ done' = TRUE

TerminalStep ==
    /\ done
    /\ UNCHANGED vars

Next ==
    \/ AdvanceRegion1Mute
    \/ AdvanceRegion1Finish
    \/ AdvanceRegion2Finish
    \/ SynchronizedJoinComplete
    \/ TerminalStep

OrthogonalIsolation ==
    (region1_state = "AudioMuted" /\ parent_state = "MediaActive") => (region2_state \in {"VideoPlaying", "VideoFinal"})

SynchronizedJoinOnDone ==
    parent_state = "PlaybackComplete" <=> on_done_emitted

NoPrematureCompletion ==
    parent_state = "PlaybackComplete" => (region1_state = "None" /\ region2_state = "None")

NoDeadlock ==
    ~done => (parent_state = "MediaActive")

==============================
