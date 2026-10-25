//! Tool descriptor, grant check, and execution decisions.
//!
//! A registered tool is not authorized. The model callable set is the tools
//! that are available for this task, covered by this grant, and named by the
//! approved plan. Connectors return results; only the lifecycle writes the
//! task record.

use crate::model::ToolCall;
use crate::toolset::ToolSpec;
use fabr_core::FabrError;
use fabr_core::taskrec::{BoundAction, Grant, PlanMeta};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

pub const MOSS_PROJECT_ID: &str = "moss_project_id";
pub const MOSS_PROJECT_KEY: &str = "moss_project_key";
/// Bearer token for `https://mcp.api.wajo.ai/fo` (`Authorization: Bearer wak_...`).
pub const FO_KEY: &str = "fo_api_key";
pub const APIFY_TOKEN: &str = "apify_api_token";
pub const BRIGHTDATA_TOKEN: &str = "brightdata_api_token";
pub const NESSIE_KEY: &str = "nessie_api_key";
pub const CARTESIA_KEY: &str = "cartesia_api_key";
pub const VISA_CERT_PATH: &str = "visa_cert_path";
pub const VISA_KEY_PATH: &str = "visa_key_path";

const SPONSOR_KEYS: &[&str] =
    &[MOSS_PROJECT_ID, MOSS_PROJECT_KEY, FO_KEY, APIFY_TOKEN, BRIGHTDATA_TOKEN, NESSIE_KEY, CARTESIA_KEY, VISA_CERT_PATH, VISA_KEY_PATH];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Local,
    Moss,
    Fo,
    Apify,
    BrightData,
    Visa,
    Nessie,
    Cartesia,
}

