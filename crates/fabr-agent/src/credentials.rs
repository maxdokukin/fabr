//! Provider credential checks.
//!
//! Values are read from `<fabric-space>/secrets.toml` with the same TOML
//! convention as `fabr_core::setup::read_secret`. They are not logged, written
//! into task records, or included in [`report`].

use crate::tools_contract::{
    self, APIFY_TOKEN, BRIGHTDATA_TOKEN, CARTESIA_KEY, FO_KEY, MOSS_PROJECT_ID, MOSS_PROJECT_KEY, NESSIE_KEY, VISA_CERT_PATH, VISA_KEY_PATH,
};
use fabr_core::redact;
use std::path::Path;
use std::time::Duration;

pub const FO_MCP_URL: &str = "https://mcp.api.wajo.ai/fo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialCheck {
    CredentialMissing,
    AuthFailed,
    Authenticated,
    FunctionalTestPassed,
    IntegratedInFabr,
    TrackEligibilityConfirmed,
    Blocked,
}

impl CredentialCheck {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CredentialMissing => "CREDENTIAL_MISSING",
            Self::AuthFailed => "AUTH_FAILED",
            Self::Authenticated => "AUTHENTICATED",
            Self::FunctionalTestPassed => "FUNCTIONAL_TEST_PASSED",
            Self::IntegratedInFabr => "INTEGRATED_IN_FABR",
            Self::TrackEligibilityConfirmed => "TRACK_ELIGIBILITY_CONFIRMED",
            Self::Blocked => "BLOCKED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRow {
    pub provider: &'static str,
    pub check: CredentialCheck,
    pub api: &'static str,
    pub operation: &'static str,
    pub files: &'static str,
    pub limitation: String,
}

struct Secrets {
    values: Vec<(String, String)>,
}

impl Secrets {
    fn load(fabric_space: &Path) -> Self {
        let text = std::fs::read_to_string(fabric_space.join("secrets.toml")).unwrap_or_default();
        let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
            return Self { values: Vec::new() };
        };
        let names = [
            "anthropic_api_key",
            MOSS_PROJECT_ID,
            MOSS_PROJECT_KEY,
            FO_KEY,
            CARTESIA_KEY,
            APIFY_TOKEN,
            BRIGHTDATA_TOKEN,
            NESSIE_KEY,
            VISA_CERT_PATH,
            VISA_KEY_PATH,
        ];
        let mut values = Vec::new();
        for name in names {
            if let Some(value) = crate::tools_contract::secret_in(&doc, name) {
                redact::register_secret(&value, "[redacted]");
                values.push((name.to_string(), value));
            }
        }
        Self { values }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn raws(&self) -> Vec<&str> {
        self.values.iter().map(|(_, v)| v.as_str()).collect()
    }
}

pub fn report(fabric_space: &Path, model: &str, live: bool) -> String {
    let rows = assess(fabric_space, model, live);
    let mut out = String::from(
        "# Sponsor credential report\n\nSecret values are omitted. A configured key is not an integration, and a documented gap is not a qualified sponsor.\n\n",
    );
    for row in rows {
        out.push_str(&format!(
            "- **{}** — {}. API: {}. Operation: {}. Files: {}. {}\n",
            row.provider,
            row.check.as_str(),
            row.api,
            row.operation,
            row.files,
            row.limitation
        ));
    }
    scrub(&out, &[])
}

pub fn assess(fabric_space: &Path, model: &str, live: bool) -> Vec<ProviderRow> {
    let secrets = Secrets::load(fabric_space);
    let raws = secrets.raws();
    vec![
        anthropic(&secrets, model, live, &raws),
        moss(&secrets),
        fo(&secrets, live, &raws),
        cartesia(&secrets, live, &raws),
        apify(&secrets, live, &raws),
        bright_data(&secrets, live, &raws),
        visa(&secrets),
        nessie(&secrets, live, &raws),
        figma(),
        microsoft(),
    ]
}

