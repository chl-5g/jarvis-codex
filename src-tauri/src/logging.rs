use serde::Serialize;
use std::{
    fs::{create_dir_all, OpenOptions},
    io::Write,
    path::PathBuf,
};

fn directory() -> PathBuf {
    if let Some(value) = std::env::var_os("JARVIS_LOG_DIR") {
        return PathBuf::from(value);
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Jarvis-codex/logs")
}

pub fn append<T: Serialize>(stream: &str, value: &T) {
    let dir = directory();
    if create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("{stream}.jsonl"));
    let Ok(line) = serde_json::to_string(value) else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

pub fn text(stream: &str, message: &str) {
    #[derive(Serialize)]
    struct Entry<'a> {
        message: &'a str,
    }
    append(stream, &Entry { message });
}

pub fn init() {
    text("jarvis-runtime", "Jarvis process started");
    std::panic::set_hook(Box::new(|panic| {
        text("jarvis-runtime", &format!("panic: {panic}"));
    }));
}
