export type LifecycleDeps = {
  stopVoice: () => Promise<void>;
  hasVoiceConnection: () => boolean;
  cleanupVoiceConnection: () => void;
  exit: () => Promise<void>;
};

/** Stops front-end voice resources before asking the trusted backend to exit. */
export async function shutdownJarvis(deps: LifecycleDeps): Promise<void> {
  if (deps.hasVoiceConnection()) {
    try {
      await deps.stopVoice();
    } catch {
      deps.cleanupVoiceConnection();
    }
  }
  await deps.exit();
}