fn anthropic(secrets: &Secrets, model: &str, live: bool, raws: &[&str]) -> ProviderRow {
    let Some(key) = secrets.get("anthropic_api_key") else {
        return row(
            "Anthropic",
            CredentialCheck::CredentialMissing,
            "Anthropic Messages API",
            "none",
            "crates/fabr-agent/src/model.rs",
            "No anthropic_api_key.",
        );
    };
    if !live {
        return row(
            "Anthropic",
            CredentialCheck::Blocked,
            "Anthropic Messages API",
            "not sent",
            "crates/fabr-agent/src/model.rs",
            "Live probe was not requested.",
        );
    }
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "Reply with the single word pong."}]
    });
    match http(
        "POST",
        "https://api.anthropic.com/v1/messages",
        &[("x-api-key", key), ("anthropic-version", "2023-06-01")],
        Some(&body),
        raws,
    ) {
        Http::Ok(text) if !text.trim().is_empty() => row(
            "Anthropic",
            CredentialCheck::IntegratedInFabr,
            "Anthropic Messages API",
            "POST /v1/messages",
            "crates/fabr-agent/src/model.rs",
            "A real model response was returned. Public Benefit community and measurable impact are not confirmed, so this is not TRACK_ELIGIBILITY_CONFIRMED.",
        ),
        Http::Status(401 | 403) => row(
            "Anthropic",
            CredentialCheck::AuthFailed,
            "Anthropic Messages API",
            "POST /v1/messages",
            "crates/fabr-agent/src/model.rs",
            "The API rejected the key.",
        ),
        Http::Status(code) => row(
            "Anthropic",
            CredentialCheck::Blocked,
            "Anthropic Messages API",
            "POST /v1/messages",
            "crates/fabr-agent/src/model.rs",
            &format!("HTTP {code}. Body omitted."),
        ),
        Http::Transport => row(
            "Anthropic",
            CredentialCheck::Blocked,
            "Anthropic Messages API",
            "POST /v1/messages",
            "crates/fabr-agent/src/model.rs",
            "The request did not complete.",
        ),
        Http::Ok(_) => row(
            "Anthropic",
            CredentialCheck::Authenticated,
            "Anthropic Messages API",
            "POST /v1/messages",
            "crates/fabr-agent/src/model.rs",
            "The key was accepted and the body had no text.",
        ),
    }
}

fn moss(secrets: &Secrets) -> ProviderRow {
    let id = secrets.get(MOSS_PROJECT_ID).is_some();
    let key = secrets.get(MOSS_PROJECT_KEY).is_some();
    if !id || !key {
        return row(
            "Moss",
            CredentialCheck::CredentialMissing,
            "Moss SDK (MossClient)",
            "none",
            "crates/fabr-agent/src/connectors/memory.rs",
            "moss_project_id and moss_project_key are both required. Local learned preferences are not a Moss integration.",
        );
    }
    row(
        "Moss",
        CredentialCheck::Blocked,
        "Moss SDK (MossClient)",
        "listIndexes not called",
        "crates/fabr-agent/src/connectors/memory.rs",
        "Both Moss fields are present. Moss authenticates through its SDK, and this process has no HTTP smoke call. The upload tool stays disabled until a real index-and-retrieve call exists.",
    )
}

