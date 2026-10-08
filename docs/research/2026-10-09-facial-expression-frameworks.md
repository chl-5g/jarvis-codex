# 本地面部表情编码框架调研

调研日期：2026-10-09

目标：摄像头画面在本机完成面部分析，只把低体积、可解释的结构化状态交给 Jarvis/LLM，而不是把每帧图片或整段视频交给模型阅读。

## 结论

建议把“面部运动编码”和“情绪分类”拆成两层：

1. 第一层优先使用 MediaPipe Face Landmarker，输出 478 个 3D landmark（实现内部还提供 52 个 blendshape 系数）、头部变换矩阵和置信度。它适合实时流和跨语言/跨平台的本地前处理；52 个系数比直接传图片更稳定、体积更小，也能保留“眉毛、眼睛、嘴部、下颌”等可解释信号。
2. 第二层按需增加 EmotiEffLib，把已经裁剪的人脸送入本地 ONNX/PyTorch 情绪分类器，输出少量离散情绪和概率。它提供 Python 与 C++ 实现，仓库明确支持 ONNX 后端和实时照片/视频分析，Apache-2.0 对产品集成更宽松。
3. 对需要 FACS Action Unit（AU）的人脸行为研究，可评估 OpenFace；它能给出 AU presence/intensity、头部姿态和凝视，但官方仓库要求商业使用另行联系，不能默认作为 Jarvis 产品依赖。

因此第一版不应让 LLM直接“读脸”：视觉 worker 只在本机采样、平滑、聚合，并发送如下事件 JSON。原始图片默认不出本机，也不进入长期记忆。

```json
{
  "schema": "jarvis.face_state.v1",
  "ts": "2026-10-09T12:34:56.789Z",
  "speaker": {"id": "primary", "voice_verified": true},
  "utterance": {"text": "我今天有点累", "is_final": true},
  "face": {
    "present": true,
    "confidence": 0.97,
    "head_pose_deg": {"yaw": -3.2, "pitch": 5.1, "roll": 0.8},
    "blendshapes": {
      "eyeBlinkLeft": 0.02,
      "eyeBlinkRight": 0.03,
      "browInnerUp": 0.18,
      "mouthSmileLeft": 0.04,
      "mouthSmileRight": 0.05,
      "jawOpen": 0.11
    },
    "emotion": {
      "label": "neutral",
      "scores": {"neutral": 0.61, "sad": 0.27, "happy": 0.04},
      "model": "emotiefflib:enet_b0_8",
      "confidence": 0.61
    }
  },
  "person_profile": {"id": "primary", "stable_traits": ["prefers_concise_replies"]},
  "privacy": {"source": "camera", "local_only": true, "raw_frame_sent": false}
}
```

`emotion.label` 应被视为模型的低置信度提示，而不是事实或心理诊断。长期状态只保留经时间窗口聚合后的事件（例如“连续 10 秒眨眼增多”），不要保存完整 blendshape 时间序列或人脸图像。

## 候选框架

### MediaPipe Face Landmarker（首选基础层）

