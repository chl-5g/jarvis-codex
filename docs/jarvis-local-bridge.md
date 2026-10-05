# Jarvis 本机配对桥

Jarvis 的跨设备预留桥默认关闭，只绑定 `127.0.0.1`，不会监听局域网地址。

通过 Tauri 命令启用后，首次调用会生成 pairing token；也可以传入至少 16 个字符的自定义 token。桥接接口只接受带有 `Authorization: Bearer <token>` 的 `GET` 请求：

- `GET /status`：读取桥接状态
- `GET /events`：读取最近 100 条本地工具、工作流和任务事件

端口默认是 `8788`，可以用 `JARVIS_BRIDGE_PORT` 调整。调用 `bridge_disable` 时必须再次提供 token。适配 iPhone 或 Shortcuts 时，应把 token 存在系统钥匙串中，并继续通过本机安全隧道访问；不要把端口暴露到 `0.0.0.0`。
