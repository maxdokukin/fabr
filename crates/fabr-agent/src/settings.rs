use fabr_core::{FabrError, Secret};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct AgentSettings {
    pub default_folder: String,
    pub model: String,
    pub api_key: Secret,
    pub provider: String,
    pub local_base_url: String,
    pub heartbeat_interval_s: u64,
    pub check_timeout_s: u64,
    pub execute_max_turns: u32,
    pub evaluate_max_reads: usize,
    pub completed_steps: Vec<String>,
}

pub const DEFAULT_PROVIDER: &str = "local";
pub const DEFAULT_LOCAL_BASE_URL: &str = "http://localhost:11434/v1";

pub fn load_agent_settings(dir: &Path) -> Result<AgentSettings, FabrError> {
    let settings_path = dir.join("settings.toml");
    let secrets_path = dir.join("secrets.toml");
    let settings_text = std::fs::read_to_string(&settings_path).map_err(|e| FabrError::Io(format!("{}: {e}", settings_path.display())))?;
    let secrets_text = std::fs::read_to_string(&secrets_path).map_err(|e| FabrError::Io(format!("{}: {e}", secrets_path.display())))?;
    let doc = settings_text.parse::<toml_edit::DocumentMut>().map_err(|e| FabrError::BadRequest(format!("settings.toml: {e}")))?;
    let secrets = secrets_text.parse::<toml_edit::DocumentMut>().map_err(|e| FabrError::BadRequest(format!("secrets.toml: {e}")))?;
    let model = doc.get("model").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let provider = {
        let p = doc.get("provider").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if p.is_empty() { DEFAULT_PROVIDER.to_string() } else { p }
    };
    let local_base_url = {
        let u = doc.get("local_base_url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if u.is_empty() { DEFAULT_LOCAL_BASE_URL.to_string() } else { u }
    };
    let default_folder = doc.get("default_folder").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let heartbeat_interval_s =
        doc.get("heartbeat_interval_s").and_then(|v| v.as_integer()).filter(|n| *n > 0).map(|n| n as u64).unwrap_or(10);
    let completed_steps = doc
        .get("setup")
        .and_then(|s| s.get("completed_steps"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let key = secrets.get("api_key").or_else(|| secrets.get("anthropic_api_key")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let api_key = Secret::new(key);
    let settings = AgentSettings {
        default_folder,
        model,
        api_key,
        provider,
        local_base_url,
        heartbeat_interval_s,
        check_timeout_s: read_check_timeout(&doc),
        execute_max_turns: read_limit(&doc, "execute_max_turns", DEFAULT_EXECUTE_MAX_TURNS as u64) as u32,
        evaluate_max_reads: read_limit(&doc, "evaluate_max_reads", DEFAULT_EVALUATE_MAX_READS as u64) as usize,
        completed_steps,
    };
    require_ready(&settings)?;
    Ok(settings)
}

pub const DEFAULT_EXECUTE_MAX_TURNS: u32 = 12;
pub const DEFAULT_EVALUATE_MAX_READS: usize = 6;

pub fn require_ready(s: &AgentSettings) -> Result<(), FabrError> {
    let steps: &[&str] = &["folder", "model"];
    for step in steps {
        if !s.completed_steps.iter().any(|have| have == step) {
            return Err(FabrError::BadRequest(format!("setup is missing {step}")));
        }
    }
    if s.model.is_empty() {
        return Err(FabrError::BadRequest("model is required".into()));
    }
    Ok(())
}

fn read_check_timeout(doc: &toml_edit::DocumentMut) -> u64 {
    doc.get("check_timeout_s")
        .and_then(|v| v.as_integer())
        .filter(|n| *n > 0)
        .map(|n| (n as u64).min(3600))
        .unwrap_or(crate::verify::DEFAULT_CHECK_TIMEOUT_S)
}

pub fn check_timeout_s(space: &Path) -> u64 {
    std::fs::read_to_string(space.join("settings.toml"))
        .ok()
        .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
        .map(|d| read_check_timeout(&d))
        .unwrap_or(crate::verify::DEFAULT_CHECK_TIMEOUT_S)
}

fn read_limit(doc: &toml_edit::DocumentMut, key: &str, default: u64) -> u64 {
    doc.get(key).and_then(|v| v.as_integer()).filter(|n| *n > 0).map(|n| (n as u64).min(200)).unwrap_or(default)
}

pub fn limits(space: &Path) -> (u32, usize) {
    let doc = std::fs::read_to_string(space.join("settings.toml")).ok().and_then(|t| t.parse::<toml_edit::DocumentMut>().ok());
    match doc {
        Some(d) => (
            read_limit(&d, "execute_max_turns", DEFAULT_EXECUTE_MAX_TURNS as u64) as u32,
            read_limit(&d, "evaluate_max_reads", DEFAULT_EVALUATE_MAX_READS as u64) as usize,
        ),
        None => (DEFAULT_EXECUTE_MAX_TURNS, DEFAULT_EVALUATE_MAX_READS),
    }
}
