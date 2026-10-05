//! Shared event envelope for the local Jarvis event stream.
//!
//! Producers keep their domain fields (`kind`, `phase`, `toolName`, and so
//! on), while this module adds stable metadata consumed by the UI and the
//! localhost bridge.  The envelope is flat for backwards compatibility with
//! the existing `jarvis-event` listener.

use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

static NEXT_EVENT_ID: AtomicU64 = AtomicU64::new(1);

const SCHEMA_VERSION: u8 = 1;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventEnvelope {
    schema_version: u8,
    event_id: String,
    timestamp: u64,
    source: String,
    #[serde(flatten)]
    fields: Map<String, Value>,
}

/// Emit a domain event to the renderer and bounded localhost bridge.
pub fn emit(app: &AppHandle, source: &str, payload: Value) {
    let mut fields = payload.as_object().cloned().unwrap_or_default();
    fields
        .entry("kind".to_owned())
        .or_insert_with(|| Value::String(source.to_owned()));
    let envelope = EventEnvelope {
        schema_version: SCHEMA_VERSION,
        event_id: next_event_id(),
        timestamp: now(),
        source: source.to_owned(),
        fields,
    };
    let value = serde_json::to_value(&envelope).unwrap_or(Value::Null);
    let _ = app.emit("jarvis-event", &value);
    crate::bridge::publish(source.to_owned(), value);
}

fn next_event_id() -> String {
    let now = now();
    format!(
        "jarvis-{now}-{}",
        NEXT_EVENT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_keeps_domain_fields_flat_and_adds_metadata() {
        let mut fields = Map::new();
        fields.insert("kind".to_owned(), Value::String("tool".to_owned()));
        fields.insert("phase".to_owned(), Value::String("started".to_owned()));
        let envelope = EventEnvelope {
            schema_version: SCHEMA_VERSION,
            event_id: "jarvis-test".to_owned(),
            timestamp: 42,
            source: "tool".to_owned(),
            fields,
        };
        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["eventId"], "jarvis-test");
        assert_eq!(value["source"], "tool");
        assert_eq!(value["kind"], "tool");
        assert_eq!(value["phase"], "started");
        assert!(value.get("fields").is_none());
    }
}
