---- MODULE test_clock ----
EXTENDS Naturals

MaxTime == 5
Deadline1 == 2
Deadline2 == 4

VARIABLES now, sleeper1, sleeper2, woken1, woken2

vars == <<now, sleeper1, sleeper2, woken1, woken2>>

Init ==
    /\ now = 0
    /\ sleeper1 = TRUE
    /\ sleeper2 = TRUE
    /\ woken1 = FALSE
    /\ woken2 = FALSE

Tick ==
    /\ now < MaxTime
    /\ now' = now + 1
    /\ UNCHANGED <<sleeper1, sleeper2, woken1, woken2>>

Wake1 ==
    /\ sleeper1
    /\ now >= Deadline1
    /\ sleeper1' = FALSE
    /\ woken1' = TRUE
    /\ UNCHANGED <<now, sleeper2, woken2>>

Wake2 ==
    /\ sleeper2
    /\ ~sleeper1
    /\ now >= Deadline2
    /\ sleeper2' = FALSE
    /\ woken2' = TRUE
    /\ UNCHANGED <<now, sleeper1, woken1>>

TerminalStutter ==
    /\ ~sleeper1
    /\ ~sleeper2
    /\ UNCHANGED vars

Next ==
    \/ Tick
    \/ Wake1
    \/ Wake2
    \/ TerminalStutter

NoEarlyWake1 == woken1 => now >= Deadline1
NoEarlyWake2 == woken2 => now >= Deadline2
WakeOrder == woken2 => woken1
TimeBounded == now <= MaxTime
WokenNotSleeping == woken1 => ~sleeper1
Woken2NotSleeping == woken2 => ~sleeper2

==============================
