/// <reference types="vite/client" />

declare module "*.mjs" {
  export function prefersCodexTask(text: string): boolean;
  export function shouldUseLocalQwen(input: { modelMode: "hybrid" | "qwen" | "codex"; voiceActive: boolean; text: string }): boolean;
  export function parseCipherPipeCommand(text: string): string | null;
}
