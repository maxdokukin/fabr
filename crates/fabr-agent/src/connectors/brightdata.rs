//! Bright Data public-web retrieval over the official remote MCP server.
//!
//! The token comes from `brightdata_api_token` in the fabric-space secrets
//! file, or from `BRIGHTDATA_API_TOKEN` / `API_TOKEN`. It is not logged.
//! Login pages, paywalled URLs, and queries that carry personal data are refused
//! before a request is sent. A missing token leaves the tool unavailable.

use crate::now_utc;
use crate::tools_contract::{self, BRIGHTDATA_TOKEN};
use fabr_core::FabrError;
use fabr_core::redact;
use serde_json::{Value, json};
use std::io::BufRead;
use std::path::Path;
use std::time::Duration;

pub const MCP_HOST: &str = "https://mcp.brightdata.com/mcp";
const SEARCH_TOOL: &str = "search_engine";
const EXTRACT_TOOL: &str = "scrape_as_markdown";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub tool: String,
    pub query: String,
    pub retrieved_at: String,
    pub hits: Vec<Hit>,
    pub error: Option<String>,
}

impl Report {
    pub fn markdown(&self) -> String {
        let mut out = format!("source: bright_data\nretrieved_at: {}\ntool: {}\nquery: {}\n", self.retrieved_at, self.tool, self.query);
        if let Some(error) = &self.error {
            out.push_str(&format!("error: {error}\n"));
        }
        if self.hits.is_empty() {
            out.push_str("hits: 0\n");
        }
        for hit in &self.hits {
            out.push_str(&format!("\n- title: {}\n  url: {}\n  snippet: {}\n", hit.title, hit.url, hit.snippet));
        }
        out
    }

    pub fn log_line(&self) -> String {
        let error = self.error.clone().unwrap_or_else(|| "none".into());
        format!("{} query={:?} hits={} error={error} retrieved_at={}", self.tool, self.query, self.hits.len(), self.retrieved_at)
    }
}

pub fn token(fabric_space: &Path) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(fabric_space.join("secrets.toml"))
        && let Ok(doc) = text.parse::<toml_edit::DocumentMut>()
        && let Some(value) = tools_contract::secret_in(&doc, BRIGHTDATA_TOKEN)
    {
        redact::register_secret(&value, "[redacted]");
        return Some(value);
    }
    for name in ["BRIGHTDATA_API_TOKEN", "API_TOKEN"] {
        if let Ok(value) = std::env::var(name) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                redact::register_secret(&value, "[redacted]");
                return Some(value);
            }
        }
    }
    None
}

pub fn retrieve(fabric_space: &Path, tool: &str, params: &Value) -> Result<Report, FabrError> {
    let Some(token) = token(fabric_space) else {
        return Err(FabrError::BadRequest(format!("tool {tool} is unavailable")));
    };
    let query = match tool {
        "bright_data_search" => params.get("query").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
        "bright_data_extract" => params.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string(),
        other => return Err(FabrError::BadRequest(format!("unknown tool {other}"))),
    };
    if query.is_empty() {
        return Err(FabrError::BadRequest("missing public web query".into()));
    }
    if let Err(reason) = policy(tool, &query) {
        return Ok(Report { tool: tool.into(), query, retrieved_at: now_utc(), hits: Vec::new(), error: Some(reason) });
    }
    let mcp_tool = if tool == "bright_data_extract" { EXTRACT_TOOL } else { SEARCH_TOOL };
    let arguments = if tool == "bright_data_extract" { json!({ "url": query }) } else { json!({ "query": query, "engine": "google" }) };
    match call(&token, mcp_tool, &arguments) {
        Ok(text) => {
            let body = untrusted_body(&text);
            let mut hits = hits_from_text(body);
            if tool == "bright_data_extract" && public_url(&query).is_ok() {
                let snippet: String = body.chars().take(240).collect();
                let mut sourced = Vec::new();
                push_hit(&mut sourced, query.clone(), query.clone(), &snippet);
                for hit in hits {
                    if sourced.len() >= 8 {
                        break;
                    }
                    if chrome_url(&hit.url) {
                        continue;
                    }
                    push_hit(&mut sourced, hit.title, hit.url, &hit.snippet);
                }
                hits = sourced;
            }
            hits.truncate(8);
            Ok(finish(tool, query, hits, None))
        }
        Err(kind) => Ok(finish(tool, query, Vec::new(), Some(kind))),
    }
}

