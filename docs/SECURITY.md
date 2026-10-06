# Security policy

## Supported versions

Only the latest release is supported.

## Reporting

Do not open a public issue for vulnerabilities that expose credentials, local
files or approval bypasses. Report them privately to the repository owner
through GitHub Security Advisories.

Include:

- affected version and macOS version;
- reproduction steps;
- whether microphone, files, Codex credentials or approval flow are affected;
- logs with tokens, paths and personal content redacted.

## Security invariants

- No API keys or Codex tokens are stored in this repository.
- Wake recognition stays on device.
- WebView remote code loading is blocked by CSP.
- Codex file access is restricted to the configured workspace sandbox.
- High-risk app-server requests require explicit user approval.
