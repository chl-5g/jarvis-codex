import { invoke } from "@tauri-apps/api/core";

export type VoiceInfo = { voiceActive: boolean; [key: string]: unknown };

export type VoiceControllerDeps = {
  cleanup: () => void;
  updateInfo: (info: VoiceInfo) => void;
  resetSpeaker: () => void;
};

export function createVoiceController(deps: VoiceControllerDeps) {
  return {
    async stop(): Promise<void> {
      try {
        const info = await invoke<VoiceInfo>("stop_codex_voice");
        deps.updateInfo(info);
      } finally {
        deps.cleanup();
        deps.resetSpeaker();
      }
    },
  };
}
