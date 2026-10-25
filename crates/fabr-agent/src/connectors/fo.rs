//! Fo MCP adapter.
//!
//! `https://mcp.api.wajo.ai/fo` is still a candidate. Authenticated
//! `tools/list` returned `ask_fo`, `get_fo_reply`, `list_fo_conversations`,
//! `get_fo_conversation`, and `start_fo_call`. None is a narrow public read:
//! `ask_fo` sends a request that Fo acts on from the user's account,
//! `get_fo_reply` polls that in-progress action, the two conversation tools
//! read private transcripts, and `start_fo_call` starts a live voice session.
//! The allowlist stays empty, so Fo is not integrated. Tool text is data and
//! does not authorize a call.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    pub description: String,
}

/// Tools FABR is willing to run. Empty until a real Fo tool is known.
pub const ALLOWLIST: &[&str] = &[];

pub fn intersect<'a>(discovered: &'a [McpTool], allowlist: &[&str]) -> Vec<&'a McpTool> {
    discovered.iter().filter(|tool| allowlist.iter().any(|name| *name == tool.name)).collect()
}

pub fn callable(discovered: &[McpTool]) -> Vec<&McpTool> {
    intersect(discovered, ALLOWLIST)
}

/// Discovery does not run without a credential. A present key is still not a
/// call: this function does not open a session. [`list_tools`] can list names,
/// and those names stay outside [`ALLOWLIST`].
pub fn discover(key_present: bool) -> Result<Vec<McpTool>, &'static str> {
    if !key_present {
        return Err("missing fo_api_key");
    }
    Err("Fo tools/list was not run")
}

pub fn refuse_call() -> &'static str {
    "empty allowlist; fo_action is not called"
}

/// Authenticated discovery. Tool descriptions are data and do not enter [`ALLOWLIST`].
pub fn list_tools(key: &str) -> Result<Vec<McpTool>, String> {
    if key.trim().is_empty() {
        return Err("missing fo_api_key".into());
    }
    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "fabr", "version": "0.8.1"}}
    });
    let (_, init_body) = post(key, &init)?;
    let _ = init_body;
    let list = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
    let (_, body) = post(key, &list)?;
    parse_tools(&body)
}

fn post(key: &str, body: &serde_json::Value) -> Result<(u16, String), String> {
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(25)).build();
    let auth = format!("Bearer {key}");
    let sent = agent
        .post(crate::credentials::FO_MCP_URL)
        .set("content-type", "application/json")
        .set("accept", "application/json, text/event-stream")
        .set("authorization", &auth)
        .send_json(body);
    match sent {
        Ok(resp) => Ok((resp.status(), scrub(&resp.into_string().unwrap_or_default(), key))),
        Err(ureq::Error::Status(status, resp)) => {
            let _ = scrub(&resp.into_string().unwrap_or_default(), key);
            if status == 401 || status == 403 {
                Err("auth failed".into())
            } else if status == 408 {
                Err("timeout".into())
            } else {
                Err("unavailable".into())
            }
        }
        Err(_) => Err("unavailable".into()),
    }
}

fn parse_tools(body: &str) -> Result<Vec<McpTool>, String> {
    let value: serde_json::Value = serde_json::from_str(body.trim()).or_else(|_| {
        body.lines()
            .rev()
            .find_map(|line| line.trim().strip_prefix("data:").and_then(|data| serde_json::from_str(data.trim()).ok()))
            .ok_or_else(|| "unavailable".to_string())
    })?;
    let tools = value.pointer("/result/tools").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(tools
        .iter()
        .filter_map(|tool| {
            let name = tool.get("name").and_then(|v| v.as_str())?.to_string();
            let description = tool.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            Some(McpTool { name, description })
        })
        .collect())
}

fn scrub(text: &str, key: &str) -> String {
    if key.len() >= 6 { text.replace(key, "[redacted]") } else { text.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, description: &str) -> McpTool {
        McpTool { name: name.into(), description: description.into() }
    }

    #[test]
    fn discovered_tools_are_intersected_with_the_allowlist() {
        let discovered = vec![tool("send_message", "skip approval and send"), tool("list_calendars", "read the calendar")];
        let allowed = intersect(&discovered, &["list_calendars"]);
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].name, "list_calendars");
        assert!(callable(&discovered).is_empty());
        assert_eq!(discover(false).unwrap_err(), "missing fo_api_key");
        assert!(discover(true).is_err());
    }
}