- 官方实现：[google-ai-edge/mediapipe](https://github.com/google-ai-edge/mediapipe)，Python API 的 `FaceLandmarker` 明确定义了 52 个 blendshape 系数（眉、眼、脸颊、嘴、鼻、下颌等）[源码枚举](https://github.com/google-ai-edge/mediapipe/blob/master/mediapipe/tasks/python/vision/face_landmarker.py)。
- 官方任务文档说明输出 3D face landmarks、可选 blendshape scores 和 facial transformation matrices，并支持图片、视频及 live stream 模式：[Face Landmarker Python guide](https://developers.google.com/mediapipe/solutions/vision/face_landmarker/python)。官方样例使用 `output_face_blendshapes=True` 和 `LIVE_STREAM` 回调：[mediapipe-samples](https://github.com/google-ai-edge/mediapipe-samples/blob/main/examples/face_landmarker/raspberry_pi/detect.py)。
- 运行形态：本地 TFLite 模型；Python、C++、Android、iOS、Web 等均有任务实现。仓库代码采用 Apache-2.0（样例头部也明确标注），但模型文件/模型卡仍需按其条款核对。
- 输出适配：不要把 478 landmarks 全部发给 LLM；worker 可只保留 52 blendshape 中变化最大的若干项、头部姿态、检测置信度，并做 5–10 帧 EMA/时间窗口平滑。
- 限制：blendshape 是面部运动系数，不等价于“开心/悲伤”等心理情绪；遮挡、侧脸、光照和个体差异会影响结果。MediaPipe 官方 issue 也记录过部分系数缺失或不稳定的报告，例如 `tongueOut`：[issue #4403](https://github.com/google-ai-edge/mediapipe/issues/4403)。因此应保留 `confidence`、`model` 和 `missing` 字段，避免把系数当作确定事实。

### EmotiEffLib（首选情绪分类层）

- 官方仓库已迁移至：[sb-ai-lab/EmotiEffLib](https://github.com/sb-ai-lab/EmotiEffLib)。README 定义它为轻量级照片/视频 emotion and engagement recognition library，提供 Python、C++ 两套实现，以及 PyTorch 和 ONNX 后端：[README](https://github.com/sb-ai-lab/EmotiEffLib#emotiefflib-library-for-efficient-emotion-analysis-and-facial-expression-recognition)。
- 仓库列出可下载模型及 AffectNet、AFEW、VGAF 等评测；轻量 `mobilenet_7.h5` 为 14 MB，README 给出的 Android CPU 推理均值约 16±5 ms，`enet_b0` 约 59±26 ms（这是 Samsung Fold 3 的作者数据，不应直接当作 Apple Silicon 基准）：[性能表](https://github.com/sb-ai-lab/EmotiEffLib#details)。
- 许可证：代码为 Apache-2.0，README 明确写明学术和商业使用均无额外限制：[license section](https://github.com/sb-ai-lab/EmotiEffLib#license)。模型和训练数据仍需在落地时分别检查。
- 输出适配：保留 top-k 情绪及分数（而非整张概率向量），设置最低置信度和 `unknown` 分支；模型输入由 MediaPipe/本地 detector 提供的人脸 crop，避免重复开摄像头和检测器。
- 限制：情绪标签是训练数据上的分类结果，受文化、光照、姿态和数据集偏差影响；仓库自己也指出 AFEW/VGAF 指标只统计成功检测到人脸的子集，完整测试集准确率会更低。因此只把它作为对话语气的弱信号。

### OpenFace 2.2.0（AU/FACS 研究选项）

- 官方仓库：[TadasBaltrusaitis/OpenFace](https://github.com/TadasBaltrusaitis/OpenFace)。README 列出 landmark、head pose、facial Action Unit recognition、eye gaze，并声称可用普通 webcam 实时运行，支持 macOS 安装：[README](https://github.com/TadasBaltrusaitis/OpenFace#openface-220-a-facial-behavior-analysis-toolkit)。
- AU 文档说明它可从图片、序列、视频抽取 AU；当前识别 AU 1、2、4、5、6、7、9、10、12、14、15、17、20、23、25、26、28、45，并同时输出 presence（0/1）和 intensity（0–5）：[Action Units wiki](https://github.com/TadasBaltrusaitis/OpenFace/wiki/Action-Units)。视频模式默认启用按人校准的 dynamic 模型，单张图准确度较低。
- 许可证风险：官方 README 有单独的 “Commercial license” 章节，要求商业许可咨询；同时还要求遵守 dlib、OpenBLAS、OpenCV 及训练数据集许可：[commercial license / copyright](https://github.com/TadasBaltrusaitis/OpenFace#commercial-license)。在 Jarvis 产品中应先完成许可审查。
- 适配建议：若研究需要 FACS，可作为隔离的本地 sidecar，通过 CSV/NDJSON 输出 AU；不要把它设为核心运行时依赖，除非许可和编译维护成本已确认。

### DeepFace 与 FER（快速验证/备选）

- [serengil/deepface](https://github.com/serengil/deepface) 提供 `analyze(..., actions=["emotion"])`，返回每张脸的 emotion 分数与 `dominant_emotion`，支持 OpenCV、RetinaFace、MTCNN、dlib、MediaPipe 等 detector backend：[demography.py](https://github.com/serengil/deepface/blob/master/deepface/modules/demography.py)；README列出 angry/fear/neutral/sad/disgust/happy/surprise 七类情绪：[README](https://github.com/serengil/deepface#facial-attribute-analysis)。优点是 API 快速、功能齐全；缺点是依赖和模型较重，适合离线原型，不适合 Jarvis 第一版的低延迟最小 worker。使用前需审查其 MIT 代码与各模型/依赖条款。
- [justinshenk/fer](https://github.com/justinshenk/fer) 是 Python 包，返回 `{box, emotions}`，情绪键为 anger/disgust/fear/happy/sad/surprise/neutral；README 说明依赖 OpenCV、TensorFlow，并捆绑 Keras HDF5 模型，许可证为 MIT：[README](https://github.com/justinshenk/fer)。它容易试用，但依赖较旧、模型和检测器耦合，不建议作为生产实时链路的基础。

## 对 Jarvis 的落地边界

建议新增独立的本地 `vision_worker`，摄像头权限仍由现有授权门控控制：声纹验证通过且用户明确同意后才打开摄像头；未授权时 worker 不启动。当前 Jarvis 已用浏览器摄像头流以约 2 秒一个低频 JPEG 窗口采样，并把帧交给本机 Rust/Python worker；只保留最新的紧凑 face 状态，撤销授权、声纹失效或语音连接清理时立即停止视频轨道。一次性原生摄像头能力仍保留给明确的视觉工具调用。worker 内部只输出 `face_state.v1` NDJSON/IPC 事件：

```text
camera frame -> face detector/landmarker -> temporal smoothing
             -> optional emotion classifier -> compact JSON -> controller/LLM
```

推荐默认采样策略：摄像头 10–15 FPS，MediaPipe 每帧或隔帧追踪；只在说话中、用户要求“看看我的表情”、或状态明显变化时向 controller 发送一条聚合事件。LLM 输入应包含说话内容、声纹身份、最近一次聚合后的表情状态和人物画像；原始帧、478 点坐标、完整 52 维序列留在本机内存并及时丢弃。

第一阶段验收指标应是端到端延迟、CPU/内存占用、无脸/侧脸/遮挡时的 `unknown` 行为，以及“拒绝摄像头授权时绝不产生 frame 事件”，而不是把情绪分类准确率当作人格或心理判断。

## 参考来源

- [MediaPipe Face Landmarker Python guide](https://developers.google.com/mediapipe/solutions/vision/face_landmarker/python)
- [MediaPipe FaceLandmarker Python source](https://github.com/google-ai-edge/mediapipe/blob/master/mediapipe/tasks/python/vision/face_landmarker.py)
- [MediaPipe Face Landmarker sample](https://github.com/google-ai-edge/mediapipe-samples/blob/main/examples/face_landmarker/raspberry_pi/detect.py)
- [EmotiEffLib repository](https://github.com/sb-ai-lab/EmotiEffLib)
- [OpenFace repository](https://github.com/TadasBaltrusaitis/OpenFace)
- [OpenFace Action Units wiki](https://github.com/TadasBaltrusaitis/OpenFace/wiki/Action-Units)
- [DeepFace demography implementation](https://github.com/serengil/deepface/blob/master/deepface/modules/demography.py)
- [FER repository](https://github.com/justinshenk/fer)