impl Provider {
    /// Whether every required secret name is present. Presence is not authorization
    /// and is not a successful provider call.
    pub fn configured(self, keys: &SponsorKeys) -> bool {
        match self {
            Provider::Local => true,
            Provider::Moss => keys.has(MOSS_PROJECT_ID) && keys.has(MOSS_PROJECT_KEY),
            Provider::Fo => keys.has(FO_KEY),
            Provider::Apify => keys.has(APIFY_TOKEN),
            Provider::BrightData => keys.has(BRIGHTDATA_TOKEN),
            Provider::Visa => keys.has(VISA_CERT_PATH) && keys.has(VISA_KEY_PATH),
            Provider::Nessie => keys.has(NESSIE_KEY),
            Provider::Cartesia => keys.has(CARTESIA_KEY),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    LocalRead,
    RemoteRead,
    LocalWrite,
    ExternalEffect,
    Payment,
}

impl Effect {
    pub fn label(self) -> &'static str {
        match self {
            Effect::LocalRead => "local_read",
            Effect::RemoteRead => "remote_read",
            Effect::LocalWrite => "local_write",
            Effect::ExternalEffect => "external_effect",
            Effect::Payment => "payment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantClass {
    LocalRead,
    ExternalUpload,
    LocalWrite,
    ExternalAction,
    PurchaseFinance,
}

impl GrantClass {
    pub fn field(self) -> &'static str {
        match self {
            GrantClass::LocalRead => "local_read",
            GrantClass::ExternalUpload => "external_upload",
            GrantClass::LocalWrite => "local_write",
            GrantClass::ExternalAction => "external_action",
            GrantClass::PurchaseFinance => "purchase_finance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idempotency {
    /// Auto-replay is allowed only when the prior result is verified.
    Proven,
    /// A timeout or a missing result is UNKNOWN and is not auto-replayed.
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    FileContent,
    IndependentProviderRecord,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    pub configured: bool,
    pub enabled: bool,
    pub healthy: bool,
    pub detail: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolDescriptor {
    pub name: &'static str,
    pub provider: Provider,
    pub input_schema: Value,
    pub grant: GrantClass,
    pub effect: Effect,
    pub data_egress: bool,
    pub reversible: bool,
    pub idempotency: Idempotency,
    pub evidence: Evidence,
    pub availability: Availability,
    pub description: &'static str,
    /// The lifecycle can dispatch this tool. Catalog presence is not enough.
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SponsorKeys {
    present: BTreeSet<String>,
}

impl SponsorKeys {
    pub fn empty() -> Self {
        Self { present: BTreeSet::new() }
    }

    pub fn parse(text: &str) -> Self {
        let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
            return Self::empty();
        };
        let mut present = BTreeSet::new();
        for key in SPONSOR_KEYS {
            if secret_in(&doc, key).is_some() {
                present.insert((*key).to_string());
            }
        }
        Self { present }
    }

    pub fn has(&self, key: &str) -> bool {
        self.present.contains(key)
    }
}

pub fn load_sponsor_keys(fabric_space: &Path) -> SponsorKeys {
    let path = fabric_space.join("secrets.toml");
    let mut keys = match std::fs::read_to_string(&path) {
        Ok(text) => SponsorKeys::parse(&text),
        Err(_) => SponsorKeys::empty(),
    };
    for key in SPONSOR_KEYS {
        if env_names(key).iter().any(|name| std::env::var(name).ok().is_some_and(|value| !value.trim().is_empty())) {
            keys.present.insert((*key).to_string());
        }
    }
    keys
}

fn env_names(canonical: &str) -> &'static [&'static str] {
    match canonical {
        name if name == BRIGHTDATA_TOKEN => &["BRIGHTDATA_API_TOKEN", "API_TOKEN"],
        name if name == MOSS_PROJECT_ID => &["MOSS_PROJECT_ID"],
        name if name == MOSS_PROJECT_KEY => &["MOSS_PROJECT_KEY"],
        name if name == FO_KEY => &["FO_API_KEY", "FO_API_KE"],
        name if name == CARTESIA_KEY => &["CARTESIA_API_KEY"],
        name if name == APIFY_TOKEN => &["APIFY_API_TOKEN"],
        name if name == NESSIE_KEY => &["NESSIE_API_KEY", "NESSI_API_KEY"],
        _ => &[],
    }
}

/// The secret value for a connector. Callers must not log it.
pub fn secret_value(fabric_space: &Path, canonical: &str) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(fabric_space.join("secrets.toml"))
        && let Ok(doc) = text.parse::<toml_edit::DocumentMut>()
        && let Some(value) = secret_in(&doc, canonical)
    {
        fabr_core::redact::register_secret(&value, "[redacted]");
        return Some(value);
    }
    for name in env_names(canonical) {
        if let Ok(value) = std::env::var(name) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                fabr_core::redact::register_secret(&value, "[redacted]");
                return Some(value);
            }
        }
    }
    None
}

/// Read a secret by its canonical name, including field names already present
/// in the local file. The value is returned to the caller and is not logged.
pub fn secret_in(doc: &toml_edit::DocumentMut, canonical: &str) -> Option<String> {
    let mut names = vec![canonical];
    match canonical {
        name if name == MOSS_PROJECT_ID => names.push("MOSS_PROJECT_ID"),
        name if name == MOSS_PROJECT_KEY => names.push("MOSS_PROJECT_KEY"),
        name if name == FO_KEY => names.push("fo_api_ke"),
        name if name == NESSIE_KEY => names.push("nessi_api_key"),
        name if name == APIFY_TOKEN => names.push("apify_api_key"),
        _ => {}
    }
    for name in names {
        if let Some(value) = doc.get(name).and_then(|item| item.as_str()).map(str::trim).filter(|value| !value.is_empty()) {
            return Some(value.to_string());
        }
    }
    None
}

pub fn holds_grant(granted: &[Grant], class: GrantClass) -> bool {
    granted.iter().any(|g| g.field == class.field())
}

/// Grant classes do not imply each other. Local read is not an upload grant.
pub fn grant_authorizes(held: GrantClass, required: GrantClass) -> bool {
    held == required
}

struct Spec {
    name: &'static str,
    provider: Provider,
    grant: GrantClass,
    effect: Effect,
    data_egress: bool,
    reversible: bool,
    idempotency: Idempotency,
    evidence: Evidence,
    description: &'static str,
    executable: bool,
    schema: fn() -> Value,
}

fn object_schema(properties: &[(&str, &str)], required: &[&str]) -> Value {
    let mut props = Map::new();
    for (name, ty) in properties {
        props.insert((*name).to_string(), serde_json::json!({ "type": ty }));
    }
    serde_json::json!({
        "type": "object",
        "properties": props,
        "required": required,
    })
}

fn catalog_specs() -> &'static [Spec] {
    &[
        Spec {
            name: "read_file",
            provider: Provider::Local,
            grant: GrantClass::LocalRead,
            effect: Effect::LocalRead,
            data_egress: false,
            reversible: true,
            idempotency: Idempotency::Proven,
            evidence: Evidence::FileContent,
            description: "Read a file inside the node",
            executable: true,
            schema: || object_schema(&[("path", "string")], &["path"]),
        },
        Spec {
            name: "write_file",
            provider: Provider::Local,
            grant: GrantClass::LocalWrite,
            effect: Effect::LocalWrite,
            data_egress: false,
            reversible: true,
            idempotency: Idempotency::Proven,
            evidence: Evidence::FileContent,
            description: "Write a file inside the node",
            executable: true,
            schema: || object_schema(&[("path", "string"), ("content", "string")], &["path", "content"]),
        },
        Spec {
            name: "list_dir",
            provider: Provider::Local,
            grant: GrantClass::LocalRead,
            effect: Effect::LocalRead,
            data_egress: false,
            reversible: true,
            idempotency: Idempotency::Proven,
            evidence: Evidence::FileContent,
            description: "List a directory inside the node",
            executable: true,
            schema: || object_schema(&[("path", "string")], &["path"]),
        },
        Spec {
            name: "make_dir",
            provider: Provider::Local,
            grant: GrantClass::LocalWrite,
            effect: Effect::LocalWrite,
            data_egress: false,
            reversible: true,
            idempotency: Idempotency::Proven,
            evidence: Evidence::FileContent,
            description: "Create a folder inside the node",
            executable: true,
            schema: || object_schema(&[("path", "string")], &["path"]),
        },
        Spec {
            name: "recall_preferences",
            provider: Provider::Local,
            grant: GrantClass::LocalRead,
            effect: Effect::LocalRead,
            data_egress: false,
            reversible: true,
            idempotency: Idempotency::Proven,
            evidence: Evidence::FileContent,
            description: "Recall confirmed learned preferences from the local file",
            executable: true,
            schema: || object_schema(&[("query", "string")], &[]),
        },
        Spec {
            name: "propose_memory",
            provider: Provider::Local,
            grant: GrantClass::LocalWrite,
            effect: Effect::LocalWrite,
            data_egress: false,
            reversible: true,
            idempotency: Idempotency::Never,
            evidence: Evidence::FileContent,
            description: "Propose a learned preference. It is not recalled until confirmed",
            executable: true,
            schema: || object_schema(&[("text", "string"), ("source", "string"), ("scope", "string")], &["text"]),
        },
        Spec {
            name: "confirm_memory",
            provider: Provider::Local,
            grant: GrantClass::LocalWrite,
            effect: Effect::LocalWrite,
            data_egress: false,
            reversible: true,
            idempotency: Idempotency::Proven,
            evidence: Evidence::FileContent,
            description: "Confirm a proposed learned preference",
            executable: true,
            schema: || object_schema(&[("id", "string")], &["id"]),
        },
        Spec {
            name: "delete_memory",
            provider: Provider::Local,
            grant: GrantClass::LocalWrite,
            effect: Effect::LocalWrite,
            data_egress: false,
            reversible: true,
            idempotency: Idempotency::Proven,
            evidence: Evidence::FileContent,
            description: "Delete a learned preference so it is not recalled",
            executable: true,
            schema: || object_schema(&[("id", "string")], &["id"]),
        },
        Spec {
            name: "moss_upload",
            provider: Provider::Moss,
            grant: GrantClass::ExternalUpload,
            effect: Effect::ExternalEffect,
            data_egress: true,
            reversible: false,
            idempotency: Idempotency::Never,
            evidence: Evidence::IndependentProviderRecord,
            description: "Upload learned content to the Moss index. Local learned preferences stay authoritative.",
            executable: false,
            schema: || object_schema(&[("memory_id", "string")], &["memory_id"]),
        },
        Spec {
            name: "moss_list",
            provider: Provider::Moss,
            grant: GrantClass::ExternalAction,
            effect: Effect::RemoteRead,
            data_egress: true,
            reversible: true,
            idempotency: Idempotency::Never,
            evidence: Evidence::IndependentProviderRecord,
            description: "List Moss index names. Does not upload the local learned file.",
            executable: true,
            schema: || object_schema(&[], &[]),
        },
        Spec {
            name: "moss_retrieve",
            provider: Provider::Moss,
            grant: GrantClass::ExternalAction,
            effect: Effect::RemoteRead,
            data_egress: true,
            reversible: true,
            idempotency: Idempotency::Never,
            evidence: Evidence::IndependentProviderRecord,
            description: "Retrieve documents from one Moss index. Does not replace the local learned file.",
            executable: true,
            schema: || object_schema(&[("index", "string")], &["index"]),
        },
        Spec {
            name: "fo_action",
            provider: Provider::Fo,
            grant: GrantClass::ExternalAction,
            effect: Effect::ExternalEffect,
            data_egress: true,
            reversible: false,
            idempotency: Idempotency::Never,
            evidence: Evidence::IndependentProviderRecord,
            description: "Run one Fo action after tools/list discovers it. Not integrated in this build.",
            executable: false,
            schema: || {
                object_schema(&[("tool", "string"), ("params", "string"), ("destination", "string")], &["tool", "params", "destination"])
            },
        },
        Spec {
            name: "apify_lookup",
            provider: Provider::Apify,
            grant: GrantClass::ExternalAction,
            effect: Effect::RemoteRead,
            data_egress: true,
            reversible: true,
            idempotency: Idempotency::Proven,
            evidence: Evidence::IndependentProviderRecord,
            description: "Run one Apify Actor and return its dataset items",
            executable: true,
            schema: || object_schema(&[("actor", "string"), ("input", "string")], &["actor", "input"]),
        },
        Spec {
            name: "bright_data_search",
            provider: Provider::BrightData,
            grant: GrantClass::ExternalAction,
            effect: Effect::RemoteRead,
            data_egress: true,
            reversible: true,
            idempotency: Idempotency::Never,
            evidence: Evidence::IndependentProviderRecord,
            description: "Search public web pages through Bright Data. No login, paywall, or personal data.",
            executable: true,
            schema: || object_schema(&[("query", "string")], &["query"]),
        },
        Spec {
            name: "bright_data_extract",
            provider: Provider::BrightData,
            grant: GrantClass::ExternalAction,
            effect: Effect::RemoteRead,
            data_egress: true,
            reversible: true,
            idempotency: Idempotency::Never,
            evidence: Evidence::IndependentProviderRecord,
            description: "Extract one public http page through Bright Data. No login, paywall, or personal data.",
            executable: true,
            schema: || object_schema(&[("url", "string")], &["url"]),
        },
        Spec {
            name: "visa_checkout",
            provider: Provider::Visa,
            grant: GrantClass::PurchaseFinance,
            effect: Effect::Payment,
            data_egress: true,
            reversible: false,
            idempotency: Idempotency::Never,
            evidence: Evidence::IndependentProviderRecord,
            description: "Start a Visa checkout",
            executable: false,
            schema: || {
                object_schema(&[("merchant", "string"), ("total", "string"), ("currency", "string")], &["merchant", "total", "currency"])
            },
        },
        Spec {
            name: "nessie_sandbox",
            provider: Provider::Nessie,
            grant: GrantClass::PurchaseFinance,
            effect: Effect::Payment,
            data_egress: true,
            reversible: false,
            idempotency: Idempotency::Never,
            evidence: Evidence::IndependentProviderRecord,
            description: "Read labeled Nessie sandbox accounts. This is not internship memory.",
            executable: true,
            schema: || object_schema(&[("account_id", "string")], &[]),
        },
        Spec {
            name: "cartesia_speak",
            provider: Provider::Cartesia,
            grant: GrantClass::ExternalAction,
            effect: Effect::ExternalEffect,
            data_egress: true,
            reversible: true,
            idempotency: Idempotency::Never,
            evidence: Evidence::FileContent,
            description: "Synthesize speech with Cartesia and write a wav inside the node. This is not the voice demo.",
            executable: true,
            schema: || {
                object_schema(&[("transcript", "string"), ("voice", "string"), ("path", "string")], &["transcript", "voice", "path"])
            },
        },
        Spec {
            name: "cartesia_transcribe",
            provider: Provider::Cartesia,
            grant: GrantClass::ExternalAction,
            effect: Effect::RemoteRead,
            data_egress: true,
            reversible: true,
            idempotency: Idempotency::Never,
            evidence: Evidence::IndependentProviderRecord,
            description: "Transcribe one audio file inside the node with Cartesia. The file bytes leave the machine.",
            executable: true,
            schema: || object_schema(&[("path", "string")], &["path"]),
        },
    ]
}

