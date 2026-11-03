use clap::Subcommand;
use fabr_core::FabrError;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug, Clone)]
pub enum SteerCmd {
    #[command(about = "Reopen a question for further exploration")]
    ExploreFurther { question_id: String },
    #[command(about = "Reject an assumption and mark dependent decisions stale")]
    RejectAssumption { assumption_id: String },
    #[command(about = "Add a new user constraint to guide exploration")]
    AddConstraint { text: String },
    #[command(about = "Prune an unpromising branch")]
    PruneBranch {
        branch_id: String,
        #[arg(long, default_value = "pruned by human")]
        reason: String,
    },
    #[command(about = "Authorize execution despite convergence gate failure")]
    Authorize {
        gate_id: String,
        #[arg(long, default_value = "authorized by human")]
        reason: String,
    },
    #[command(about = "Redirect exploration with new instructions")]
    Redirect { instructions: String },
    #[command(about = "Rewind exploration state to a previous checkpoint")]
    Rewind { checkpoint_id: String },
}

pub fn resolve_space_dir(folder: Option<PathBuf>) -> PathBuf {
    match folder {
        Some(f) => {
            if f.join(".fabric-space").exists() {
                f.join(".fabric-space")
            } else if f.ends_with(".fabric-space") {
                f
            } else {
                f.join(".fabric-space")
            }
        }
        None => {
            let cur = PathBuf::from(".");
            if cur.join(".fabric-space").exists() {
                cur.join(".fabric-space")
            } else {
                let ctx = fabr_core::setup::Ctx::from_env();
                if let Ok(res) = fabr_core::setup::settings_read(&ctx) {
                    if !res.settings.default_folder.is_empty() {
                        PathBuf::from(res.settings.default_folder).join(".fabric-space")
                    } else {
                        cur.join(".fabric-space")
                    }
                } else {
                    cur.join(".fabric-space")
                }
            }
        }
    }
}

pub fn resolve_run_id(space_dir: &Path, task_id: &str, run_id: Option<String>) -> Result<String, FabrError> {
    if let Some(r) = run_id {
        return Ok(r);
    }
    let runs = fabr_agent::lantern::list_runs(space_dir, task_id).unwrap_or_default();
    if let Some(last) = runs.last() {
        return Ok(last.run_id.clone());
    }
    let runs_dir = space_dir.join("runs").join(task_id);
    if runs_dir.exists() {
        let mut found = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&runs_dir) {
            for e in entries.flatten() {
                if e.path().is_dir() {
                    found.push(e.file_name().to_string_lossy().to_string());
                }
            }
        }
        found.sort();
        if let Some(r) = found.last() {
            return Ok(r.clone());
        }
    }
    Err(FabrError::NotFound(format!("No runs found for task {task_id} in {}", space_dir.display())))
}

pub fn cmd_lantern(task_id: String, run_id: Option<String>, folder: Option<PathBuf>, json: bool) -> Result<(), FabrError> {
    let space_dir = resolve_space_dir(folder);
    let target_run_id = resolve_run_id(&space_dir, &task_id, run_id)?;
    let run_log = fabr_agent::lantern::RunLog::new(&space_dir, &task_id, &target_run_id)?;
    let events = run_log.read_all()?;
    let graph = fabr_agent::lantern::LanternGraph::from_events(&events);

    if json {
        println!("{}", serde_json::to_string_pretty(&graph).map_err(|e| FabrError::BadRequest(e.to_string()))?);
    } else {
        println!("Task: {task_id} | Run: {target_run_id}\n");
        println!("{}", graph.render_ascii());
    }
    Ok(())
}

pub fn cmd_steer(task_id: String, action: SteerCmd, folder: Option<PathBuf>, run_id: Option<String>) -> Result<(), FabrError> {
    let space_dir = resolve_space_dir(folder);
    let target_run_id = resolve_run_id(&space_dir, &task_id, run_id)?;
    let run_log = fabr_agent::lantern::RunLog::new(&space_dir, &task_id, &target_run_id)?;
    let events = run_log.read_all()?;
    let mut graph = fabr_agent::lantern::LanternGraph::from_events(&events);
    let effects_log = fabr_agent::lantern::EffectsLog::new(run_log.dir());

    let mut state = if let Ok(Some(cp)) = fabr_agent::lantern::Checkpoint::load_latest(run_log.dir()) {
        cp.exploration_state
    } else {
        fabr_agent::lantern::ExplorationState::new(fabr_agent::lantern::ExplorationBudget::default())
    };

    let steering_action = match action {
        SteerCmd::ExploreFurther { question_id } => fabr_agent::lantern::SteeringAction::ExploreFurther { question_id },
        SteerCmd::RejectAssumption { assumption_id } => fabr_agent::lantern::SteeringAction::RejectAssumption { assumption_id },
        SteerCmd::AddConstraint { text } => fabr_agent::lantern::SteeringAction::AddConstraint { text },
        SteerCmd::PruneBranch { branch_id, reason } => fabr_agent::lantern::SteeringAction::PruneBranch { branch_id, reason },
        SteerCmd::Authorize { gate_id, reason } => fabr_agent::lantern::SteeringAction::AuthorizeDespiteGate { gate_id, reason },
        SteerCmd::Redirect { instructions } => fabr_agent::lantern::SteeringAction::Redirect { instructions },
        SteerCmd::Rewind { checkpoint_id } => fabr_agent::lantern::SteeringAction::RewindTo { checkpoint_id },
    };

    let report = fabr_agent::lantern::apply_steering(&mut state, &mut graph, &run_log, &effects_log, &task_id, steering_action, "human")?;

    println!("=== Steering Applied ===");
    println!("Action: {}", report.action_type);
    println!("Details: {}", report.details);
    if !report.affected_nodes.is_empty() {
        println!("Affected nodes: {}", report.affected_nodes.join(", "));
    }
    for w in &report.warnings {
        println!("WARNING: {w}");
    }

    Ok(())
}
