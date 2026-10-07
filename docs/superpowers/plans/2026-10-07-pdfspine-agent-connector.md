# Pdfspine OCR connector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (native implementation in this session).

**Goal:** 将 pdfspine 的图片/PDF OCR 能力接入 Jarvis 的 Rust Agent 工具网关。

**Architecture:** pdfspine 提供独立 Rust JSONL worker；Jarvis Rust connector 管理 worker 生命周期、workspace 路径、限制、事件和工具调用。模型只接触三个 OCR tool schema。

**Tech Stack:** Rust 1.96、serde_json、pdf-api/pdf-ocr、PaddleOCR PP-OCRv5、Jarvis Tauri tool gateway。

**Spec:** `docs/superpowers/specs/2026-10-07-pdfspine-agent-connector-design.md`

## Global Constraints

- pdfspine 和 Jarvis 保持独立仓库。
- Python 不参与 OCR 主路径；pdfspine worker 使用 Rust。
- 工具只能访问当前 workspace，输出不得覆盖输入 PDF。
- 模型自主选择工具，禁止关键词预判。
- 所有用户可见文案、工具描述和连接器策略写入 `config/*.json`。
- Rust 保留大小、页数、DPI、超时和路径安全硬限制。

## Review Focus

- 扫描 PDF 没有文字层：worker 必须返回页级 OCR 文本。
- 图片输入不是 PDF：worker 必须通过 pdfspine image-document 路径识别。
- 大文件、超页数和超 DPI：必须在启动 OCR 前拒绝。
- OCR 过程中取消或 worker 崩溃：Jarvis 必须清理子进程和 pending 请求。
- 输出路径覆盖输入文件或越出 workspace：必须拒绝并记录事件。

### Task 1: pdfspine Rust worker

**Files:**
- Create: `/Users/caihaolun/pdfspine/crates/pdf-ocr-worker/Cargo.toml`
- Create: `/Users/caihaolun/pdfspine/crates/pdf-ocr-worker/src/main.rs`
- Modify: `/Users/caihaolun/pdfspine/Cargo.toml`
- Test: `/Users/caihaolun/pdfspine/crates/pdf-ocr-worker/tests/protocol.rs`

- [ ] Add JSONL protocol with `status`, `ocr_image`, `ocr_pdf`, `make_searchable_pdf`, `cancel`.
- [ ] Use `pdf-api` with the `paddle-ocr` feature; validate file size, page range and DPI before loading.
- [ ] Keep stdout machine-readable and send diagnostics to stderr.
- [ ] Test malformed JSON, unsupported operation, image OCR fixture, PDF OCR fixture and output collision rejection.
- [ ] Run `cargo test -p pdf-ocr-worker`.

### Task 2: Jarvis pdfspine connector

**Files:**
- Create: `/Users/caihaolun/Jarvis-codex/src-tauri/src/pdfspine.rs`
- Modify: `/Users/caihaolun/Jarvis-codex/src-tauri/src/lib.rs`
- Modify: `/Users/caihaolun/Jarvis-codex/src-tauri/Cargo.toml`
- Test: `/Users/caihaolun/Jarvis-codex/src-tauri/src/pdfspine.rs`

- [ ] Add `PdfSpineConnector` with one serialized child process, request IDs, deadline, cancellation and shutdown cleanup.
- [ ] Resolve `JARVIS_PDFSPINE_BIN` from `.env`/environment; return a bounded unavailable error when unset.
- [ ] Validate workspace paths and output collision before sending a request.
- [ ] Emit connector lifecycle events through `jarvis-event` and `jarvis-events.jsonl`.
- [ ] Test timeout, worker EOF, malformed response, cancellation and path traversal.

### Task 3: Dynamic OCR tool registration

**Files:**
- Modify: `/Users/caihaolun/Jarvis-codex/src-tauri/src/tools.rs`
- Modify: `/Users/caihaolun/Jarvis-codex/config/tools.json`
- Modify: `/Users/caihaolun/Jarvis-codex/config/connectors.json`
- Test: `/Users/caihaolun/Jarvis-codex/src-tauri/src/tools.rs`

- [ ] Register `ocr_image`, `ocr_pdf`, and `make_searchable_pdf` schemas with bounded arguments.
- [ ] Route execution through `PdfSpineConnector`, never through shell commands or model-generated paths.
- [ ] Declare pdfspine availability and audit stream in connector config.
- [ ] Test schemas, argument limits and connector error mapping.

### Task 4: Context and UI traceability

**Files:**
- Modify: `/Users/caihaolun/Jarvis-codex/src-tauri/src/on_device_model.rs`
- Modify: `/Users/caihaolun/Jarvis-codex/src/main.ts`
- Modify: `/Users/caihaolun/Jarvis-codex/config/ui.json`
- Test: `/Users/caihaolun/Jarvis-codex/tests/wav.test.mjs`

- [ ] Include tool registry and connector readiness in the existing model context without adding keyword routing.
- [ ] Render OCR lifecycle events in the existing chronological event stream.
- [ ] Add configured unavailable/complete/error copy.
- [ ] Test that the UI displays OCR progress and that local Qwen receives the same tool schemas.

### Task 5: Integration verification and documentation

**Files:**
- Modify: `/Users/caihaolun/Jarvis-codex/README.md`
- Modify: `/Users/caihaolun/Jarvis-codex/docs/ARCHITECTURE.md`
- Modify: `/Users/caihaolun/Jarvis-codex/.env.example`

- [ ] Document pdfspine installation/build with the PaddleOCR feature and `JARVIS_PDFSPINE_BIN`.
- [ ] Run pdfspine tests, Jarvis Rust tests, Python worker tests, JavaScript tests, web build, clippy and diff check.
- [ ] Run an end-to-end smoke test on one image and one scanned PDF.
- [ ] Commit pdfspine and Jarvis changes as separate repository commits.