fn availability_for(provider: Provider, keys: &SponsorKeys) -> Availability {
    if provider == Provider::Local {
        return Availability { configured: true, enabled: true, healthy: true, detail: "local" };
    }
    if provider.configured(keys) {
        Availability { configured: true, enabled: true, healthy: true, detail: "credential present; provider health not probed" }
    } else {
        Availability { configured: false, enabled: false, healthy: false, detail: "missing api key" }
    }
}

pub fn catalog(keys: &SponsorKeys) -> Vec<ToolDescriptor> {
    catalog_specs()
        .iter()
        .map(|spec| ToolDescriptor {
            name: spec.name,
            provider: spec.provider,
            input_schema: (spec.schema)(),
            grant: spec.grant,
            effect: spec.effect,
            data_egress: spec.data_egress,
            reversible: spec.reversible,
            idempotency: spec.idempotency,
            evidence: spec.evidence,
            availability: availability_for(spec.provider, keys),
            description: spec.description,
            executable: spec.executable,
        })
        .collect()
}

/// Tools whose credential is present. A missing sponsor key removes that tool.
pub fn configured_tools(keys: &SponsorKeys) -> Vec<ToolDescriptor> {
    catalog(keys).into_iter().filter(|t| t.availability.enabled).collect()
}

fn needs_explicit_grant(tool: &ToolDescriptor) -> bool {
    tool.data_egress || matches!(tool.effect, Effect::RemoteRead | Effect::ExternalEffect | Effect::Payment)
}