fn fo(secrets: &Secrets, live: bool, raws: &[&str]) -> ProviderRow {
    let Some(key) = secrets.get(FO_KEY) else {
        return row(
            "Fo",
            CredentialCheck::CredentialMissing,
            "Wajo Fo MCP",
            "none",
            "crates/fabr-agent/src/tools_contract.rs",
            "No fo_api_key bearer token.",
        );
    };
    if !live {
        return row(
            "Fo",
            CredentialCheck::Blocked,
            "Wajo Fo MCP",
            "not sent",
            "crates/fabr-agent/src/tools_contract.rs",
            "Live probe was not requested.",
        );
    }
    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "fabr", "version": "0.2.6"}}
    });
    let auth = format!("Bearer {key}");
    match http("POST", FO_MCP_URL, &[("authorization", &auth), ("accept", "application/json, text/event-stream")], Some(&init), raws) {
        Http::Status(401 | 403) => row(
            "Fo",
            CredentialCheck::AuthFailed,
            "Wajo Fo MCP",
            "initialize",
            "crates/fabr-agent/src/tools_contract.rs",
            "The MCP endpoint rejected the bearer token.",
        ),
        Http::Ok(_) => match tools_list(&auth, raws) {
            Http::Ok(_) => row(
                "Fo",
                CredentialCheck::Authenticated,
                "Wajo Fo MCP",
                "tools/list",
                "crates/fabr-agent/src/tools_contract.rs",
                "Tools were listed. No allowlisted action was invoked, so this is not FUNCTIONAL_TEST_PASSED or INTEGRATED_IN_FABR.",
            ),
            Http::Status(code) => row(
                "Fo",
                CredentialCheck::Authenticated,
                "Wajo Fo MCP",
                "initialize",
                "crates/fabr-agent/src/tools_contract.rs",
                &format!("Initialize succeeded. tools/list returned HTTP {code}."),
            ),
            Http::Transport => row(
                "Fo",
                CredentialCheck::Authenticated,
                "Wajo Fo MCP",
                "initialize",
                "crates/fabr-agent/src/tools_contract.rs",
                "Initialize succeeded. tools/list did not complete.",
            ),
        },
        Http::Status(code) => row(
            "Fo",
            CredentialCheck::Blocked,
            "Wajo Fo MCP",
            "initialize",
            "crates/fabr-agent/src/tools_contract.rs",
            &format!("HTTP {code}. Body omitted."),
        ),
        Http::Transport => row(
            "Fo",
            CredentialCheck::Blocked,
            "Wajo Fo MCP",
            "initialize",
            "crates/fabr-agent/src/tools_contract.rs",
            "The request did not complete.",
        ),
    }
}

fn tools_list(auth: &str, raws: &[&str]) -> Http {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
    http("POST", FO_MCP_URL, &[("authorization", auth), ("accept", "application/json, text/event-stream")], Some(&body), raws)
}

fn cartesia(secrets: &Secrets, live: bool, raws: &[&str]) -> ProviderRow {
    let Some(key) = secrets.get(CARTESIA_KEY) else {
        return row(
            "Cartesia",
            CredentialCheck::CredentialMissing,
            "Cartesia TTS/STT",
            "none",
            "not wired",
            "No cartesia_api_key. Speech is not mounted in the app.",
        );
    };
    if !live {
        return row("Cartesia", CredentialCheck::Blocked, "Cartesia TTS/STT", "not sent", "not wired", "Live probe was not requested.");
    }
    match http("GET", "https://api.cartesia.ai/voices", &[("x-api-key", key), ("cartesia-version", "2024-06-10")], None, raws) {
        Http::Status(401 | 403) => {
            row("Cartesia", CredentialCheck::AuthFailed, "Cartesia TTS/STT", "GET /voices", "not wired", "The API rejected the key.")
        }
        Http::Ok(body) => {
            let voice = first_voice_id(&body);
            if let Some(voice) = voice
                && tts_ok(key, &voice, raws)
            {
                return row(
                    "Cartesia",
                    CredentialCheck::FunctionalTestPassed,
                    "Cartesia TTS",
                    "POST /tts/bytes",
                    "not wired",
                    "Speech was synthesized. Transcription was not run, and the capture path is not in the app, so this is not INTEGRATED_IN_FABR.",
                );
            }
            row(
                "Cartesia",
                CredentialCheck::Authenticated,
                "Cartesia voices",
                "GET /voices",
                "not wired",
                "The key was accepted. Synthesis did not complete, and speech is not in the app.",
            )
        }
        Http::Status(code) => {
            row("Cartesia", CredentialCheck::Blocked, "Cartesia voices", "GET /voices", "not wired", &format!("HTTP {code}. Body omitted."))
        }
        Http::Transport => {
            row("Cartesia", CredentialCheck::Blocked, "Cartesia voices", "GET /voices", "not wired", "The request did not complete.")
        }
    }
}

