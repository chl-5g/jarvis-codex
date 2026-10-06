const CODEX_TASK_PATTERN = /https?:\/\/|访问|打开|查看|执行|读取|修改|安装|删除|搜索/;

export function prefersCodexTask(text) {
  return CODEX_TASK_PATTERN.test(text);
}

export function shouldUseLocalQwen({ modelMode, voiceActive, text }) {
  // Online-first routing: local Qwen is an explicit offline/degraded route.
  // The hybrid mode reaches the native Codex task path first and falls back
  // only when that request cannot be submitted.
  return modelMode === "qwen" && !voiceActive && !prefersCodexTask(text);
}

export function parseCipherPipeCommand(text) {
  const match = text.trim().match(/^(?:发给\s*CipherPipe|cipherpipe)[:：]\s*(.+)$/i);
  return match ? match[1].trim() : null;
}
