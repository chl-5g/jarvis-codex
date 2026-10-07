const transitions = {
  "capture-speaker": "capturing-speaker",
  "initialize-codex": "initializing-codex",
  "load-memory": "loading-memory",
  "connect-voice": "connecting-voice",
};

/**
 * Run one Jarvis voice session as a sequential workflow.
 * Each step receives the accumulated context and returns a partial update.
 */
export async function runWorkflow(steps, hooks = {}) {
  let context = {};
  let state = "waiting-wake";

  for (const step of steps) {
    state = transitions[step.name] ?? state;
    hooks.onStepStart?.(step.name, state);
    try {
      const update = await step.run(context);
      if (update && typeof update === "object") context = { ...context, ...update };
      hooks.onStepSuccess?.(step.name, update, context);
    } catch (error) {
      const failure = error instanceof Error ? error : new Error(String(error));
      state = "degraded";
      const result = { state, context, failedStep: step.name, error: failure };
      hooks.onFailure?.(step.name, failure, context);
      return result;
    }
  }

  state = "ready";
  const result = { state, context };
  hooks.onComplete?.(result);
  return result;
}