pub fn authorized(tool: &ToolDescriptor, granted: &[Grant], approved: &[String]) -> bool {
    if !approved.iter().any(|name| name == tool.name) {
        return false;
    }
    if needs_explicit_grant(tool) {
        return holds_grant(granted, tool.grant);
    }
    true
}

/// Tools the model may see and the executor may dispatch for this task.
pub fn model_callable(keys: &SponsorKeys, granted: &[Grant], approved: &[String]) -> Vec<ToolDescriptor> {
    catalog(keys).into_iter().filter(|tool| tool.executable && tool.availability.enabled && authorized(tool, granted, approved)).collect()
}

pub fn executable_name(name: &str) -> bool {
    catalog(&SponsorKeys::empty()).iter().any(|tool| tool.executable && tool.name == name)
}

pub fn executable_names() -> Vec<&'static str> {
    catalog(&SponsorKeys::empty()).into_iter().filter(|tool| tool.executable).map(|tool| tool.name).collect()
}

pub fn in_catalog(name: &str) -> bool {
    catalog(&SponsorKeys::empty()).iter().any(|tool| tool.name == name)
}

/// The planner may name this tool only when it is executable and its credential is present.
pub fn offered(name: &str, keys: &SponsorKeys) -> bool {
    catalog(keys).iter().any(|tool| tool.executable && tool.availability.enabled && tool.name == name)
}

/// External tools need a bound operation. A tool name in the plan is not enough.
pub fn must_bind(name: &str) -> bool {
    catalog(&SponsorKeys::empty()).iter().any(|tool| tool.executable && tool.name == name && tool.provider != Provider::Local)
}

/// Names, enabled/disabled, and input schemas. Secret values and secret paths are omitted.
pub fn planner_view(keys: &SponsorKeys) -> String {
    catalog(keys)
        .into_iter()
        .filter(|tool| tool.executable)
        .map(|tool| {
            let state = if tool.availability.enabled { "enabled" } else { "disabled" };
            format!("- {} ({state}): {} schema {}", tool.name, tool.description, tool.input_schema)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn destination_of(tool: &str, params: &Value) -> String {
    let key = match tool {
        "bright_data_extract" => "url",
        "bright_data_search" => "query",
        "fo_action" => "destination",
        "recall_preferences" => "query",
        "propose_memory" => "text",
        "confirm_memory" | "delete_memory" => "id",
        "moss_upload" => "memory_id",
        "moss_retrieve" => "index",
        "cartesia_speak" => "transcript",
        "cartesia_transcribe" | "read_file" | "write_file" | "list_dir" | "make_dir" => "path",
        "nessie_sandbox" => "account_id",
        "apify_lookup" => "actor",
        _ => "destination",
    };
    let fallback = if key == "destination" { None } else { params.get("destination").and_then(|v| v.as_str()) };
    params.get(key).and_then(|v| v.as_str()).or(fallback).unwrap_or("").to_string()
}

pub fn external_error_is_unknown(error: &str) -> bool {
    matches!(error, "timeout" | "unavailable")
}

pub fn plan_tool_names(keys: &SponsorKeys) -> Vec<&'static str> {
    catalog(keys).into_iter().filter(|tool| tool.executable && tool.availability.enabled).map(|tool| tool.name).collect()
}

pub fn to_advertised(tools: &[ToolDescriptor]) -> Vec<ToolSpec> {
    tools
        .iter()
        .map(|tool| ToolSpec {
            name: tool.name.to_string(),
            description: tool.description.to_string(),
            input_schema: tool.input_schema.clone(),
            read_only: false,
        })
        .collect()
}

pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> =
                keys.iter().map(|k| format!("{}:{}", serde_json::to_string(k).unwrap_or_default(), canonical_json(&map[*k]))).collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".into()),
    }
}

