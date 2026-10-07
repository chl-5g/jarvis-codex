//! Bounded local VAD worker; identity extraction is a separate operation.
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time::{timeout, Duration},
};

pub struct SpeakerActivity(Mutex<Option<Worker>>);

struct Worker {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    speech_logged: bool,
}

impl SpeakerActivity {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }

    pub async fn stop(&self) {
        if let Some(mut worker) = self.0.lock().await.take() {
            let _ = worker.child.kill().await;
        }
    }

    pub async fn start(&self, app: &AppHandle) -> Result<Value, String> {
        self.stop().await;
        let resource = app.path().resource_dir().map_err(|e| e.to_string())?;
        let bundled = resource.join("speaker_activity.py");
        let script = if bundled.is_file() {
            bundled
        } else {
            std::path::PathBuf::from(crate::config::project_root())
                .join("src-tauri/speaker_activity.py")
        };
        let mut child = Command::new(crate::offline_speech::speaker_python_path())
            .arg(script)
            .arg("--config")
            .arg(include_str!("../../config/voice.json"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut worker = Worker {
            input: child.stdin.take().ok_or("speaker.vad.stdin")?,
            output: BufReader::new(child.stdout.take().ok_or("speaker.vad.stdout")?).lines(),
            child,
            speech_logged: false,
        };
        let result = worker.read().await?;
        *self.0.lock().await = Some(worker);
        crate::logging::text(
            "jarvis-runtime",
            "speaker activity: listening (no voiceprint capture yet)",
        );
        Ok(result)
    }

    pub async fn feed(&self, pcm: String) -> Result<Value, String> {
        if pcm.len() > 43000 {
            return Err("speaker.vad.frame-too-large".to_owned());
        }
        let mut guard = self.0.lock().await;
        let worker = guard.as_mut().ok_or("speaker.vad.not-started")?;
        let request = format!("{}\n", json!({ "pcm": pcm }));
        worker
            .input
            .write_all(request.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        worker.input.flush().await.map_err(|e| e.to_string())?;
        let result = worker.read().await?;
        if result
            .get("speaking")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && !worker.speech_logged
        {
            worker.speech_logged = true;
            crate::logging::text(
                "jarvis-runtime",
                "speaker activity: speaking detected; voiceprint extraction is now allowed",
            );
        }
        if result
            .get("ready")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            crate::logging::text(
                "jarvis-runtime",
                "speaker activity: speech segment ended; starting voiceprint verification",
            );
            crate::logging::text(
                "jarvis-runtime",
                "speaker activity: speech sample ready; starting voiceprint verification",
            );
        }
        Ok(result)
    }
}

impl Worker {
    async fn read(&mut self) -> Result<Value, String> {
        let line = timeout(Duration::from_secs(5), self.output.next_line())
            .await
            .map_err(|_| "speaker.vad.timeout")?
            .map_err(|e| e.to_string())?
            .ok_or("speaker.vad.exited")?;
        serde_json::from_str(&line).map_err(|e| e.to_string())
    }
}
