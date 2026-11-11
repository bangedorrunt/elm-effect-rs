export const title = "Effect Interpreter: Step/Drive Trampoline, Frames, Interrupts, and Masks";

export const state = (v) => ({
  Interpreter: v.mode,
  Interrupt: v.interrupt ? "Requested" : "None",
  Mask: `${v.mask}`,
  Result: v.result,
});

export const step = (action, from, to) => {
  switch (action) {
    case "BeginStep":
      return "boundary check passes: no unmasked interrupt";
    case "InterruptAtBoundary":
      return "unmasked interrupt at boundary fails before a step";
    case "PushFrame":
      return "pending advance pushes a continuation frame";
    case "RunDone":
      return "step returns Done";
    case "RunFail":
      return "step returns Fail: unwind every frame";
    case "ResumeFrame":
      return "resume the waiting frame (LIFO)";
    case "RequestInterrupt":
      return "interrupt requested; delivered at the next boundary";
    case "EnterMask":
      return "enter uninterruptible region";
    case "LeaveMask":
      return "leave uninterruptible region";
    case "TerminalDone":
      return null;
    case "TerminalFail":
      return null;
    default:
      return action;
  }
};