pub fn call_params(call: &ToolCall) -> Value {
    if let Some(params) = &call.params
        && params.as_object().is_some_and(|m| !m.is_empty())
    {
        return params.clone();
    }
    let mut map = Map::new();
    if !call.path.is_empty() {
        map.insert("path".into(), Value::String(call.path.clone()));
    }
    if let Some(content) = &call.content {
        map.insert("content".into(), Value::String(content.clone()));
    }
    Value::Object(map)
}

pub fn bind_action(meta: &mut PlanMeta, tool: &str, params: &Value) -> Result<(), FabrError> {
    let Some(desc) = catalog(&SponsorKeys::empty()).into_iter().find(|item| item.name == tool) else {
        return Err(FabrError::BadRequest(format!("unknown tool {tool}")));
    };
    schema_ok(&desc.input_schema, params)?;
    let action = BoundAction {
        tool: tool.to_string(),
        params: canonical_json(params),
        destination: destination_of(tool, params),
        effect: desc.effect.label().to_string(),
        grant_class: desc.grant.field().to_string(),
    };
    if let Some(existing) = meta.bound_actions.iter_mut().find(|item| item.tool == action.tool && item.params == action.params) {
        *existing = action;
    } else {
        meta.bound_actions.push(action);
    }
    Ok(())
}

pub fn action_covers(bound: &[BoundAction], tool_name: &str, params: &Value) -> bool {
    let Some(desc) = catalog(&SponsorKeys::empty()).into_iter().find(|item| item.name == tool_name) else {
        return false;
    };
    let canon = canonical_json(params);
    let destination = destination_of(tool_name, params);
    bound.iter().any(|action| {
        action.tool == tool_name
            && action.params == canon
            && action.destination == destination
            && action.effect == desc.effect.label()
            && action.grant_class == desc.grant.field()
    })
}

pub fn execution_allowed(
    tool_name: &str,
    keys: &SponsorKeys,
    granted: &[Grant],
    approved: &[String],
    bound: &[BoundAction],
    params: &Value,
) -> Result<(), FabrError> {
    let Some(tool) = catalog(keys).into_iter().find(|item| item.name == tool_name) else {
        return Err(FabrError::BadRequest(format!("unknown tool {tool_name}")));
    };
    if !tool.executable {
        return Err(FabrError::BadRequest(format!("tool {tool_name} is not integrated")));
    }
    if !tool.availability.enabled {
        return Err(FabrError::BadRequest(format!("tool {tool_name} is unavailable")));
    }
    if matches!(tool_name, "read_file" | "write_file" | "list_dir" | "make_dir") {
        if bound.iter().any(|action| action.tool == tool_name) && !action_covers(bound, tool_name, params) {
            return Err(FabrError::BadRequest(format!("approval does not cover this {tool_name} action")));
        }
        return Ok(());
    }
    if !authorized(&tool, granted, approved) {
        return Err(FabrError::BadRequest(format!("approval does not cover tool {tool_name}")));
    }
    if tool.provider != Provider::Local {
        schema_ok(&tool.input_schema, params)?;
        if !action_covers(bound, tool_name, params) {
            return Err(FabrError::BadRequest(format!("approval does not cover this {tool_name} action")));
        }
    } else if matches!(tool_name, "recall_preferences" | "propose_memory" | "confirm_memory" | "delete_memory") {
        schema_ok(&tool.input_schema, params)?;
    }
    Ok(())
}

pub fn approved_params_match(bound: &[BoundAction], tool: &str, params: &Value) -> bool {
    let canon = canonical_json(params);
    bound.iter().any(|action| action.tool == tool && action.params == canon)
}

fn schema_ok(schema: &Value, params: &Value) -> Result<(), FabrError> {
    let obj = params.as_object().ok_or_else(|| FabrError::BadRequest("tool params must be an object".into()))?;
    let required = schema.get("required").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let properties = schema.get("properties").and_then(|v| v.as_object());
    for req in required {
        let Some(name) = req.as_str() else { continue };
        if !obj.contains_key(name) {
            return Err(FabrError::BadRequest(format!("missing param {name}")));
        }
    }
    if let Some(properties) = properties {
        for (key, value) in obj {
            let Some(prop) = properties.get(key) else {
                return Err(FabrError::BadRequest(format!("unknown param {key}")));
            };
            let expected = prop.get("type").and_then(|t| t.as_str()).unwrap_or("string");
            let ok = match expected {
                "string" => value.is_string(),
                "number" => value.is_number(),
                "boolean" => value.is_boolean(),
                _ => true,
            };
            if !ok {
                return Err(FabrError::BadRequest(format!("param {key} has the wrong type")));
            }
        }
    }
    Ok(())
}

