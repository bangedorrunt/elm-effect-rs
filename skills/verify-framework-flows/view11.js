export const title = "Scope Masking: LIFO Releases, Uninterruptible Regions, and Deferred Interrupts";

export const state = (v) => ({
  Scope: v.phase,
  Mask: `${v.mask}`,
  Interrupt: v.interrupt_delivered ? "Delivered" : (v.interrupt ? "Requested" : "None"),
});

export const step = (action, from, to) => {
  switch (action) {
    case "Acquire":
      return "acquire a resource and push its finalizer";
    case "EnterMask":
      return "enter uninterruptible region";
    case "LeaveMask":
      return "leave uninterruptible region";
    case "RequestInterrupt":
      return "interrupt requested";
    case "FinishBody":
      return "body completes; close the scope";
    case "StartRelease":
      return "start the next finalizer (release is uninterruptible)";
    case "FinishRelease":
      return "finalizer completes, reverse acquisition order";
    case "DeliverInterrupt":
      return "deliver cancellation at an unmasked boundary";
    case "Close":
      return "all finalizers settled; scope closed";
    case "Terminal":
      return null;
    default:
      return action;
  }
};
