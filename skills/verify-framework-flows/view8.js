export const title = "Statechart Actor Supervision: Scoped Lifecycles, Mailbox FIFO, and Recursive Teardown";

export const state = (v) => ({
  ParentActor: v.parent_actor,
  ChildActor: v.child_actor,
  ChildScope: v.child_scope,
});

export const step = (action, from, to) => {
  switch (action) {
    case "SpawnChildActor":
      return "parent spawns child actor in child scope";
    case "SendMessageToChild":
      return "parent enqueues message to child actor mailbox";
    case "ChildProcessMessage":
      return "child consumes message from FIFO queue";
    case "ParentInitiateStop":
      return "parent state exit triggers recursive stop";
    case "ChildExecuteFinalizerAndClose":
      return "child executes LIFO finalizers and closes scope";
    case "ParentCompleteStop":
      return "all descendants stopped cleanly: parent closes";
    case "TerminalStep":
      return null;
    default:
      return action;
  }
};
