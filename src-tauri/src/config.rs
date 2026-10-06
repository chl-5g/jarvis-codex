use serde_json::Value;
use std::sync::OnceLock;

const CONFIG_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/prompts.json"
));
const PERMISSIONS_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/permissions.json"
));
const PATHS_JSON: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../config/paths.json"));
const CONNECTORS_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/connectors.json"
));
const TOOLS_JSON: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../config/tools.json"));
static CONFIG: OnceLock<Value> = OnceLock::new();
static PERMISSIONS: OnceLock<Value> = OnceLock::new();
static PATHS: OnceLock<Value> = OnceLock::new();
static PROJECT_ROOT: OnceLock<String> = OnceLock::new();
static CONNECTORS: OnceLock<Value> = OnceLock::new();
static TOOLS: OnceLock<Value> = OnceLock::new();

fn value() -> &'static Value {
    CONFIG.get_or_init(|| {
        serde_json::from_str(CONFIG_JSON).unwrap_or_else(|_| Value::Object(Default::default()))
    })
}

fn permissions() -> &'static Value {
    PERMISSIONS.get_or_init(|| {
        serde_json::from_str(PERMISSIONS_JSON).unwrap_or_else(|_| Value::Object(Default::default()))
    })
}

fn paths() -> &'static Value {
    PATHS.get_or_init(|| {
        serde_json::from_str(PATHS_JSON).unwrap_or_else(|_| Value::Object(Default::default()))
    })
}

pub fn project_root() -> &'static str {
    PROJECT_ROOT
        .get_or_init(|| {
            std::env::var("PROJECTPATH")
                .or_else(|_| std::env::var("JARVIS_PROJECT_ROOT"))
                .unwrap_or_else(|_| {
                    paths()
                        .get("projectRoot")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned()
                })
        })
        .as_str()
}

pub fn log_directory() -> String {
    let root = project_root();
    let directory = paths()
        .get("logDirectory")
        .and_then(Value::as_str)
        .unwrap_or("logs");
    format!("{root}/{directory}")
}

pub fn connectors() -> Value {
    CONNECTORS
        .get_or_init(|| {
            serde_json::from_str(CONNECTORS_JSON)
                .unwrap_or_else(|_| Value::Object(Default::default()))
        })
        .clone()
}

fn tools() -> &'static Value {
    TOOLS.get_or_init(|| {
        serde_json::from_str(TOOLS_JSON).unwrap_or_else(|_| Value::Object(Default::default()))
    })
}

pub fn prompt(key: &str) -> &'static str {
    value().get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn tool_description(name: &str) -> &'static str {
    tools()
        .get("descriptions")
        .and_then(|items| items.get(name))
        .and_then(Value::as_str)
        .unwrap_or("")
}

pub fn permission_settings_url(capability: &str) -> &'static str {
    permissions()
        .get("settingsUrls")
        .and_then(|items| items.get(capability))
        .and_then(Value::as_str)
        .unwrap_or("")
}

pub fn permission_capabilities() -> Vec<String> {
    permissions()
        .get("settingsUrls")
        .and_then(Value::as_object)
        .map(|items| items.keys().cloned().collect())
        .unwrap_or_default()
}
