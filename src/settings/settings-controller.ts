import { invoke } from "@tauri-apps/api/core";

export type BridgeStatus = { enabled: boolean; bindAddress: string; port?: number; endpoint?: string };

export function syncPermissionControls(permissionMode: string, modelMode: string, permissionLabels: Record<string, string>, modelModeLabels: Record<string, string>): void {
  const input = document.querySelector<HTMLInputElement>(`input[name="permission-mode"][value="${permissionMode}"]`);
  if (input) input.checked = true;
  const permissionLabel = document.querySelector<HTMLElement>("#permission-mode-label");
  if (permissionLabel) permissionLabel.textContent = permissionLabels[permissionMode];
  const modelInput = document.querySelector<HTMLInputElement>(`input[name="model-mode"][value="${modelMode}"]`);
  if (modelInput) modelInput.checked = true;
  const modelLabel = document.querySelector<HTMLElement>("#model-mode-label");
  if (modelLabel) modelLabel.textContent = modelModeLabels[modelMode];
}

export function showBridgeStatus(status: BridgeStatus, tokenKey: string): void {
  const bind = document.querySelector<HTMLInputElement>("#bridge-bind");
  const bridgeStatus = document.querySelector<HTMLElement>("#bridge-status");
  const endpoint = document.querySelector<HTMLElement>("#bridge-endpoint");
  const token = document.querySelector<HTMLElement>("#bridge-token");
  if (!bind || !bridgeStatus || !endpoint || !token) return;
  bind.value = status.bindAddress || bind.value;
  bridgeStatus.textContent = status.enabled ? `已启用 · ${status.bindAddress}:${status.port ?? "?"}` : "本地桥已关闭";
  endpoint.textContent = status.enabled ? `端点：${status.endpoint ?? "未知"} · 使用 Bearer token 访问 /command 和 /events` : "默认只监听本机；改为私有局域网地址后，快捷指令可 POST /command。";
  const savedToken = localStorage.getItem(tokenKey);
  token.hidden = !status.enabled && !savedToken;
  if (!status.enabled && savedToken) token.textContent = "已有配对 token 保存在本机设置中";
}

export async function refreshBridgeStatus(tokenKey: string): Promise<void> {
  try {
    showBridgeStatus(await invoke<BridgeStatus>("bridge_status"), tokenKey);
  } catch (error) {
    const bridgeStatus = document.querySelector<HTMLElement>("#bridge-status");
    if (bridgeStatus) bridgeStatus.textContent = `本地桥状态读取失败：${String(error)}`;
  }
}
