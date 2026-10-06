# Contributing to Jarvis × Codex

Thank you for helping improve Jarvis × Codex. Contributions of focused bug
fixes, compatibility updates, tests, documentation, and carefully scoped
features are welcome.

## Before starting

For a small, well-defined fix, you can open a pull request directly. For a
large feature, protocol change, UI redesign, or architectural refactor, open an
issue first so maintainers and contributors can agree on the problem and the
smallest viable solution before substantial work begins.

Do not use a public issue for security vulnerabilities. Follow
[SECURITY.md](SECURITY.md) instead.

## Contribution workflow

The `main` branch is protected. Direct pushes are not allowed, including for
maintainers. Every change must be reviewed and merged through a pull request.

1. Fork `Big-Guan/jarvis-codex` on GitHub.
2. Clone your fork.
3. Add the upstream repository.
4. Create a focused branch from the latest upstream `main`.
5. Make and test your change.
6. Push the branch to your fork.
7. Open a pull request against `Big-Guan/jarvis-codex:main`.

Example:

```bash
git clone https://github.com/YOUR_USERNAME/jarvis-codex.git
cd jarvis-codex
git remote add upstream https://github.com/Big-Guan/jarvis-codex.git
git fetch upstream
git switch -c fix/short-description upstream/main
```

After making your changes:

```bash
npm ci
npm run check
npm run build
git push -u origin fix/short-description
```

Keep each branch and pull request limited to one coherent change. Separate
unrelated fixes instead of combining them into a large PR.

## Development environment

You need:

- macOS 13 or later
- Node.js 20 or later
- Rust stable with the `rustfmt` and `clippy` components
- Xcode Command Line Tools and Swift
- A local Codex App, ChatGPT App, or Codex CLI installation for runtime tests

Install dependencies and start the application with:

```bash
npm ci
npm run dev
```

To select the initial workspace:

```bash
JARVIS_WORKSPACE=/absolute/path npm run dev
```

## Design principles

- Start from the user problem and the runtime constraints.
- Prefer the simplest solution with the fewest assumptions.
- Avoid speculative abstractions and unrelated refactors.
- Preserve the single-thread Voice and text model.
- Never let the wake helper and WebRTC session capture the microphone at the
  same time.
- Treat Codex realtime conversation as an experimental upstream interface.
- Validate permission profiles and trust boundaries in Rust, not only in the
  frontend.
- Avoid destructive or irreversible behavior unless the user explicitly asks
  for the exact action and target.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before changing runtime,
thread, microphone, permission, or process-lifecycle code.

## Tests and verification

All pull requests must pass:

```bash
npm run check
npm run build
```

`npm run check` builds the Swift wake helper, runs Node tests, builds the web
frontend, checks Rust formatting, and runs Clippy with warnings treated as
errors.

Add or update tests when behavior changes. A source-code assertion is useful
for protecting a protocol invariant, but it is not a substitute for an
appropriate unit, integration, or real-device test.

Perform a real macOS smoke test when a change affects any of these areas:

- microphone or speech-recognition permission
- warm wake or cold wake
- WebRTC or Codex realtime protocol messages
- transcript handoff and Voice interruption
- STOP and late-turn interruption
- workspace thread creation or resumption
- permission profiles or approvals
- login startup, helper signing, application signing, or packaging

Describe the manual test environment and result in the pull request. If you
cannot run a required real-device test, state that clearly so a maintainer can
perform it before merge.

## Pull request requirements

A pull request should:

- explain the user-visible problem and why the change is needed
- describe the smallest solution implemented
- list automated and manual tests performed
- include screenshots or recordings for visible UI changes
- call out changes to permissions, privacy, signing, or upstream protocol use
- update English and Chinese documentation when user-facing behavior changes
- avoid generated files, credentials, signing keys, DMGs, and local runtime
  artifacts

PRs require passing CI and at least one approving review before merge. Resolve
review conversations and keep the branch up to date with `main` when requested.

## Commit guidance

Use short, imperative commit subjects. Conventional prefixes are encouraged:

```text
fix: release the wake microphone before Voice starts
feat: add a Voice connection diagnostic
docs: clarify the fork and pull request workflow
test: cover late turns after STOP
```

Clean up temporary or noisy commits before the PR is merged when a maintainer
requests it. Maintainers may squash-merge a pull request.

## Documentation languages

`README.md` is the default English README. `README.zh-CN.md` is the Simplified
Chinese version. User-facing documentation changes should keep both versions
consistent.

## License

By submitting a contribution, you agree that it may be distributed under the
project's [GNU General Public License v3.0](LICENSE).

## Review and merge

Maintainers review correctness, scope, security boundaries, regressions, and
test evidence. Approval is not guaranteed solely because CI passes. Once the
required review and checks pass, a maintainer will merge the pull request.
