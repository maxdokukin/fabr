use fabr_core::FabrError;
use std::path::Path;

pub const DEFAULT_EXECUTE_TIMEOUT_S: u64 = crate::claude::DEFAULT_TIMEOUT_S;

#[derive(Debug, Clone)]
pub struct AgentSettings {
    pub default_folder: String,
    pub model: String,
    pub effort: String,
    pub claude_bin: String,
    pub heartbeat_interval_s: u64,
    pub check_timeout_s: u64,
    pub execute_timeout_s: u64,
    pub completed_steps: Vec<String>,
}

pub fn load_agent_settings(dir: &Path) -> Result<AgentSettings, FabrError> {
    let settings_path = dir.join("settings.toml");
    let settings_text = std::fs::read_to_string(&settings_path).map_err(|e| FabrError::Io(format!("{}: {e}", settings_path.display())))?;
    let doc = settings_text.parse::<toml_edit::DocumentMut>().map_err(|e| FabrError::BadRequest(format!("settings.toml: {e}")))?;
    let text = |k: &str| doc.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let heartbeat_interval_s =
        doc.get("heartbeat_interval_s").and_then(|v| v.as_integer()).filter(|n| *n > 0).map(|n| n as u64).unwrap_or(10);
    let completed_steps = doc
        .get("setup")
        .and_then(|s| s.get("completed_steps"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(AgentSettings {
        default_folder: text("default_folder"),
        model: text("model"),
        effort: text("effort"),
        claude_bin: text("claude_bin"),
        heartbeat_interval_s,
        check_timeout_s: read_check_timeout(&doc),
        execute_timeout_s: read_execute_timeout(&doc),
        completed_steps,
    })
}

fn read_check_timeout(doc: &toml_edit::DocumentMut) -> u64 {
    doc.get("check_timeout_s")
        .and_then(|v| v.as_integer())
        .filter(|n| *n > 0)
        .map(|n| (n as u64).min(3600))
        .unwrap_or(crate::verify::DEFAULT_CHECK_TIMEOUT_S)
}

fn read_execute_timeout(doc: &toml_edit::DocumentMut) -> u64 {
    doc.get("execute_timeout_s")
        .and_then(|v| v.as_integer())
        .filter(|n| *n > 0)
        .map(|n| (n as u64).min(86_400))
        .unwrap_or(DEFAULT_EXECUTE_TIMEOUT_S)
}

fn doc_of(space: &Path) -> Option<toml_edit::DocumentMut> {
    std::fs::read_to_string(space.join("settings.toml")).ok().and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
}

pub fn check_timeout_s(space: &Path) -> u64 {
    doc_of(space).map(|d| read_check_timeout(&d)).unwrap_or(crate::verify::DEFAULT_CHECK_TIMEOUT_S)
}

pub fn execute_timeout_s(space: &Path) -> u64 {
    doc_of(space).map(|d| read_execute_timeout(&d)).unwrap_or(DEFAULT_EXECUTE_TIMEOUT_S)
}

pub fn max_budget_usd(space: &Path) -> f64 {
    doc_of(space)
        .and_then(|d| d.get("max_budget_usd").and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64))))
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(0.0)
}
