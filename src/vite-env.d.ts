/// <reference types="vite/client" />

declare module "*.mjs" {
  export function prefersCodexTask(text: string): boolean;
  export function shouldUseLocalQwen(input: { modelMode: "hybrid" | "qwen" | "codex"; voiceActive: boolean; text: string }): boolean;
  export function parseCipherPipeCommand(text: string): string | null;
  export class AudioEventWindow {
    constructor(sampleRate: number, windowMs?: number);
    push(samples: Float32Array): Record<string, unknown> | null;
  }
  export function runWorkflow(
    steps: Array<{
      name: string;
      run: (context: Record<string, any>) => Promise<Record<string, any> | void>;
    }>,
    hooks?: {
      onStepStart?: (name: string, state: string) => void;
      onStepSuccess?: (name: string, update: unknown, context: Record<string, any>) => void;
      onFailure?: (name: string, error: Error, context: Record<string, any>) => void;
      onComplete?: (result: { state: string; context: Record<string, any> }) => void;
    },
  ): Promise<{
    state: string;
    context: Record<string, any>;
    failedStep?: string;
    error?: Error;
  }>;
}
