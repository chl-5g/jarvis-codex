export const WORKSPACE_KEY = "jarvis.workspace";
export const THREAD_KEY_PREFIX = "jarvis.threadId:v5-speaker:";

export function savedThreadId(workspace: string): string | null {
  return localStorage.getItem(`${THREAD_KEY_PREFIX}${workspace}`);
}

export function saveThreadId(workspace: string, threadId: string): void {
  localStorage.setItem(`${THREAD_KEY_PREFIX}${workspace}`, threadId);
}

export function savedWorkspace(): string | null {
  return localStorage.getItem(WORKSPACE_KEY)?.trim() || null;
}

export function saveWorkspace(workspace: string): void {
  localStorage.setItem(WORKSPACE_KEY, workspace);
}

export async function ensureWorkspace(options: {
  current: string;
  projectWorkspace: string;
  resolveDefault: () => Promise<string>;
  onChange: (workspace: string) => void;
}): Promise<string> {
  const usable = (value: string | null | undefined): value is string => Boolean(
    value && value !== "/" && !value.includes("/outputs/Jarvis/"),
  );
  let workspace = options.current;
  if (!usable(workspace)) {
    const configured = savedWorkspace();
    workspace = usable(configured)
      ? configured
      : (await options.resolveDefault()) || options.projectWorkspace;
  }
  saveWorkspace(workspace);
  options.onChange(workspace);
  return workspace;
}