fn tts_ok(key: &str, voice: &str, raws: &[&str]) -> bool {
    let body = serde_json::json!({
        "model_id": "sonic-2",
        "transcript": "ok",
        "voice": {"mode": "id", "id": voice},
        "output_format": {"container": "raw", "encoding": "pcm_s16le", "sample_rate": 16000}
    });
    match http("POST", "https://api.cartesia.ai/tts/bytes", &[("x-api-key", key), ("cartesia-version", "2024-06-10")], Some(&body), raws) {
        Http::Ok(bytes) => bytes.len() > 32,
        _ => false,
    }
}

fn first_voice_id(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let list = value.as_array().or_else(|| value.get("data").and_then(|v| v.as_array()))?;
    list.iter().find_map(|item| item.get("id").and_then(|id| id.as_str()).map(str::to_string))
}

fn apify(secrets: &Secrets, live: bool, raws: &[&str]) -> ProviderRow {
    token_probe(
        secrets,
        live,
        raws,
        TokenProbe {
            provider: "Apify",
            field: APIFY_TOKEN,
            missing: "No apify_api_token.",
            api: "Apify API",
            url: "https://api.apify.com/v2/users/me",
            operation: "GET /v2/users/me",
            files: "crates/fabr-agent/src/tools_contract.rs",
            authed: "The token was accepted. No Actor was run, so this is not FUNCTIONAL_TEST_PASSED or INTEGRATED_IN_FABR.",
        },
    )
}

fn bright_data(secrets: &Secrets, live: bool, raws: &[&str]) -> ProviderRow {
    token_probe(
        secrets,
        live,
        raws,
        TokenProbe {
            provider: "Bright Data",
            field: BRIGHTDATA_TOKEN,
            missing: "No brightdata_api_token.",
            api: "Bright Data API",
            url: "https://api.brightdata.com/zone/get_active_zones",
            operation: "GET /zone/get_active_zones",
            files: "crates/fabr-agent/src/tools_contract.rs",
            authed: "The token was accepted. No live public page was retrieved, so this is not FUNCTIONAL_TEST_PASSED or INTEGRATED_IN_FABR.",
        },
    )
}

struct TokenProbe {
    provider: &'static str,
    field: &'static str,
    missing: &'static str,
    api: &'static str,
    url: &'static str,
    operation: &'static str,
    files: &'static str,
    authed: &'static str,
}

fn token_probe(secrets: &Secrets, live: bool, raws: &[&str], spec: TokenProbe) -> ProviderRow {
    let Some(token) = secrets.get(spec.field) else {
        return row(spec.provider, CredentialCheck::CredentialMissing, spec.api, "none", spec.files, spec.missing);
    };
    if !live {
        return row(spec.provider, CredentialCheck::Blocked, spec.api, "not sent", spec.files, "Live probe was not requested.");
    }
    let auth = format!("Bearer {token}");
    match http("GET", spec.url, &[("authorization", &auth)], None, raws) {
        Http::Status(401 | 403) => {
            row(spec.provider, CredentialCheck::AuthFailed, spec.api, spec.operation, spec.files, "The API rejected the token.")
        }
        Http::Ok(_) => row(spec.provider, CredentialCheck::Authenticated, spec.api, spec.operation, spec.files, spec.authed),
        Http::Status(code) => {
            row(spec.provider, CredentialCheck::Blocked, spec.api, spec.operation, spec.files, &format!("HTTP {code}. Body omitted."))
        }
        Http::Transport => {
            row(spec.provider, CredentialCheck::Blocked, spec.api, spec.operation, spec.files, "The request did not complete.")
        }
    }
}

