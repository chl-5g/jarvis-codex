const CODEX_TASK_PATTERN = /https?:\/\/|访问|打开|查看|执行|读取|修改|安装|删除|搜索/;

export function prefersCodexTask(text) {
  return CODEX_TASK_PATTERN.test(text);
}

export function shouldUseLocalQwen({ modelMode, voiceActive, text }) {
  return modelMode === "qwen"
    || (modelMode === "hybrid" && !voiceActive && !prefersCodexTask(text));
}