fn finish(tool: &str, query: String, hits: Vec<Hit>, error: Option<String>) -> Report {
    let error = match error {
        Some(error) => Some(error),
        None if hits.is_empty() => Some("empty".into()),
        None => None,
    };
    Report { tool: tool.into(), query, retrieved_at: now_utc(), hits, error }
}

pub fn policy(tool: &str, query: &str) -> Result<(), String> {
    let lower = query.to_lowercase();
    if lower.contains('@') || looks_like_ssn(&lower) {
        return Err("personal data refused".into());
    }
    if tool == "bright_data_extract" {
        public_url(query)?;
    }
    Ok(())
}

pub fn public_url(url: &str) -> Result<(), String> {
    let lower = url.to_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err("only public http urls are allowed".into());
    }
    if url.contains('@') {
        return Err("urls with credentials are refused".into());
    }
    let path = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url).to_lowercase();
    if path.starts_with("localhost")
        || path.starts_with("127.")
        || path.starts_with("0.0.0.0")
        || path.starts_with("10.")
        || path.starts_with("192.168.")
        || path.starts_with("[::1]")
    {
        return Err("only public http urls are allowed".into());
    }
    for blocked in ["/login", "/signin", "/sign-in", "/signup", "/checkout", "/account", "/billing", "/paywall", "/subscribe"] {
        if path.contains(blocked) {
            return Err("login or paywalled urls are refused".into());
        }
    }
    Ok(())
}

fn looks_like_ssn(text: &str) -> bool {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    text.contains("ssn") || (digits.len() == 9 && text.contains('-'))
}

fn untrusted_body(text: &str) -> &str {
    let Some(begin) = text.find("_BEGIN=====") else {
        return text;
    };
    let rest = text[begin + "_BEGIN=====".len()..].trim();
    if let Some(end) = rest.find("=====UNTRUSTED_") {
        return rest[..end].trim();
    }
    rest
}

pub fn hits_from_text(text: &str) -> Vec<Hit> {
    let text = untrusted_body(text);
    if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
        let hits = hits_from_json(&value);
        if !hits.is_empty() {
            return hits;
        }
    }
    let mut hits = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let Some(mid) = rest[start..].find("](") else { break };
        let Some(end) = rest[start + mid + 2..].find(')') else { break };
        let title = rest[start + 1..start + mid].trim().to_string();
        let url = clean_url(rest[start + mid + 2..start + mid + 2 + end].trim());
        if public_url(&url).is_ok() {
            push_hit(&mut hits, title, url, "");
        }
        rest = &rest[start + mid + 2 + end + 1..];
    }
    for token in text.split_whitespace() {
        let url = clean_url(token.trim_matches(|c: char| matches!(c, ')' | '(' | ',' | '"' | '\'' | '<' | '>')));
        if public_url(&url).is_ok() {
            push_hit(&mut hits, String::new(), url, "");
        }
    }
    hits
}

fn hits_from_json(value: &Value) -> Vec<Hit> {
    let mut hits = Vec::new();
    collect_json(&mut hits, value);
    hits
}

fn collect_json(hits: &mut Vec<Hit>, value: &Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_json(hits, item);
            }
        }
        Value::Object(map) => {
            let url = ["url", "link", "href"].iter().find_map(|key| map.get(*key).and_then(|v| v.as_str()));
            if let Some(url) = url
                && public_url(url).is_ok()
            {
                let title = map.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let snippet: String = ["snippet", "description", "text"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .chars()
                    .take(400)
                    .collect();
                push_hit(hits, title, url.to_string(), &snippet);
            }
            for child in map.values() {
                if !child.is_object() && !child.is_array() {
                    continue;
                }
                collect_json(hits, child);
            }
        }
        _ => {}
    }
}

fn push_hit(hits: &mut Vec<Hit>, title: String, url: String, snippet: &str) {
    if hits.iter().any(|hit| hit.url == url) {
        return;
    }
    let title = if title.is_empty() { url.clone() } else { title };
    let snippet = if paywalled(snippet) { String::new() } else { snippet.chars().take(400).collect() };
    hits.push(Hit { title, url, snippet });
}

fn clean_url(url: &str) -> String {
    url.split_whitespace().next().unwrap_or(url).trim_matches(|c: char| matches!(c, '"' | '\'' | '<' | '>')).to_string()
}

