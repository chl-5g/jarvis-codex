# Pdfspine OCR connector design

## Goal

让 Jarvis 能通过模型自主调用本地 pdfspine，对图片和扫描版 PDF 执行 OCR，并可生成带隐形文字层的可搜索 PDF。pdfspine 保持独立仓库和 Rust OCR 实现；Jarvis Rust 编排器拥有连接器生命周期、权限、超时、取消、工具注册和审计。

## Existing capability

`/Users/caihaolun/pdfspine` 已有 `pdf-api`、`pdf-ocr` 和 PaddleOCR PP-OCRv5 集成。OCR feature 默认关闭，图片可作为 image document 打开，PDF 可逐页渲染识别，且已有 searchable sandwich PDF 输出能力。

## Boundary

- pdfspine 新增一个受限 JSONL Rust worker，只负责文档读取、渲染、OCR 和 searchable PDF 输出。
- Jarvis 新增 `pdfspine` connector，负责启动 worker、校验路径、限制输入、超时、取消、关闭和事件审计。
- 模型只看到 `ocr_image`、`ocr_pdf`、`make_searchable_pdf` 的描述和参数，不接触进程、命令、权限或任意路径。
- 所有路径必须解析到当前 workspace；输出文件只能写入 workspace 下的配置目录。
- OCR 不可用时返回真实的 connector 错误，不启动第二个 Agent，不回退到关键词判断。

## Protocol

请求：`{"id":1,"op":"status|ocr_image|ocr_pdf|make_searchable_pdf",...}`。

响应：`{"id":1,"ok":true,"result":...}` 或 `{"id":1,"ok":false,"error":"..."}`。

固定限制：输入文件 100 MiB、PDF 200 页、DPI 72-300、单次文本输出 200,000 字符；worker 只接受 JSONL stdin，日志写 stderr。

## Tool behavior

- `ocr_image(path, language?, dpi?)` 返回识别文本、页/图像尺寸和受限置信度摘要。
- `ocr_pdf(path, language?, dpi?, pages?)` 返回页级文本和来源页号。
- `make_searchable_pdf(path, output_path?, language?, dpi?)` 在 workspace 内生成新 PDF，禁止覆盖输入文件。

工具调用、worker 状态、页级进度、错误和输出路径进入现有 `jarvis-events` 日志；模型上下文只接收必要的 OCR 结果。

## Failure handling

缺少 worker、PaddleOCR feature、模型文件、输入格式错误、超限、超时、取消和损坏输出都映射为可读的 connector error。Jarvis 保持运行并让模型说明实际原因。

## Verification

- pdfspine Rust worker 测试 JSONL 校验、图片 OCR、扫描 PDF OCR、输出 PDF、取消和限制。
- Jarvis Rust connector 测试路径边界、请求串行化、超时、取消、worker 退出和输出校验。
- tool schema 测试保证模型收到三个 OCR 工具。
- 端到端 smoke test：模型选择 OCR 工具、事件日志包含调用轨迹、生成文件可再次读取。
