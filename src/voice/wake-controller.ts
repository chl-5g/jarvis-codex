import { invoke } from "@tauri-apps/api/core";

export type WakeStatus = { enabled: boolean; ready: boolean; authorization: string };

export type WakeControllerDeps = {
  setStatus: (status: WakeStatus) => void;
  onDenied: () => void;
  onError: (error: unknown) => void;
};

export function createWakeController(deps: WakeControllerDeps) {
  let inFlight: Promise<void> | null = null;
  return async function armWakeListener(): Promise<void> {
    if (inFlight) return inFlight;
    inFlight = (async () => {
      try {
        const status = await invoke<WakeStatus>("arm_wake_listener");
        deps.setStatus(status);
        if (["denied", "restricted"].includes(status.authorization)) deps.onDenied();
      } catch (error) {
        deps.onError(error);
      }
    })().finally(() => { inFlight = null; });
    return inFlight;
  };
}