fn chrome_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    let path = lower.split_once("://").map(|(_, rest)| rest).unwrap_or(&lower);
    let path = path.split(['?', '#']).next().unwrap_or(path).trim_end_matches('/');
    [
        "github.com/features",
        "github.com/mcp",
        "github.com/security",
        "github.com/why-github",
        "github.com/enterprise",
        "github.com/marketplace",
        "github.com/team",
        "github.com/solutions",
        "github.com/resources",
        "github.com/customer-stories",
        "github.blog",
        "docs.github.com",
    ]
    .iter()
    .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

fn paywalled(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["sign in to continue", "subscribe to read", "this content is for subscribers", "paywall"].iter().any(|needle| lower.contains(needle))
}

fn call(token: &str, tool: &str, arguments: &Value) -> Result<String, String> {
    let base = std::env::var("FABR_BRIGHTDATA_MCP_URL").unwrap_or_else(|_| MCP_HOST.into());
    let url = mcp_url(&base, token);
    let session = initialize(&url, token).ok().flatten();
    let body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": tool, "arguments": arguments}
    });
    let (status, raw, _) = post(&url, &body, token, session.as_deref())?;
    let text = scrub(&raw, token);
    if status == 429 {
        return Err("rate_limited".into());
    }
    if status == 408 {
        return Err("timeout".into());
    }
    if !(200..300).contains(&status) {
        return Err("unavailable".into());
    }
    let payload = json_payload(&raw).unwrap_or(Value::Null);
    if payload.get("error").is_some() {
        return Err(classify_message(&text));
    }
    let result = payload.get("result").cloned().unwrap_or(payload);
    if result.get("isError").and_then(|v| v.as_bool()) == Some(true) {
        return Err(classify_message(&text));
    }
    let collected = scrub(&content_text(&result), token);
    if paywalled(&collected) {
        return Err("paywalled".into());
    }
    if collected.trim().is_empty() { Err("empty".into()) } else { Ok(collected) }
}

fn initialize(url: &str, token: &str) -> Result<Option<String>, String> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "fabr", "version": "0.2.6"}}
    });
    let (status, text, session) = post(url, &body, token, None)?;
    if !(200..300).contains(&status) {
        return Err("unavailable".into());
    }
    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(5)).build();
    let mut req = agent.post(url).set("content-type", "application/json").set("accept", "application/json, text/event-stream");
    if let Some(session_id) = session.as_deref() {
        req = req.set("mcp-session-id", session_id);
    }
    let _ = req.send_json(note);
    let _ = text;
    Ok(session)
}

fn content_text(result: &Value) -> String {
    let mut out = String::new();
    if let Some(blocks) = result.get("content").and_then(|v| v.as_array()) {
        for block in blocks {
            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    }
    if out.is_empty()
        && let Some(text) = result.as_str()
    {
        out = text.to_string();
    }
    out
}

fn json_payload(text: &str) -> Option<Value> {
    if let Some(value) = parse_one_json(text.trim())
        && (value.get("result").is_some() || value.get("error").is_some() || value.get("jsonrpc").is_some())
    {
        return Some(value);
    }
    let mut last = None;
    for line in text.lines() {
        let Some(data) = line.trim().strip_prefix("data:") else { continue };
        if let Some(value) = parse_one_json(data.trim()) {
            last = Some(value);
        }
    }
    last
}

fn parse_one_json(input: &str) -> Option<Value> {
    let mut values = serde_json::Deserializer::from_str(input).into_iter::<Value>();
    values.next()?.ok()
}

fn classify_message(text: &str) -> String {
    let lower = text.to_lowercase();
    if lower.contains("rate limit") || lower.contains("too many requests") {
        "rate_limited".into()
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "timeout".into()
    } else if paywalled(&lower) {
        "paywalled".into()
    } else {
        "unavailable".into()
    }
}

fn mcp_url(base: &str, token: &str) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}token={}", encode(token))
}

fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn post(url: &str, body: &Value, token: &str, session: Option<&str>) -> Result<(u16, String, Option<String>), String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(70)).build();
    let mut req = agent.post(url).set("content-type", "application/json").set("accept", "application/json, text/event-stream");
    if let Some(session) = session {
        req = req.set("mcp-session-id", session);
    }
    match req.send_json(body) {
        Ok(resp) => read_reply(resp.status(), resp.header("mcp-session-id").map(str::to_string), resp.into_reader()),
        Err(ureq::Error::Status(status, resp)) => read_reply(status, resp.header("mcp-session-id").map(str::to_string), resp.into_reader()),
        Err(err) => {
            let text = scrub(&err.to_string(), token);
            if text.to_lowercase().contains("timed out") || text.to_lowercase().contains("timeout") {
                Err("timeout".into())
            } else {
                Err("unavailable".into())
            }
        }
    }
}

