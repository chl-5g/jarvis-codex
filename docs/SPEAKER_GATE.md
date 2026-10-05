# Allen 本地声纹门控

Jarvis 默认把说话人标记为 \`unknown\`：可以回答普通问题，但线程指令禁止 Computer Use。声纹模型只在本机运行；没有声纹档案或音频过短时保持 \`unknown\`。声纹比对失败时结果为 \`rejected\`，前端只回复“未识别的说话人”，不创建 Codex 任务。

先在本机 venv 中登记 Allen 的至少两段 16 kHz WAV：

\`\`\`sh
VENV=/path/to/jarvis-venv/bin/python
$VENV scripts/speaker_gate.py \
  --profile ~/.jarvis/allen-speaker.json \
  enroll allen-1.wav allen-2.wav allen-3.wav
\`\`\`

检查一段录音：

\`\`\`sh
$VENV scripts/speaker_gate.py \
  --profile ~/.jarvis/allen-speaker.json \
  verify wake.wav
\`\`\`

输出 JSON 的 \`speakerAccess\` 可直接映射到现有 \`jarvis-wake\` 事件：

- \`allen\`：允许 Computer Use；
- \`unknown\`：允许普通回答，禁止 Computer Use；
- \`rejected\`：不执行任务，只播报“未识别的说话人”。

唤醒监听器已经支持接收 \`speakerAccess\` 字段。集成时，让 \`AVAudioEngine\` 的 tap 在检测到唤醒词后把短片段写成临时 WAV，交给常驻的 \`speaker_gate.py serve\` 进程；常驻进程通过 stdin 接收一行 JSON（\`{"audio": "...", "profile": "..."}\`），返回一行结果。模型只加载一次，避免每次唤醒都重新初始化。