pub fn validate_call(call: &ToolCall, keys: &SponsorKeys) -> Result<(), FabrError> {
    let Some(tool) = catalog(keys).into_iter().find(|tool| tool.name == call.name) else {
        return Err(FabrError::BadRequest(format!("unknown tool {}", call.name)));
    };
    if !tool.availability.enabled {
        return Err(FabrError::BadRequest(format!("tool {} is unavailable", call.name)));
    }
    schema_ok(&tool.input_schema, &call_params(call))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteReplay {
    Apply,
    SkipVerified,
    DoNotReplay,
}

pub fn write_replay(node_dir: &Path, log: &str, path: &str, content: &str) -> WriteReplay {
    if !log_has_write(log, path) {
        return WriteReplay::Apply;
    }
    match crate::toolset::read_node_text(node_dir, path) {
        Ok(body) if body == content => WriteReplay::SkipVerified,
        _ => WriteReplay::DoNotReplay,
    }
}

pub fn log_has_write(log: &str, path: &str) -> bool {
    let needle = format!("execute: write_file {path}");
    log.lines().any(|line| line.contains(&needle))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalPrep {
    Skip,
    Apply,
}

pub fn prepare_local(node_dir: &Path, log: &str, call: &ToolCall, keys: &SponsorKeys) -> Result<LocalPrep, FabrError> {
    validate_call(call, keys)?;
    if call.name == "write_file" {
        let content = call
            .content
            .clone()
            .or_else(|| call_params(call).get("content").and_then(|v| v.as_str()).map(str::to_string))
            .unwrap_or_default();
        let path = if call.path.is_empty() {
            call_params(call).get("path").and_then(|v| v.as_str()).unwrap_or("").to_string()
        } else {
            call.path.clone()
        };
        return match write_replay(node_dir, log, &path, &content) {
            WriteReplay::Apply => Ok(LocalPrep::Apply),
            WriteReplay::SkipVerified | WriteReplay::DoNotReplay => Ok(LocalPrep::Skip),
        };
    }
    Ok(LocalPrep::Apply)
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn invocation_id(task_id: &str, tool: &str, params: &Value) -> String {
    let material = format!("{task_id}\n{tool}\n{}", canonical_json(params));
    format!("inv_{:016x}", fnv64(material.as_bytes()))
}

pub fn external_intent_message(id: &str, tool: &str) -> String {
    format!("external {id} {tool} intent")
}

pub fn external_unknown_message(id: &str, tool: &str, reason: &str) -> String {
    format!("external {id} {tool} UNKNOWN {reason}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalOutcome {
    Succeeded { evidence: String },
    Failed { message: String },
    Unknown { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalMark {
    Intent,
    Unknown,
    Succeeded,
    Failed,
}

fn external_marks(log: &str) -> Vec<(String, String, ExternalMark)> {
    let mut out = Vec::new();
    for line in log.lines() {
        let Some((_, action)) = line.split_once("execute:") else { continue };
        let Some(rest) = action.trim().strip_prefix("external ") else { continue };
        let mut parts = rest.splitn(4, ' ');
        let Some(id) = parts.next() else { continue };
        let Some(tool) = parts.next() else { continue };
        let kind = parts.next().unwrap_or("");
        let mark = match kind {
            "intent" => ExternalMark::Intent,
            "UNKNOWN" => ExternalMark::Unknown,
            "succeeded" => ExternalMark::Succeeded,
            "failed" => ExternalMark::Failed,
            _ => continue,
        };
        out.push((id.to_string(), tool.to_string(), mark));
    }
    out
}

/// When an external intent has no verified result, return the UNKNOWN line to record.
/// Ambiguous external work is not dispatched again.
pub fn unresolved_external_message(log: &str) -> Option<String> {
    let marks = external_marks(log);
    let mut pending: Vec<(String, String)> = Vec::new();
    for (id, tool, mark) in &marks {
        match mark {
            ExternalMark::Intent => pending.push((id.clone(), tool.clone())),
            ExternalMark::Unknown | ExternalMark::Succeeded | ExternalMark::Failed => {
                pending.retain(|(have, _)| have != id);
            }
        }
    }
    let (id, tool) = pending.into_iter().next()?;
    Some(external_unknown_message(&id, &tool, "timed out before a verified result"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalReplay {
    Dispatch,
    DoNotReplay,
}

pub fn external_replay(log: &str, id: &str, idempotency: Idempotency) -> ExternalReplay {
    let started = external_marks(log).iter().any(|(have, _, mark)| {
        have == id && matches!(mark, ExternalMark::Intent | ExternalMark::Unknown | ExternalMark::Succeeded | ExternalMark::Failed)
    });
    if !started {
        return ExternalReplay::Dispatch;
    }
    // A started external effect is not dispatched again. Proven idempotency
    // still does not retry; the caller reconciles UNKNOWN instead.
    match idempotency {
        Idempotency::Proven | Idempotency::Never => ExternalReplay::DoNotReplay,
    }
}

/// Append the intent, run `dispatch` once, and record the outcome.
/// A second call does not dispatch. UNKNOWN is not replayed.
pub fn run_external<F>(
    log: &mut String,
    task_id: &str,
    tool: &str,
    params: &Value,
    idempotency: Idempotency,
    mut dispatch: F,
) -> ExternalOutcome
where
    F: FnMut() -> ExternalOutcome,
{
    let id = invocation_id(task_id, tool, params);
    if external_replay(log, &id, idempotency) == ExternalReplay::DoNotReplay {
        if unresolved_external_message(log).is_some() {
            push_execute(log, &external_unknown_message(&id, tool, "timed out before a verified result"));
        }
        return ExternalOutcome::Unknown { reason: "not replayed".into() };
    }
    push_execute(log, &external_intent_message(&id, tool));
    let outcome = dispatch();
    let line = match &outcome {
        ExternalOutcome::Succeeded { evidence } => format!("external {id} {tool} succeeded {evidence}"),
        ExternalOutcome::Failed { message } => format!("external {id} {tool} failed {message}"),
        ExternalOutcome::Unknown { reason } => external_unknown_message(&id, tool, reason),
    };
    push_execute(log, &line);
    outcome
}

fn push_execute(log: &mut String, message: &str) {
    if !log.is_empty() && !log.ends_with('\n') {
        log.push('\n');
    }
    log.push_str("- 2026-10-24T00:00:00Z execute: ");
    log.push_str(message);
    log.push('\n');
}

pub fn is_external_action(action: &str) -> bool {
    action.starts_with("external ")
}

pub fn log_has_unknown(log: &str) -> bool {
    external_marks(log).iter().any(|(_, _, mark)| *mark == ExternalMark::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabr_core::taskrec::Grant;
    use serde_json::json;

    fn grant(field: &str) -> Grant {
        Grant { field: field.into(), scope: "task".into() }
    }

    fn keys(toml: &str) -> SponsorKeys {
        SponsorKeys::parse(toml)
    }

    #[test]
    fn plan_names_include_local_memory_and_not_unwired_sponsors() {
        let names = plan_tool_names(&SponsorKeys::empty());
        assert!(names.contains(&"recall_preferences"));
        assert!(names.contains(&"read_file"));
        assert!(!names.contains(&"moss_upload"));
        assert!(!names.contains(&"fo_action"));
        assert!(!names.contains(&"visa_checkout"));
        assert!(!names.contains(&"bright_data_search"));
        let named = plan_tool_names(&keys("brightdata_api_token = \"present\"\n"));
        assert!(named.contains(&"bright_data_search"));
        assert!(named.contains(&"bright_data_extract"));
        assert!(!named.contains(&"visa_checkout"));
        assert!(keys("MOSS_PROJECT_ID = \"id\"\nMOSS_PROJECT_KEY = \"key\"\n").has(MOSS_PROJECT_ID));
        assert!(keys("fo_api_ke = \"present\"\n").has(FO_KEY));
        assert!(keys("nessi_api_key = \"present\"\n").has(NESSIE_KEY));
        assert!(keys("apify_api_key = \"present\"\n").has(APIFY_TOKEN));
    }

    #[test]
    fn missing_sponsor_key_removes_the_tool() {
        let absent = configured_tools(&SponsorKeys::empty());
        assert!(absent.iter().all(|tool| tool.provider == Provider::Local));
        assert!(absent.iter().all(|tool| tool.name != "moss_upload"));
        let present = configured_tools(&keys("moss_project_id = \"id\"\nmoss_project_key = \"present\"\n"));
        assert!(present.iter().any(|tool| tool.name == "moss_upload" && tool.availability.enabled));
        let one = configured_tools(&keys("moss_project_key = \"present\"\n"));
        assert!(one.iter().all(|tool| tool.name != "moss_upload"));
        let blank = configured_tools(&keys("moss_project_id = \"\"\nmoss_project_key = \"\"\n"));
        assert!(blank.iter().all(|tool| tool.name != "moss_upload"));
    }

    #[test]
    fn local_read_grant_does_not_authorize_external_upload() {
        assert!(!grant_authorizes(GrantClass::LocalRead, GrantClass::ExternalUpload));
        let tool = configured_tools(&keys("moss_project_id = \"id\"\nmoss_project_key = \"present\"\n"))
            .into_iter()
            .find(|tool| tool.name == "moss_upload")
            .unwrap();
        let approved = vec!["moss_upload".to_string()];
        assert!(!authorized(&tool, &[grant("local_read")], &approved));
        assert!(authorized(&tool, &[grant("external_upload")], &approved));
        let callable =
            model_callable(&keys("moss_project_id = \"id\"\nmoss_project_key = \"present\"\n"), &[grant("local_read")], &approved);
        assert!(callable.iter().all(|tool| tool.name != "moss_upload"));
    }

    #[test]
    fn unapproved_tools_are_excluded_from_the_callable_set() {
        let approved = vec!["read_file".to_string()];
        let callable = model_callable(
            &keys("moss_project_id = \"id\"\nmoss_project_key = \"present\"\nfo_api_key = \"present\"\n"),
            &[grant("external_upload"), grant("external_action")],
            &approved,
        );
        let names: Vec<_> = callable.iter().map(|tool| tool.name).collect();
        assert_eq!(names, vec!["read_file"]);
        let advertised = to_advertised(&callable);
        assert!(advertised.iter().all(|tool| tool.name == "read_file"));
    }

    #[test]
    fn changed_bound_parameters_do_not_match() {
        let mut meta = PlanMeta {
            reversibility: Some("reversible".into()),
            tools: vec!["write_file".into()],
            assumptions: Vec::new(),
            checks: Vec::new(),
            bound_actions: Vec::new(),
            requires: Default::default(),
        };
        let original = json!({"path": "greet.txt", "content": "hi"});
        bind_action(&mut meta, "write_file", &original).unwrap();
        assert!(approved_params_match(&meta.bound_actions, "write_file", &original));
        let changed = json!({"content": "no", "path": "greet.txt"});
        assert!(!approved_params_match(&meta.bound_actions, "write_file", &changed));
        assert_ne!(canonical_json(&original), canonical_json(&changed));
        assert_eq!(canonical_json(&original), canonical_json(&json!({"content": "hi", "path": "greet.txt"})));
    }

    #[test]
    fn write_file_replays_only_the_same_verified_content() {
        let tmp = tempfile::tempdir().unwrap();
        let node = tmp.path();
        std::fs::write(node.join("greet.txt"), "hi").unwrap();
        let log = "- 2026-10-24T00:00:00Z execute: write_file greet.txt\n";
        assert_eq!(write_replay(node, log, "greet.txt", "hi"), WriteReplay::SkipVerified);
        assert_eq!(write_replay(node, log, "greet.txt", "HI-AGAIN"), WriteReplay::DoNotReplay);
        assert_eq!(write_replay(node, "- 2026-10-24T00:00:00Z execute: started\n", "greet.txt", "hi"), WriteReplay::Apply);
    }

    #[test]
    fn timed_out_external_effect_is_not_replayed() {
        let params = json!({"tool": "ping", "params": "{}"});
        let mut calls = 0;
        let mut log = String::new();
        let first = run_external(&mut log, "task-a", "fo_action", &params, Idempotency::Never, || {
            calls += 1;
            ExternalOutcome::Unknown { reason: "timed out".into() }
        });
        assert!(matches!(first, ExternalOutcome::Unknown { .. }));
        assert_eq!(calls, 1);
        assert!(log.contains("UNKNOWN"));
        assert!(log.contains("intent"));
        let second = run_external(&mut log, "task-a", "fo_action", &params, Idempotency::Never, || {
            calls += 1;
            ExternalOutcome::Succeeded { evidence: "should-not-run".into() }
        });
        assert!(matches!(second, ExternalOutcome::Unknown { .. }));
        assert_eq!(calls, 1);
        assert!(!log.contains("should-not-run"));
        assert!(unresolved_external_message(&log).is_none());
    }

    #[test]
    fn intent_without_a_result_asks_for_unknown() {
        let log = "- 2026-10-24T00:00:00Z execute: started\n- 2026-10-24T00:00:00Z execute: external inv_abc fo_action intent\n";
        let msg = unresolved_external_message(log).unwrap();
        assert!(msg.contains("UNKNOWN"));
        assert!(msg.contains("inv_abc"));
        assert_eq!(external_replay(log, "inv_abc", Idempotency::Never), ExternalReplay::DoNotReplay);
        assert!(!log_has_unknown(log));
        assert!(log_has_unknown(&format!("- 2026-10-24T00:00:00Z execute: {msg}")));
    }

    #[test]
    fn planner_view_lists_availability_without_secret_values() {
        let secret = "super-secret-token-value";
        let view = planner_view(&keys(&format!("brightdata_api_token = \"{secret}\"\n")));
        assert!(view.contains("bright_data_search (enabled)"));
        assert!(view.contains("write_file (enabled)"));
        assert!(!view.contains("visa_checkout"));
        assert!(!view.contains(secret));
        assert!(!view.contains("secrets.toml"));
        assert!(!view.contains("fo_action"));
        let absent = planner_view(&SponsorKeys::empty());
        assert!(absent.contains("bright_data_search (disabled)"));
        assert!(absent.contains("write_file (enabled)"));
        assert!(!absent.contains(secret));
        assert!(offered("write_file", &SponsorKeys::empty()));
        assert!(!offered("bright_data_search", &SponsorKeys::empty()));
        assert!(must_bind("bright_data_search"));
        assert!(!must_bind("write_file"));
        assert!(!must_bind("fo_action"));
    }

    #[test]
    fn fo_destination_binding_does_not_cover_a_different_destination() {
        let mut meta = PlanMeta {
            reversibility: Some("irreversible".into()),
            tools: vec!["fo_action".into()],
            assumptions: Vec::new(),
            checks: Vec::new(),
            bound_actions: Vec::new(),
            requires: Default::default(),
        };
        let first = json!({"tool": "send", "params": "{}", "destination": "ops@example.test"});
        bind_action(&mut meta, "fo_action", &first).unwrap();
        assert!(action_covers(&meta.bound_actions, "fo_action", &first));
        let other = json!({"tool": "send", "params": "{}", "destination": "other@example.test"});
        assert!(!action_covers(&meta.bound_actions, "fo_action", &other));
        assert_ne!(meta.bound_actions[0].destination, "other@example.test");
        assert_eq!(meta.bound_actions[0].grant_class, "external_action");
        assert_eq!(meta.bound_actions[0].effect, "external_effect");
    }

    #[test]
    fn extract_url_binding_rejects_a_different_url() {
        let mut meta = PlanMeta {
            reversibility: Some("reversible".into()),
            tools: vec!["bright_data_extract".into()],
            assumptions: Vec::new(),
            checks: Vec::new(),
            bound_actions: Vec::new(),
            requires: Default::default(),
        };
        let approved = json!({"url": "https://docs.rs/ureq"});
        bind_action(&mut meta, "bright_data_extract", &approved).unwrap();
        let keys = keys("brightdata_api_token = \"present\"\n");
        let granted = vec![grant("external_action")];
        let tools = vec!["bright_data_extract".to_string()];
        execution_allowed("bright_data_extract", &keys, &granted, &tools, &meta.bound_actions, &approved).unwrap();
        let other = json!({"url": "https://example.com/other"});
        let err = execution_allowed("bright_data_extract", &keys, &granted, &tools, &meta.bound_actions, &other).unwrap_err();
        assert!(err.message().contains("does not cover"));
        let unbound = execution_allowed("bright_data_extract", &keys, &granted, &tools, &[], &approved).unwrap_err();
        assert!(unbound.message().contains("does not cover"));
    }
}