fn read_reply(status: u16, session: Option<String>, reader: impl std::io::Read) -> Result<(u16, String, Option<String>), String> {
    let mut reader = std::io::BufReader::new(reader);
    let mut collected = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).map_err(|_| "unavailable".to_string())?;
        if read == 0 {
            break;
        }
        collected.push_str(&line);
        if let Some(payload) = json_payload(&collected)
            && (payload.get("result").is_some() || payload.get("error").is_some())
        {
            break;
        }
        if collected.len() > 2_000_000 {
            break;
        }
    }
    Ok((status, collected, session))
}

fn scrub(text: &str, token: &str) -> String {
    let mut out = if token.len() >= 6 { text.replace(token, "[redacted]") } else { text.to_string() };
    out = out.replace(&encode(token), "[redacted]");
    redact::redact(&out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parser_keeps_public_urls_and_drops_login_links() {
        let text =
            "[ureq docs](https://docs.rs/ureq)\n[issue](https://github.com/algesten/ureq/issues/1)\n[login](https://example.com/login)";
        let hits = hits_from_text(text);
        let urls: Vec<_> = hits.iter().map(|hit| hit.url.as_str()).collect();
        assert_eq!(urls, vec!["https://docs.rs/ureq", "https://github.com/algesten/ureq/issues/1"]);
        let titled = hits_from_text("[ureq](https://crates.io/crates/ureq \"See ureq\")");
        assert_eq!(titled[0].url, "https://crates.io/crates/ureq");
    }

    #[test]
    fn policy_refuses_personal_data_and_paywalled_urls() {
        assert_eq!(policy("bright_data_search", "mail user@example.com").unwrap_err(), "personal data refused");
        assert_eq!(policy("bright_data_extract", "https://news.example/account/1").unwrap_err(), "login or paywalled urls are refused");
        assert!(policy("bright_data_search", "Rust ureq documentation and GitHub issues").is_ok());
    }

    #[test]
    fn empty_and_rate_limit_are_explicit_errors() {
        let empty = finish("bright_data_search", "ureq".into(), Vec::new(), None);
        assert_eq!(empty.error.as_deref(), Some("empty"));
        assert!(empty.hits.is_empty());
        assert!(!empty.markdown().contains("https://"));
        let limited = finish("bright_data_search", "ureq".into(), Vec::new(), Some("rate_limited".into()));
        assert!(limited.log_line().contains("error=rate_limited"));
        assert_eq!(classify_message("HTTP 429 too many requests"), "rate_limited");
        assert_eq!(classify_message("request timed out"), "timeout");
    }

    #[test]
    fn wrapped_serp_keeps_organic_links_and_empty_stays_empty() {
        let wrapped = "SECURITY NOTICE\n=====UNTRUSTED_abc_BEGIN=====\n{\"organic\":[{\"title\":\"ureq\",\"link\":\"https://docs.rs/ureq\",\"description\":\"http client\"}]}\n=====UNTRUSTED_abc_END=====\n";
        let hits = hits_from_text(wrapped);
        assert_eq!(hits[0].title, "ureq");
        assert_eq!(hits[0].url, "https://docs.rs/ureq");
        let empty = "=====UNTRUSTED_abc_BEGIN=====\n{\"organic\":[]}\n=====UNTRUSTED_abc_END=====";
        assert!(hits_from_text(empty).is_empty());
    }

    #[test]
    fn json_search_results_keep_source_urls() {
        let value = json!({"organic": [{"title": "docs", "link": "https://docs.rs/ureq", "description": "http client"}]});
        let hits = hits_from_json(&value);
        assert_eq!(hits[0].url, "https://docs.rs/ureq");
        assert!(hits[0].snippet.contains("http client"));
    }

    #[test]
    fn missing_token_does_not_invent_a_result() {
        let tmp = tempfile::tempdir().unwrap();
        let err = retrieve(tmp.path(), "bright_data_search", &json!({"query": "ureq docs"})).unwrap_err();
        assert!(err.message().contains("unavailable"));
    }
}
