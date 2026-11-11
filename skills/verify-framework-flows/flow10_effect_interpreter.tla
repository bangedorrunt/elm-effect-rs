---- MODULE flow10_effect_interpreter ----
EXTENDS Naturals, Sequences

MaxFrames == 2

VARIABLES mode, frames, interrupt, mask, last, result

vars == <<mode, frames, interrupt, mask, last, result>>

Init ==
    /\ mode = "Poll"
    /\ frames = <<>>
    /\ interrupt = FALSE
    /\ mask = 0
    /\ last = "init"
    /\ result = "none"

\* A pending advance pushes a continuation frame; the loop keeps stepping.
PushFrame ==
    /\ mode = "Step"
    /\ Len(frames) < MaxFrames
    /\ frames' = Append(frames, Len(frames) + 1)
    /\ mode' = "Step"
    /\ last' = "push"
    /\ UNCHANGED <<interrupt, mask, result>>

\* Done from the top frame: empty stack means the run succeeded; otherwise
\* resume the frame that was waiting for this value.
RunDone ==
    /\ mode = "Step"
    /\ result' = "done"
    /\ mode' = IF frames = <<>> THEN "Done" ELSE "Resume"
    /\ last' = "done"
    /\ UNCHANGED <<frames, interrupt, mask>>

\* Failure is terminal and unwinds every frame at once. It never resumes.
RunFail ==
    /\ mode = "Step"
    /\ result' = "fail"
    /\ mode' = "Fail"
    /\ frames' = <<>>
    /\ last' = "fail"
    /\ UNCHANGED <<interrupt, mask>>

\* The resumed frame either completes the run or runs its continuation.
ResumeFrame ==
    /\ mode = "Resume"
    /\ Len(frames) > 0
    /\ frames' = IF Len(frames) = 1 THEN <<>> ELSE SubSeq(frames, 1, Len(frames) - 1)
    /\ mode' = IF Len(frames) = 1 THEN "Done" ELSE "Step"
    /\ last' = "resume"
    /\ UNCHANGED <<interrupt, mask, result>>

\* The boundary check: a step starts only when the fiber is unmasked or has
\* no pending interrupt.
BeginStep ==
    /\ mode = "Poll"
    /\ (interrupt = FALSE \/ mask > 0)
    /\ mode' = "Step"
    /\ last' = "step"
    /\ UNCHANGED <<frames, interrupt, mask, result>>

\* An unmasked interrupt at a boundary never enters a step; it fails the run.
InterruptAtBoundary ==
    /\ mode = "Poll"
    /\ interrupt
    /\ mask = 0
    /\ mode' = "Fail"
    /\ frames' = <<>>
    /\ result' = "interrupt"
    /\ last' = "interrupt"
    /\ UNCHANGED <<interrupt, mask>>

RequestInterrupt ==
    /\ mode \in {"Poll", "Step", "Resume"}
    /\ ~interrupt
    /\ interrupt' = TRUE
    /\ last' = "request"
    /\ UNCHANGED <<mode, frames, mask, result>>

EnterMask ==
    /\ mode \in {"Poll", "Step", "Resume"}
    /\ mask < 2
    /\ mask' = mask + 1
    /\ last' = "mask"
    /\ UNCHANGED <<mode, frames, interrupt, result>>

LeaveMask ==
    /\ mode \in {"Poll", "Step", "Resume"}
    /\ mask > 0
    /\ mask' = mask - 1
    /\ last' = "mask"
    /\ UNCHANGED <<mode, frames, interrupt, result>>

TerminalDone ==
    /\ mode = "Done"
    /\ UNCHANGED vars

TerminalFail ==
    /\ mode = "Fail"
    /\ UNCHANGED vars

Next ==
    \/ PushFrame
    \/ RunDone
    \/ RunFail
    \/ ResumeFrame
    \/ BeginStep
    \/ InterruptAtBoundary
    \/ RequestInterrupt
    \/ EnterMask
    \/ LeaveMask
    \/ TerminalDone
    \/ TerminalFail

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ mode \in {"Poll", "Step", "Resume", "Done", "Fail"}
    /\ mask \in 0..2
    /\ interrupt \in BOOLEAN
    /\ result \in {"none", "done", "fail", "interrupt"}
    /\ last \in {"init", "step", "push", "resume", "done", "fail", "request", "mask", "interrupt"}
    /\ Len(frames) <= MaxFrames

FailUnwindsAllFrames ==
    mode = "Fail" => frames = <<>>

DoneHasNoFrames ==
    mode = "Done" => frames = <<>>

NoStepWhileUnmaskedInterrupted ==
    last = "step" => (~interrupt \/ mask > 0)

EveryFailureHasAReason ==
    mode = "Fail" => result \in {"fail", "interrupt"}

ResumeNeedsAFrame ==
    mode = "Resume" => Len(frames) > 0

==============================
