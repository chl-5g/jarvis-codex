# Jarvis TODO

## 声纹采集与本地身份识别

- 语音降级入口：Rust `offline_speech.rs` 管理 `src-tauri/speech_worker.py`；旧控制器已移除
- 当前状态：语音控制器已有录音缓冲 `RECORDER`，但尚未接入声纹识别模块。
- 目标：在 Whisper 转录的同时，将原始 PCM 音频分流给本地 Resemblyzer。
- 安全要求：
  - 仅在明确开启“声纹注册模式”时采集；
  - 做 VAD、音量和有效语音时长检查；
  - 新向量通过相似度阈值后才更新；
  - 默认只保存声纹向量，不持续保存原始音频；
  - 提供删除声纹和关闭采集的入口。
- 依赖：Resemblyzer 已安装在 Jarvis 虚拟环境；代码仓库位于 `/Users/caihaolun/resemblyzer`。
- 相关本地数据目录：`/Users/caihaolun/.local/share/jarvis/voiceprint/`
