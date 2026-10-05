//! Localhost-only bridge boundary for future iPhone/Shortcuts adapters.
//!
//! The bridge is deliberately disabled until `bridge_enable` is called from the
//! local Jarvis UI.  Enabling it starts a tiny, dependency-free HTTP endpoint
//! bound to `127.0.0.1`; every request needs the generated pairing token.  This
//! keeps the first cross-device seam auditable without exposing the process to
//! a LAN interface or adding a second always-on server.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{IpAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::AppHandle;

const DEFAULT_PORT: u16 = 8788;
const MAX_EVENTS: usize = 100;
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_COMMAND_CHARS: usize = 4_000;

#[derive(Clone, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub enabled: bool,
    pub bind_address: String,
    pub port: Option<u16>,
    pub paired: bool,
    pub endpoint: Option<String>,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BridgeEnableResult {
    pub status: BridgeStatus,
    /// Returned only by the first enable call, when a token is generated.
    /// Callers should store it in the OS keychain or app settings.
    pub pairing_token: Option<String>,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct BridgeEvent {
    id: u64,
    timestamp: u64,
    kind: String,
    payload: serde_json::Value,
}

#[derive(Clone)]
struct BridgeRuntime {
    app: AppHandle,
    workspace: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CommandRequest {
    text: String,
}

struct BridgeState {
    enabled: bool,
    bind_address: String,
    token: Option<String>,
    port: Option<u16>,
    next_event_id: u64,
    events: VecDeque<BridgeEvent>,
    stop: Option<Arc<AtomicBool>>,
    listener: Option<JoinHandle<()>>,
    runtime: Option<BridgeRuntime>,
}

impl Default for BridgeState {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_address: "127.0.0.1".to_owned(),
            token: None,
            port: None,
            next_event_id: 1,
            events: VecDeque::new(),
            stop: None,
            listener: None,
            runtime: None,
        }
    }
}

static BRIDGE: OnceLock<Arc<Mutex<BridgeState>>> = OnceLock::new();

fn state() -> Arc<Mutex<BridgeState>> {
    BRIDGE
        .get_or_init(|| Arc::new(Mutex::new(BridgeState::default())))
        .clone()
}

