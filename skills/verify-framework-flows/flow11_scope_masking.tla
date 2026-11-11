---- MODULE flow11_scope_masking ----
EXTENDS Naturals, Sequences

MaxResources == 2

VARIABLES phase, mask, interrupt, interrupt_delivered,
          acquired, released, release_in_flight, body_done, last

vars == <<phase, mask, interrupt, interrupt_delivered,
          acquired, released, release_in_flight, body_done, last>>

Init ==
    /\ phase = "Running"
    /\ mask = 0
    /\ interrupt = FALSE
    /\ interrupt_delivered = FALSE
    /\ acquired = <<>>
    /\ released = <<>>
    /\ release_in_flight = FALSE
    /\ body_done = FALSE
    /\ last = "init"

Acquire ==
    /\ phase = "Running"
    /\ Len(acquired) < MaxResources
    /\ acquired' = Append(acquired, Len(acquired) + 1)
    /\ last' = "acquire"
    /\ UNCHANGED <<phase, mask, interrupt, interrupt_delivered,
                    released, release_in_flight, body_done>>

EnterMask ==
    /\ phase \in {"Running", "Closing"}
    /\ mask < 2
    /\ ~interrupt_delivered
    /\ mask' = mask + 1
    /\ last' = "mask"
    /\ UNCHANGED <<phase, interrupt, interrupt_delivered,
                    acquired, released, release_in_flight, body_done>>

LeaveMask ==
    /\ phase \in {"Running", "Closing"}
    /\ mask > 0
    /\ mask' = mask - 1
    /\ last' = "mask"
    /\ UNCHANGED <<phase, interrupt, interrupt_delivered,
                    acquired, released, release_in_flight, body_done>>

RequestInterrupt ==
    /\ phase \in {"Running", "Closing"}
    /\ ~interrupt
    /\ interrupt' = TRUE
    /\ last' = "request"
    /\ UNCHANGED <<phase, mask, interrupt_delivered,
                    acquired, released, release_in_flight, body_done>>

FinishBody ==
    /\ phase = "Running"
    /\ ~body_done
    /\ body_done' = TRUE
    /\ phase' = "Closing"
    /\ last' = "finish"
    /\ UNCHANGED <<mask, interrupt, interrupt_delivered,
                    acquired, released, release_in_flight>>

\* A release is uninterruptible: delivery waits for it to finish.
StartRelease ==
    /\ phase = "Closing"
    /\ ~release_in_flight
    /\ Len(released) < Len(acquired)
    /\ release_in_flight' = TRUE
    /\ last' = "release-start"
    /\ UNCHANGED <<phase, mask, interrupt, interrupt_delivered,
                    acquired, released, body_done>>

FinishRelease ==
    /\ phase = "Closing"
    /\ release_in_flight
    /\ released' = Append(released, acquired[Len(acquired) - Len(released)])
    /\ release_in_flight' = FALSE
    /\ last' = "release"
    /\ UNCHANGED <<phase, mask, interrupt, interrupt_delivered,
                    acquired, body_done>>

\* Cancellation is delivered only at an unmasked, non-release boundary.
DeliverInterrupt ==
    /\ phase \in {"Running", "Closing"}
    /\ interrupt
    /\ mask = 0
    /\ ~release_in_flight
    /\ ~interrupt_delivered
    /\ interrupt_delivered' = TRUE
    /\ phase' = "Closing"
    /\ last' = "deliver"
    /\ UNCHANGED <<mask, interrupt, acquired, released, release_in_flight, body_done>>

Close ==
    /\ phase = "Closing"
    /\ ~release_in_flight
    /\ Len(released) = Len(acquired)
    /\ phase' = "Closed"
    /\ last' = "close"
    /\ UNCHANGED <<mask, interrupt, interrupt_delivered,
                    acquired, released, release_in_flight, body_done>>

Terminal ==
    /\ phase = "Closed"
    /\ UNCHANGED vars

Next ==
    \/ Acquire
    \/ EnterMask
    \/ LeaveMask
    \/ RequestInterrupt
    \/ FinishBody
    \/ StartRelease
    \/ FinishRelease
    \/ DeliverInterrupt
    \/ Close
    \/ Terminal

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ phase \in {"Running", "Closing", "Closed"}
    /\ mask \in 0..2
    /\ interrupt \in BOOLEAN
    /\ interrupt_delivered \in BOOLEAN
    /\ release_in_flight \in BOOLEAN
    /\ body_done \in BOOLEAN
    /\ Len(acquired) <= MaxResources
    /\ Len(released) <= Len(acquired)
    /\ last \in {"init", "acquire", "mask", "request", "finish",
                 "release-start", "release", "deliver", "close"}

ReleasedOnce ==
    Len(released) <= Len(acquired)

LifoRelease ==
    Len(released) > 0 =>
        \A i \in DOMAIN released:
            released[i] = acquired[Len(acquired) - i + 1]

NoDuplicateReleased ==
    \A i, j \in DOMAIN released: i /= j => released[i] /= released[j]

NoReleaseBeforeClose ==
    Len(released) > 0 => phase \in {"Closing", "Closed"}

TerminalReleasedAll ==
    phase = "Closed" => Len(released) = Len(acquired)

MaskedInterruptDeferred ==
    mask > 0 => ~interrupt_delivered

InterruptDeliveredUnmasked ==
    interrupt_delivered => mask = 0

NormalCloseNeedsBody ==
    (~interrupt_delivered /\ phase = "Closed") => body_done

==============================