fn visa(secrets: &Secrets) -> ProviderRow {
    let cert = secrets.get(VISA_CERT_PATH);
    let key = secrets.get(VISA_KEY_PATH);
    if cert.is_none() || key.is_none() {
        return row(
            "Visa",
            CredentialCheck::CredentialMissing,
            "Visa developer sandbox",
            "none",
            "not wired",
            "visa_cert_path and visa_key_path are both required. No Visa API call was made.",
        );
    }
    let readable = cert.is_some_and(file_is_readable) && key.is_some_and(file_is_readable);
    if !readable {
        return row(
            "Visa",
            CredentialCheck::AuthFailed,
            "Visa developer sandbox",
            "none",
            "not wired",
            "A certificate path is set and the file is not readable. The path is omitted.",
        );
    }
    row(
        "Visa",
        CredentialCheck::Blocked,
        "Visa developer sandbox",
        "none",
        "not wired",
        "Certificate files are readable. No Visa product call was made, so this is not an integration.",
    )
}

fn nessie(secrets: &Secrets, live: bool, raws: &[&str]) -> ProviderRow {
    let Some(key) = secrets.get(NESSIE_KEY) else {
        return row(
            "Nessie",
            CredentialCheck::CredentialMissing,
            "Nessie sandbox",
            "none",
            "crates/fabr-agent/src/tools_contract.rs",
            "No nessie_api_key.",
        );
    };
    if !live {
        return row(
            "Nessie",
            CredentialCheck::Blocked,
            "Nessie sandbox",
            "not sent",
            "crates/fabr-agent/src/tools_contract.rs",
            "Live probe was not requested.",
        );
    }
    let url = format!("https://api.nessieisreal.com/accounts?key={key}");
    match http("GET", &url, &[], None, raws) {
        Http::Status(401 | 403) => row(
            "Nessie",
            CredentialCheck::AuthFailed,
            "Nessie sandbox",
            "GET /accounts",
            "crates/fabr-agent/src/tools_contract.rs",
            "The sandbox rejected the key.",
        ),
        Http::Ok(_) => row(
            "Nessie",
            CredentialCheck::FunctionalTestPassed,
            "Nessie sandbox",
            "GET /accounts",
            "crates/fabr-agent/src/tools_contract.rs",
            "Account data came back from the sandbox and is simulated. The FABR tool is not dispatched, so this is not INTEGRATED_IN_FABR.",
        ),
        Http::Status(code) => row(
            "Nessie",
            CredentialCheck::Blocked,
            "Nessie sandbox",
            "GET /accounts",
            "crates/fabr-agent/src/tools_contract.rs",
            &format!("HTTP {code}. Body omitted."),
        ),
        Http::Transport => row(
            "Nessie",
            CredentialCheck::Blocked,
            "Nessie sandbox",
            "GET /accounts",
            "crates/fabr-agent/src/tools_contract.rs",
            "The request did not complete.",
        ),
    }
}

fn figma() -> ProviderRow {
    row(
        "Figma",
        CredentialCheck::Blocked,
        "none",
        "none",
        "not wired",
        "No runtime secret is required for manual slides. Submission slides are not in the repo.",
    )
}

fn microsoft() -> ProviderRow {
    row(
        "Microsoft",
        CredentialCheck::Blocked,
        "none",
        "none",
        "not wired",
        "No Microsoft API credential is part of this architecture. Azure was not configured. Track eligibility without Azure is unverified.",
    )
}