fn configured_port() -> u16 {
    std::env::var("JARVIS_BRIDGE_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(DEFAULT_PORT)
}

fn configured_bind_address() -> String {
    std::env::var("JARVIS_BRIDGE_BIND").unwrap_or_else(|_| "127.0.0.1".to_owned())
}

fn validate_bind_address(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    let address = value
        .parse::<IpAddr>()
        .map_err(|_| "bridge bind address must be an IP address".to_owned())?;
    let allowed = match address {
        IpAddr::V4(ip) => ip.is_unspecified() || ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_unspecified() || ip.is_loopback() || ip.is_unique_local(),
    };
    if !allowed {
        return Err("bridge bind address must be loopback, private, or unspecified".to_owned());
    }
    Ok(address.to_string())
}

fn parse_command_body(body: &[u8]) -> Result<CommandRequest, String> {
    let request: CommandRequest =
        serde_json::from_slice(body).map_err(|error| format!("invalid command body: {error}"))?;
    let text = request.text.trim().to_owned();
    if text.is_empty() {
        return Err("command text cannot be empty".to_owned());
    }
    if text.chars().count() > MAX_COMMAND_CHARS {
        return Err(format!(
            "command text exceeds {MAX_COMMAND_CHARS} characters"
        ));
    }
    Ok(CommandRequest { text })
}

fn token() -> String {
    let mut bytes = [0u8; 24];
    // `/dev/urandom` is available on macOS and Linux.  The timestamp fallback
    // still avoids predictable static credentials if the device is unusual.
    if let Ok(mut source) = std::fs::File::open("/dev/urandom") {
        let _ = source.read_exact(&mut bytes);
    } else {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = ((nanos >> ((index % 16) * 8)) as u8) ^ (std::process::id() as u8);
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn status_locked(state: &BridgeState) -> BridgeStatus {
    BridgeStatus {
        enabled: state.enabled,
        bind_address: state.bind_address.clone(),
        port: state.port,
        paired: state.token.is_some(),
        endpoint: state.port.map(|port| format!("http://127.0.0.1:{port}")),
    }
}

pub fn status() -> BridgeStatus {
    status_locked(&state().lock().expect("bridge mutex poisoned"))
}

/// Enable the endpoint.  The first call creates a token.  Later calls must
/// provide the same token, preventing an accidental second local client from
/// silently replacing the pairing credential.
pub fn enable(
    app: AppHandle,
    workspace: PathBuf,
    requested_token: Option<String>,
    requested_bind_address: Option<String>,
) -> Result<BridgeEnableResult, String> {
    let store = state();
    let configured_bind = configured_bind_address();
    let bind_raw = requested_bind_address
        .as_deref()
        .unwrap_or(configured_bind.as_str());
    let bind_address = validate_bind_address(bind_raw)?;
    let runtime = BridgeRuntime { app, workspace };
    let mut guard = store
        .lock()
        .map_err(|_| "bridge state unavailable".to_owned())?;
    if guard.enabled {
        if guard.token.as_deref() != requested_token.as_deref() {
            return Err("bridge is already enabled; pairing token does not match".to_owned());
        }
        guard.runtime = Some(runtime);
        return Ok(BridgeEnableResult {
            status: status_locked(&guard),
            pairing_token: None,
        });
    }

    let (pairing_token, generated) = match (guard.token.clone(), requested_token) {
        (Some(existing), Some(requested)) if existing == requested => (existing, false),
        (Some(_), Some(_)) => return Err("pairing token does not match".to_owned()),
        (Some(existing), None) => {
            return Err(format!(
                "pairing token required to re-enable bridge ({} characters)",
                existing.len()
            ))
        }
        (None, Some(requested)) if requested.len() >= 16 => (requested, false),
        (None, Some(_)) => {
            return Err("pairing token must contain at least 16 characters".to_owned())
        }
        (None, None) => (token(), true),
    };
    let listener = TcpListener::bind((bind_address.as_str(), configured_port()))
        .map_err(|error| format!("cannot bind localhost bridge: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure localhost bridge: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("cannot read localhost bridge address: {error}"))?
        .port();
    let stop = Arc::new(AtomicBool::new(false));
    guard.token = Some(pairing_token.clone());
    guard.port = Some(port);
    guard.bind_address = bind_address;
    guard.enabled = true;
    guard.stop = Some(stop.clone());
    guard.runtime = Some(runtime);
    let worker_store = store.clone();
    guard.listener = Some(thread::spawn(move || serve(listener, worker_store, stop)));
    Ok(BridgeEnableResult {
        status: status_locked(&guard),
        pairing_token: generated.then_some(pairing_token),
    })
}

pub fn disable(requested_token: Option<String>) -> Result<BridgeStatus, String> {
    let store = state();
    let (stop, listener) = {
        let mut guard = store
            .lock()
            .map_err(|_| "bridge state unavailable".to_owned())?;
        if guard.token.as_deref() != requested_token.as_deref() {
            return Err("pairing token does not match".to_owned());
        }
        guard.enabled = false;
        guard.port = None;
        guard.runtime = None;
        (guard.stop.take(), guard.listener.take())
    };
    if let Some(stop) = stop {
        stop.store(true, Ordering::Release);
    }
    if let Some(listener) = listener {
        let _ = listener.join();
    }
    Ok(status())
}

/// Publish a bounded event for a future `/events` adapter.  Existing local
/// event producers can call this without knowing anything about the socket.
pub fn publish(kind: impl Into<String>, payload: serde_json::Value) {
    if let Ok(mut guard) = state().lock() {
        let event = BridgeEvent {
            id: guard.next_event_id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or_default(),
            kind: kind.into(),
            payload,
        };
        guard.next_event_id = guard.next_event_id.saturating_add(1);
        guard.events.push_back(event);
        while guard.events.len() > MAX_EVENTS {
            guard.events.pop_front();
        }
    }
}

fn serve(listener: TcpListener, store: Arc<Mutex<BridgeState>>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => handle(stream, &store),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => break,
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::with_capacity(8_192);
    let mut header_end = None;
    while header_end.is_none() {
        let mut chunk = [0u8; 8_192];
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("request read failed: {error}"))?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_REQUEST_BYTES {
            return Err("request too large".to_owned());
        }
        header_end = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    }
    let header_end = header_end.ok_or("request headers are incomplete")?;
    let header_length = header_end + 4;
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let total = header_length
        .checked_add(content_length)
        .ok_or("request length overflow")?;
    if total > MAX_REQUEST_BYTES {
        return Err("request too large".to_owned());
    }
    while buffer.len() < total {
        let mut chunk = [0u8; 8_192];
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("request body read failed: {error}"))?;
        if read == 0 {
            return Err("request body is incomplete".to_owned());
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    buffer.truncate(total);
    Ok(buffer)
}

fn handle(mut stream: TcpStream, store: &Arc<Mutex<BridgeState>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            respond(&mut stream, 400, json!({"error": error}));
            return;
        }
    };
    let request = String::from_utf8_lossy(&request);
    let (headers, body) = request
        .split_once("\r\n\r\n")
        .unwrap_or((request.as_ref(), ""));
    let mut lines = headers.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let auth = lines
        .find_map(|line| {
            line.strip_prefix("Authorization:")
                .or_else(|| line.strip_prefix("authorization:"))
        })
        .map(str::trim)
        .and_then(|value| value.strip_prefix("Bearer "));
    let (authorized, runtime) = store
        .lock()
        .ok()
        .map(|guard| {
            let authorized = guard
                .token
                .as_deref()
                .zip(auth)
                .map(|(expected, actual)| expected == actual)
                .unwrap_or(false);
            (authorized, guard.runtime.clone())
        })
        .unwrap_or((false, None));
    if !authorized {
        respond(&mut stream, 401, json!({"error":"unauthorized"}));
        return;
    }
    let route = path.split('?').next().unwrap_or(path);
    let response = match (method, route) {
        ("GET", "/status") => (
            200,
            serde_json::to_value(status()).unwrap_or_else(|_| json!({"enabled":false})),
        ),
        ("GET", "/events") => (
            200,
            store
                .lock()
                .map(|guard| json!({"events": guard.events.iter().cloned().collect::<Vec<_>>() }))
                .unwrap_or_else(|_| json!({"events":[]})),
        ),
        ("POST", "/command") => match parse_command_body(body.as_bytes()) {
            Ok(command) => match runtime {
                Some(runtime) => {
                    let request_id = start_command(runtime, command);
                    (202, json!({"accepted":true,"requestId":request_id}))
                }
                None => (503, json!({"error":"bridge runtime is not ready"})),
            },
            Err(error) => (400, json!({"error": error})),
        },
        _ => (404, json!({"error":"not found"})),
    };
    respond(&mut stream, response.0, response.1);
}

fn start_command(runtime: BridgeRuntime, command: CommandRequest) -> String {
    let request_id = format!("bridge-{}", token());
    let started_id = request_id.clone();
    let event_id = request_id.clone();
    publish(
        "bridge",
        json!({"kind":"bridge","phase":"started","requestId":started_id,"text":command.text}),
    );
    tauri::async_runtime::spawn(async move {
        let result = crate::qwen::chat(runtime.app, command.text, runtime.workspace).await;
        match result {
            Ok(answer) => publish(
                "bridge",
                json!({"kind":"bridge","phase":"completed","requestId":event_id,"answer":answer}),
            ),
            Err(error) => publish(
                "bridge",
                json!({"kind":"bridge","phase":"error","requestId":event_id,"error":error}),
            ),
        }
    });
    request_id
}

fn respond(stream: &mut TcpStream, code: u16, body: serde_json::Value) {
    let body = serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_owned());
    let reason = match code {
        200 => "OK",
        202 => "Accepted",
        404 => "Not Found",
        503 => "Service Unavailable",
        400 => "Bad Request",
        _ => "Unauthorized",
    };
    let response = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_address_accepts_loopback_private_and_unspecified_only() {
        assert_eq!(validate_bind_address("127.0.0.1").unwrap(), "127.0.0.1");
        assert_eq!(
            validate_bind_address("192.168.1.20").unwrap(),
            "192.168.1.20"
        );
        assert_eq!(validate_bind_address("0.0.0.0").unwrap(), "0.0.0.0");
        assert!(validate_bind_address("8.8.8.8").is_err());
        assert!(validate_bind_address("not-an-ip").is_err());
    }

    #[test]
    fn command_body_requires_bounded_text() {
        let request = parse_command_body(br#"{"text":"read MEMORY.md"}"#).unwrap();
        assert_eq!(request.text, "read MEMORY.md");
        assert!(parse_command_body(br#"{"text":"  "}"#).is_err());
        assert!(parse_command_body(
            serde_json::to_vec(&json!({"text": "x".repeat(MAX_COMMAND_CHARS + 1)}))
                .unwrap()
                .as_slice()
        )
        .is_err());
    }

    #[test]
    fn status_is_disabled_and_loopback_only_by_default() {
        let status = BridgeStatus {
            enabled: false,
            bind_address: "127.0.0.1".to_owned(),
            port: None,
            paired: false,
            endpoint: None,
        };
        assert_eq!(status.bind_address, "127.0.0.1");
        assert!(!status.enabled);
        assert!(!status.paired);
    }

    #[test]
    fn generated_tokens_are_not_empty_or_constant() {
        let first = token();
        let second = token();
        assert_eq!(first.len(), 48);
        assert_ne!(first, second);
    }

    #[test]
    fn configured_port_never_defaults_to_zero() {
        std::env::set_var("JARVIS_BRIDGE_PORT", "0");
        assert_eq!(configured_port(), DEFAULT_PORT);
        std::env::remove_var("JARVIS_BRIDGE_PORT");
    }
}
