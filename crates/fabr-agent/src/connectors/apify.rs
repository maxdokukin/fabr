//! One Apify Actor run. A missing token disables only this tool.

use crate::tools_contract::{self, APIFY_TOKEN};
use fabr_core::FabrError;
use fabr_core::redact;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

pub fn run(space: &Path, params: &Value) -> Result<String, FabrError> {
    let actor = params.get("actor").and_then(|v| v.as_str()).unwrap_or("").trim();
    if !actor_ok(actor) {
        return Err(FabrError::BadRequest("apify actor id is not canonical".into()));
    }
    let input: Value = match params.get("input").and_then(|v| v.as_str()) {
        Some(text) => serde_json::from_str(text).map_err(|_| FabrError::BadRequest("apify input must be json".into()))?,
        None => json!({}),
    };
    let token =
        tools_contract::secret_value(space, APIFY_TOKEN).ok_or_else(|| FabrError::BadRequest("tool apify_lookup is unavailable".into()))?;
    let url = format!("https://api.apify.com/v2/acts/{actor}/run-sync-get-dataset-items");
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).build();
    let sent = agent.post(&url).set("content-type", "application/json").set("authorization", &format!("Bearer {token}")).send_json(input);
    let (status, text) = match sent {
        Ok(resp) => (resp.status(), resp.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(status, resp)) => (status, resp.into_string().unwrap_or_default()),
        Err(err) => {
            let text = scrub(&err.to_string(), &token);
            if text.to_lowercase().contains("timeout") {
                return Err(FabrError::Timeout("timeout".into()));
            }
            return Err(FabrError::Remote("unavailable".into()));
        }
    };
    let text = scrub(&text, &token);
    if status == 401 || status == 403 {
        return Err(FabrError::BadRequest("auth failed".into()));
    }
    if status == 408 {
        return Err(FabrError::Timeout("timeout".into()));
    }
    if !(200..300).contains(&status) {
        return Err(FabrError::Remote(format!("http {status}")));
    }
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let count = value.as_array().map(|items| items.len()).unwrap_or(0);
    Ok(format!("apify_lookup actor={actor} items={count}"))
}

fn actor_ok(actor: &str) -> bool {
    !actor.is_empty()
        && actor.len() <= 120
        && actor.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '~' | '_' | '-' | '.' | '/'))
        && !actor.contains("..")
}

fn scrub(text: &str, token: &str) -> String {
    let out = if token.len() >= 6 { text.replace(token, "[redacted]") } else { text.to_string() };
    redact::redact(&out)
}