fn file_is_readable(path: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

fn row(
    provider: &'static str,
    check: CredentialCheck,
    api: &'static str,
    operation: &'static str,
    files: &'static str,
    limitation: &str,
) -> ProviderRow {
    ProviderRow { provider, check, api, operation, files, limitation: limitation.to_string() }
}

enum Http {
    Ok(String),
    Status(u16),
    Transport,
}

fn http(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&serde_json::Value>, raws: &[&str]) -> Http {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(25)).build();
    let req = match method {
        "GET" => agent.get(url),
        _ => agent.post(url),
    };
    let mut req = req.set("content-type", "application/json");
    for (name, value) in headers {
        req = req.set(name, value);
    }
    let sent = if let Some(body) = body { req.send_json(body) } else { req.call() };
    match sent {
        Ok(resp) => {
            let code = resp.status();
            let text = scrub(&resp.into_string().unwrap_or_default(), raws);
            if (200..300).contains(&code) { Http::Ok(text) } else { Http::Status(code) }
        }
        Err(ureq::Error::Status(code, resp)) => {
            let _ = scrub(&resp.into_string().unwrap_or_default(), raws);
            Http::Status(code)
        }
        Err(_) => Http::Transport,
    }
}

fn scrub(text: &str, raws: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in raws {
        if secret.len() >= 6 {
            out = out.replace(secret, "[redacted]");
        }
    }
    redact::redact(&out)
}

pub fn contains_secret(report_text: &str, fabric_space: &Path) -> bool {
    let secrets = Secrets::load(fabric_space);
    secrets.raws().iter().any(|secret| secret.len() >= 6 && report_text.contains(secret))
}

pub fn sponsor_tool_enabled(name: &str, fabric_space: &Path) -> bool {
    let keys = tools_contract::load_sponsor_keys(fabric_space);
    tools_contract::configured_tools(&keys).iter().any(|tool| tool.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_sponsor_fields_stay_missing_and_do_not_leak() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("secrets.toml"), "anthropic_api_key = \"\"\nmoss_project_id = \"only-id\"\n").unwrap();
        let text = report(tmp.path(), "claude-sonnet-5-5", false);
        assert!(text.contains("Anthropic** — CREDENTIAL_MISSING") || text.contains("**Anthropic** — CREDENTIAL_MISSING"));
        assert!(text.contains("**Moss** — CREDENTIAL_MISSING"));
        assert!(text.contains("**Fo** — CREDENTIAL_MISSING"));
        assert!(text.contains("**Cartesia** — CREDENTIAL_MISSING"));
        assert!(text.contains("**Visa** — CREDENTIAL_MISSING"));
        assert!(text.contains("**Nessie** — CREDENTIAL_MISSING"));
        assert!(!text.contains("only-id"));
        assert!(!text.contains("INTEGRATED_IN_FABR"));
        assert!(!text.contains("FUNCTIONAL_TEST_PASSED"));
        assert!(!sponsor_tool_enabled("moss_upload", tmp.path()));
    }

    #[test]
    fn scrub_removes_a_bearer_value() {
        let secret = "wak_supersecretvalue";
        let out = scrub(&format!("Authorization: Bearer {secret}"), &[secret]);
        assert!(!out.contains(secret));
        assert!(out.contains("[redacted]"));
    }

    #[test]
    fn one_moss_field_does_not_enable_upload() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("secrets.toml"), "moss_project_key = \"present\"\n").unwrap();
        let rows = assess(tmp.path(), "claude-sonnet-5-5", false);
        let moss = rows.iter().find(|row| row.provider == "Moss").unwrap();
        assert_eq!(moss.check, CredentialCheck::CredentialMissing);
        assert!(!sponsor_tool_enabled("moss_upload", tmp.path()));
    }

    #[test]
    #[ignore = "calls providers with the local secrets file"]
    fn live_home_report_has_no_secret_values() {
        let space = Path::new(&std::env::var("HOME").unwrap()).join(".fabric-space");
        let text = report(&space, "claude-sonnet-5-5", true);
        assert!(!contains_secret(&text, &space), "report leaked a secret");
        let dest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/sponsor-credentials.md");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, text).unwrap();
    }
}
