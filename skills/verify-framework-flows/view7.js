export const title = "Statechart Parallel Regions: Orthogonal Isolation and Synchronized Join Completion";

export const state = (v) => ({
  ParentState: v.parent_state,
  RegionAudio: v.region1_state,
  RegionVideo: v.region2_state,
});

export const step = (action, from, to) => {
  switch (action) {
    case "AdvanceRegion1Mute":
      return "audio region handles mute independently";
    case "AdvanceRegion1Finish":
      return "audio region reaches final state";
    case "AdvanceRegion2Finish":
      return "video region reaches final state";
    case "SynchronizedJoinComplete":
      return "all parallel regions completed: bubble onDone to parent";
    case "TerminalStep":
      return null;
    default:
      return action;
  }
};
