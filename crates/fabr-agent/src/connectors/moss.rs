//! Moss index and retrieve. The local learned-preferences file stays authoritative.
//! Upload is a separate tool and is not called from here.

use crate::tools_contract::{self, MOSS_PROJECT_ID, MOSS_PROJECT_KEY};
use fabr_core::FabrError;
use fabr_core::redact;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

const DEFAULT_URL: &str = "https://service.usemoss.dev/v1/manage";

pub fn list_indexes(space: &Path) -> Result<String, FabrError> {
    let (id, key) = creds(space)?;
    let body = json!({"action": "listIndexes", "projectId": id});
    let value = post(&key, &body)?;
    let names = names_of(&value);
    Ok(format!("moss_list indexes={} names={}", names.len(), names.join(",")))
}

pub fn retrieve(space: &Path, index: &str) -> Result<String, FabrError> {
    let index = index.trim();
    if index.is_empty() {
        return Err(FabrError::BadRequest("missing param index".into()));
    }
    let (id, key) = creds(space)?;
    let body = json!({"action": "getDocs", "projectId": id, "indexName": index});
    let value = post(&key, &body)?;
    let docs = value.get("docs").or_else(|| value.get("documents")).or_else(|| value.get("data")).cloned().unwrap_or(value);
    let count = docs.as_array().map(|items| items.len()).unwrap_or(0);
    Ok(format!("moss_retrieve index={index} docs={count} source=moss"))
}

fn creds(space: &Path) -> Result<(String, String), FabrError> {
    let id = tools_contract::secret_value(space, MOSS_PROJECT_ID)
        .ok_or_else(|| FabrError::BadRequest("tool moss_list is unavailable".into()))?;
    let key = tools_contract::secret_value(space, MOSS_PROJECT_KEY)
        .ok_or_else(|| FabrError::BadRequest("tool moss_list is unavailable".into()))?;
    Ok((id, key))
}

fn post(key: &str, body: &Value) -> Result<Value, FabrError> {
    let url = std::env::var("FABR_MOSS_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(25)).build();
    let sent =
        agent.post(&url).set("content-type", "application/json").set("x-service-version", "v1").set("x-project-key", key).send_json(body);
    let (status, text) = match sent {
        Ok(resp) => (resp.status(), resp.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(status, resp)) => (status, resp.into_string().unwrap_or_default()),
        Err(err) => {
            let text = scrub(&err.to_string(), key);
            if text.to_lowercase().contains("timeout") {
                return Err(FabrError::Timeout("timeout".into()));
            }
            return Err(FabrError::Remote("unavailable".into()));
        }
    };
    let text = scrub(&text, key);
    if status == 401 || status == 403 {
        return Err(FabrError::BadRequest("auth failed".into()));
    }
    if status == 408 {
        return Err(FabrError::Timeout("timeout".into()));
    }
    if !(200..300).contains(&status) {
        return Err(FabrError::Remote(format!("http {status}")));
    }
    serde_json::from_str(&text).map_err(|_| FabrError::Remote("unavailable".into()))
}

fn names_of(value: &Value) -> Vec<String> {
    let list = value
        .as_array()
        .or_else(|| value.get("indexes").and_then(|v| v.as_array()))
        .or_else(|| value.get("data").and_then(|v| v.as_array()));
    let Some(list) = list else { return Vec::new() };
    list.iter()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("name").and_then(|v| v.as_str()))
                .or_else(|| item.get("indexName").and_then(|v| v.as_str()))
                .map(str::to_string)
        })
        .take(20)
        .collect()
}

fn scrub(text: &str, key: &str) -> String {
    let mut out = if key.len() >= 6 { text.replace(key, "[redacted]") } else { text.to_string() };
    out = redact::redact(&out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_moss_credentials_do_not_call_out() {
        let tmp = tempfile::tempdir().unwrap();
        let err = list_indexes(tmp.path()).unwrap_err();
        assert!(err.message().contains("unavailable"));
    }
}
