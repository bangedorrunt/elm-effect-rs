export const title = "Elm Loop: Reactive Fold, Pre-Dispatch Journaling, and Fenced Command Window";

export const state = (v) => ({
  LoopPhase: v.phase,
  EpochStatus: v.epoch_valid ? "ValidEpoch" : "StaleEpoch",
  JournalStatus: v.journal_logged ? "Logged" : "Unlogged",
});

export const step = (action, from, to) => {
  switch (action) {
    case "ReceiveMessage":
      return "external Msg arrives via app.send()";
    case "JournalAppendSuccess":
      return "pre-dispatch append accepted to durable log";
    case "JournalAppendStaleEpoch":
      return "zombie epoch rejected at fence -> skip fold";
    case "PureReduceStep":
      return "pure fold: update(model, msg) yields commands";
    case "WindowExecuteCommand":
      return "bounded window executes command via provider";
    case "FeedOutcomeBackToLoop":
      return "interpreter dispatches outcome Msg back to fold";
    case "FencingEvent":
      return "epoch increments elsewhere";
    case "TerminalDone":
      return null;
    default:
      return action;
  }
};
