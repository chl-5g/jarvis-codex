# Jarvis 编码规范

本项目遵守 `/Users/caihaolun/notes/01-个人/feedback-coding-standards.md` 的完整规范；该文件是细节唯一来源。这里保留项目执行时必须落实的摘要。

## 必须遵守

- 解耦优先：可选能力独立模块化，通过抽象接口、回调或注册表接入；未启用时不进入主流程。
- 手术刀式修改：只改完成目标所需的代码，保留现有注释，不顺手重构或删除文件。
- 配置外置：用户可见文案、固定选项、模型和权限策略放在项目 `config/*.json`，代码只读取配置键；项目根目录可由 `$PROJECTPATH` 或 `JARVIS_PROJECT_ROOT` 覆盖。
- 权限按需：启动只读取状态，只有明确使用能力时才申请操作系统权限。
- 按风险选择验证时机：不要求每个小改动或每个阶段集中跑全量测试；在准备交付、提交或声称完成前运行必要检查。
- 提交按完整功能批次进行，不要求每个小任务单独提交。禁止声称没有实际验证过的功能已完成。

## Jarvis 边界

- Rust 编排器拥有模型路由、记忆、Skills、工具网关、权限和事件流。
- 原生 Codex Voice 是主语音路径；端侧模型和本地语音是降级路径。
- Python worker 只负责转写和合成，不接收提示词、工具参数或权限决定。
- 运行日志、事件日志和对话日志统一写入 `config/paths.json` 指定的日志目录。
- 模型只负责理解意图和选择能力；平台 API、操作系统能力和 personal agent 通信都必须通过 `config/connectors.json` 注册的连接器执行。
- 订酒店、打车、日历、支付等现实事务属于平台连接器；连接器负责认证、权限、确认、执行和审计，不能把这些责任塞进模型提示词。

## 收尾检查

```text
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
npm run web:build
git diff --check
```
