pub mod board;
pub mod caps;
pub mod connectors;
pub mod credentials;
pub mod lifecycle;
pub mod link;
pub mod model;
pub mod run;
pub mod runner;
pub mod settings;
pub mod sponsor_tools;
pub mod state;
pub mod store;
pub mod tools_contract;
pub mod toolset;
pub mod verify;

pub use caps::Harness;
pub use lifecycle::{Ctl, advance_with, approval_allows_execute, registry_path, task_path_key};
pub use model::{
    AnthropicModel, Block, CallObserver, DefineDraft, EvalVerdict, EvaluateInput, ExecuteInput, ExecuteTurn, JudgedItem, Judgement,
    Message, Model, ModelCall, PlanDraft, PlanInput, ScriptedModel, ScriptedTurn, ToolCall, ToolUse, TurnOutcome,
};
pub use run::run_once;
pub use settings::{AgentSettings, load_agent_settings};
pub use verify::spawn_allowed;

use fabr_core::FabrError;

pub(crate) fn now_utc() -> String {
    format!("{}Z", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S"))
}

pub(crate) fn require_local(source: &str) -> Result<(), FabrError> {
    if source == "local" { Ok(()) } else { Err(FabrError::BadRequest("only local sources are supported".into())) }
}
