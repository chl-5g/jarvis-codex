//! Localhost-only bridge boundary for future iPhone/Shortcuts adapters.
//!
//! The bridge is deliberately disabled until `bridge_enable` is called from the
//! local Jarvis UI.  Enabling it starts a tiny, dependency-free HTTP endpoint
//! bound to `127.0.0.1`; every request needs the generated pairing token.  This
//! keeps the first cross-device seam auditable without exposing the process to
//! a LAN interface or adding a second always-on server.

use serde::Serialize;
use serde_json::json;
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DEFAULT_PORT: u16 = 8788;
const MAX_EVENTS: usize = 100;

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

struct BridgeState {
    enabled: bool,
    token: Option<String>,
    port: Option<u16>,
    next_event_id: u64,
    events: VecDeque<BridgeEvent>,
    stop: Option<Arc<AtomicBool>>,
    listener: Option<JoinHandle<()>>,
}

impl Default for BridgeState {
    fn default() -> Self {
        Self {
            enabled: false,
            token: None,
            port: None,
            next_event_id: 1,
            events: VecDeque::new(),
            stop: None,
            listener: None,
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
        bind_address: "127.0.0.1".to_owned(),
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
pub fn enable(requested_token: Option<String>) -> Result<BridgeEnableResult, String> {
    let store = state();
    let mut guard = store
        .lock()
        .map_err(|_| "bridge state unavailable".to_owned())?;
    if guard.enabled {
        if guard.token.as_deref() != requested_token.as_deref() {
            return Err("bridge is already enabled; pairing token does not match".to_owned());
        }
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
    let listener = TcpListener::bind(("127.0.0.1", configured_port()))
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
    guard.enabled = true;
    guard.stop = Some(stop.clone());
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

fn handle(mut stream: TcpStream, store: &Arc<Mutex<BridgeState>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buffer = [0u8; 8192];
    let size = match stream.read(&mut buffer) {
        Ok(size) => size,
        Err(_) => return,
    };
    let request = String::from_utf8_lossy(&buffer[..size]);
    let mut lines = request.lines();
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
    let authorized = store
        .lock()
        .ok()
        .and_then(|guard| guard.token.clone())
        .as_deref()
        .zip(auth)
        .map(|(expected, actual)| expected == actual)
        .unwrap_or(false);
    if method != "GET" || !authorized {
        respond(&mut stream, 401, json!({"error":"unauthorized"}));
        return;
    }
    let body = match path.split('?').next().unwrap_or(path) {
        "/status" => serde_json::to_value(status()).unwrap_or_else(|_| json!({"enabled":false})),
        "/events" => store
            .lock()
            .map(|guard| json!({"events": guard.events.iter().cloned().collect::<Vec<_>>() }))
            .unwrap_or_else(|_| json!({"events":[]})),
        _ => {
            respond(&mut stream, 404, json!({"error":"not found"}));
            return;
        }
    };
    respond(&mut stream, 200, body);
}

fn respond(stream: &mut TcpStream, code: u16, body: serde_json::Value) {
    let body = serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_owned());
    let reason = match code {
        200 => "OK",
        404 => "Not Found",
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
