export const title = "Statechart Delayed Transitions: Virtual Time Dwell and Preemptive Invalidation";

export const state = (v) => ({
  MachineState: v.machine_state,
  TimerStatus: v.timer_status,
  VirtualTicks: "Tick_" + v.clock_ticks,
});

export const step = (action, from, to) => {
  switch (action) {
    case "VirtualClockAdvances":
      return "virtual clock advances via TestClock";
    case "TimerFiresOnDwellMet":
      return "timer dwell met: execute delayed transition";
    case "EarlyExternalEventArrives":
      return "early event preempts state: cancel timer intent";
    case "StaleTimerDiscarded":
      return "stale timer arrival rejected safely";
    case "TerminalStep":
      return null;
    default:
      return action;
  }
};
