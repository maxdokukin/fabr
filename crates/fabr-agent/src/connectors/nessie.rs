//! Nessie sandbox reads. Results are labeled sandbox and are not internship memory.

use crate::tools_contract::{self, NESSIE_KEY};
use fabr_core::FabrError;
use fabr_core::redact;
use std::path::Path;
use std::time::Duration;

pub fn sandbox_accounts(space: &Path) -> Result<String, FabrError> {
    let key = tools_contract::secret_value(space, NESSIE_KEY)
        .ok_or_else(|| FabrError::BadRequest("tool nessie_sandbox is unavailable".into()))?;
    let url = format!("https://api.nessieisreal.com/accounts?key={key}");
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(25)).build();
    let sent = agent.get(&url).call();
    let (status, text) = match sent {
        Ok(resp) => (resp.status(), resp.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(status, resp)) => (status, resp.into_string().unwrap_or_default()),
        Err(err) => {
            let text = scrub(&err.to_string(), &key);
            if text.to_lowercase().contains("timeout") {
                return Err(FabrError::Timeout("timeout".into()));
            }
            return Err(FabrError::Remote("unavailable".into()));
        }
    };
    let text = scrub(&text, &key);
    if status == 401 || status == 403 {
        return Err(FabrError::BadRequest("nessie sandbox rejected the key".into()));
    }
    if status == 408 {
        return Err(FabrError::Timeout("timeout".into()));
    }
    if !(200..300).contains(&status) {
        return Err(FabrError::Remote(format!("http {status}")));
    }
    let count = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v.as_array().map(|items| items.len())).unwrap_or(0);
    Ok(format!("nessie sandbox accounts={count}"))
}

fn scrub(text: &str, key: &str) -> String {
    let mut out = if key.len() >= 6 { text.replace(key, "[redacted]") } else { text.to_string() };
    out = redact::redact(&out);
    out
}
