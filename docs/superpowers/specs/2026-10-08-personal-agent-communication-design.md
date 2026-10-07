# Jarvis Personal Agent 通信层设计

## 目标

让每个设备上的 Jarvis 都能作为独立的 Personal Agent，通过 CipherPipe 加密通信，向其他 Jarvis 委托受限任务并接收结果。

第一阶段只实现“指定 Agent 任务请求 → 接收 Agent 校验并执行 → 返回任务结果”的闭环。

## 边界

CipherPipe 负责端到端加密、消息路由和送达；Jarvis 负责 Agent 身份登记、能力声明、任务状态、权限审批、执行和审计。远程消息不得直接触发任意 shell 命令。

第一阶段不实现自动全网发现、群聊、复杂 DAG 编排、跨用户信任联盟和自动化高风险操作。

## 消息协议

所有 Agent 消息使用 JSON envelope，字段包括：

- `version`: 协议版本，首版为 `1`
- `message_id`: 唯一消息 ID
- `kind`: `agent_hello`、`task_request`、`task_result`、`approval_request`、`task_cancel`
- `task_id`: 任务关联 ID；hello 消息可省略
- `from` / `to`: CipherPipe 公钥身份
- `created_at` / `expires_at`: Unix 秒
- `payload`: 类型对应内容

`task_request.payload` 包含 `capability`、JSON `input`、`requires_approval`。`task_result.payload` 包含 `status`（completed/failed/rejected/expired）、`output` 或 `error`。

接收方必须拒绝未知版本、空身份、过期任务、重复 `message_id` 和不允许的能力。

## Agent 注册和能力

本地维护 Agent registry，记录公钥、显示名称、设备、信任状态、能力列表和最后在线时间。第一阶段支持显式配置/本地持久化注册，不做自动发现。

能力使用稳定名称和 JSON schema；执行前必须经过本地 capability allowlist。高风险能力必须产生审批请求，由用户确认后才能执行。

## 运行流程

1. Jarvis A 根据 registry 选择目标 Agent 和能力。
2. 构造并校验 `task_request`，通过现有 CipherPipe adapter 发送。
3. Jarvis B 收到消息，验证 envelope、身份、过期时间、去重状态和能力 ACL。
4. 若需要审批，发出 `approval_request` 并暂停任务；否则交给本地受限执行器。
5. B 发送 `task_result`；A 按 `task_id` 更新状态并发出 Jarvis 事件。
6. 超时、取消和重复消息均保持幂等，不重复执行任务。

## 安全要求

禁止使用 CipherPipe `agent.py` 的任意 shell 执行模式。远程任务只能调用 Jarvis 已注册且经过权限检查的工具。任务必须有截止时间、最大输入大小和审计事件。CipherPipe 的 relay 只作为密文传输层，不能被视为任务授权来源。

## 文件规划

- 新增 Rust 协议、registry、任务状态模块及单元测试。
- 扩展现有 CipherPipe adapter，发送/接收结构化 envelope。
- 在 Tauri 命令和事件流中暴露 Agent 注册、任务发送、审批和结果状态。
- 增加前端最小任务状态展示和手动发送入口。
- 增加协议校验、过期、重复、ACL 和执行失败测试。

## 成功标准

本机可注册一个远端 Agent，发送一个结构化任务；接收端在能力允许时执行并返回结果；拒绝过期、重复、未授权能力和任意 shell 请求；发送方能在 UI 事件流看到 queued/running/completed 或 rejected 状态。

