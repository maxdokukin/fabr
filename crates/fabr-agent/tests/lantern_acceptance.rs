use fabr_agent::lantern::{
    apply_steering, evaluate_convergence, Checkpoint, ConvergenceOutcome, EffectsLog, EvidenceSource, ExplorationBudget,
    ExplorationState, HypothesisStatus, LanternGraph, LanternNode, NodeKind, NodeStatus, QuestionStatus, RunLog, SteeringAction,
};
use fabr_agent::lifecycle::advance;
use fabr_agent::link::open_reminder;
use fabr_agent::model::{DraftAssertion, PlanDraft, ScriptedModel};
use fabr_core::taskrec;
use tempfile::tempdir;

#[test]
fn test_lantern_end_to_end_acceptance_scenario() {
    // -------------------------------------------------------------------------
    // Acceptance Test Scenario (per PLAN.md):
    // "Give Fabr an ambiguous task with multiple plausible approaches. Watch it
    // autonomously create investigation branches, collect contradictory evidence,
    // eliminate alternatives, and arrive at a convergence checkpoint. Inspect
    // why it chose to stop. Reject one assumption, watch the affected decisions
    // become invalid, and let Fabr conduct additional investigation before
    // producing a revised plan."
    // -------------------------------------------------------------------------

    let temp_space = tempdir().expect("create temp dir for fabric space");
    let task_id = "task-migrate-auth";
    let run_id = "run-20261027-test";

    let run_log = RunLog::new(temp_space.path(), task_id, run_id).expect("initialize run log");
    let effects_log = EffectsLog::new(run_log.dir());

    // Step 1: Initialize exploration state with bounded budget
    let mut state = ExplorationState::new(ExplorationBudget {
        max_tool_calls: 20,
        max_tokens: 50_000,
        max_branches: 5,
        max_depth: 2,
        used_tool_calls: 0,
        used_tokens: 0,
    });

    // Step 2: Open questions on ambiguous task: "Migrate API Authentication"
    let q1 = state.open_question("What authentication mechanisms currently exist?", None);
    let q2 = state.open_question("Which modern auth approach best satisfies offline requirements?", None);

    // Step 3: Open investigation branches (Exploration expansion / widening)
    let b1 = state.open_branch(&q1, "Inspect routes, middleware, and config", 0);
    let b2 = state.open_branch(&q2, "Evaluate Token-based vs OAuth2 protocols", 0);

    // Step 4: Collect evidence from codebase inspection
    let ev1 = state.add_evidence(
        &b1,
        EvidenceSource::File { path: "src/server/session.rs".into() },
        "Found legacy stateful cookie sessions in server/session.rs",
        "pub struct Session { pub id: String, pub user_id: u64 }",
    );
    let ev2 = state.add_evidence(
        &b1,
        EvidenceSource::File { path: "src/middleware/jwt.rs".into() },
        "Found partial JWT bearer implementation in middleware/jwt.rs",
        "pub fn verify_jwt(token: &str) -> Result<Claims, AuthError>",
    );
    let ev3 = state.add_evidence(
        &b2,
        EvidenceSource::File { path: "config/deployment.toml".into() },
        "Found strict offline / air-gapped requirement",
        "air_gapped = true\nexternal_network = false",
    );

    // Step 5: Form candidate hypotheses
    let h_session = state.add_hypothesis(&q1, "Extend session-based auth", vec![ev1.clone()]);
    let h_jwt = state.add_hypothesis(&q2, "Full token-based JWT migration", vec![ev2.clone()]);
    let h_oauth2 = state.add_hypothesis(&q2, "OAuth2 with external IdP", vec![]);

    // Propose resolving Q1:
    state.propose_decision(
        &q1,
        &h_session,
        "Deprecate session auth in favor of stateless tokens",
        vec![ev1.clone()],
        vec![],
        "Stateful sessions cannot scale in cluster deployment",
    );

    // Step 6: Contradiction / elimination:
    // Evidence ev3 (air_gapped) contradicts external OAuth2 IdP requirement!
    state.eliminate_hypothesis(&h_oauth2, "Air-gapped deployment forbids external OAuth2 identity provider calls", vec![ev3.clone()]);

    // Verify hypothesis elimination status
    let oauth2_hyp = state.hypotheses.iter().find(|h| h.id == h_oauth2).unwrap();
    assert!(matches!(oauth2_hyp.status, HypothesisStatus::Eliminated { .. }));

    // Step 7: Declare assumption needed for JWT transition
    let asm1 = state.declare_assumption(
        "Client devices have synchronized clocks within 5 minutes for JWT verification",
        "NTP synchronization assumption",
        "Clock drift causes false authentication failures on legacy devices",
    );

    // Step 8: Propose decision based on evidence and surviving candidates
    let dec1 = state.propose_decision(
        &q2,
        &h_jwt,
        "Migrate to full JWT token auth",
        vec![ev2.clone(), ev3.clone()],
        vec![asm1.clone()],
        "JWT is partially implemented locally and complies with air-gap constraints. OAuth2 eliminated.",
    );

    // Step 9: Convergence Engine evaluates sufficiency
    let verdict1 = evaluate_convergence(&state);

    // Inspect stopping rationale and rejected alternatives
    assert_eq!(verdict1.alternatives.len(), 1);
    assert_eq!(verdict1.alternatives[0].hypothesis_id, h_oauth2);
    assert!(verdict1.alternatives[0].rejection_reason.contains("Air-gapped"));
    assert!(!verdict1.stopping_rationale.is_empty());
    assert_eq!(verdict1.residual_assumptions.len(), 1);
    assert_eq!(verdict1.residual_assumptions[0].assumption_id, asm1);

    // Verify initial graph provenance
    let mut initial_graph = LanternGraph::new();
    initial_graph.edges.push(fabr_agent::lantern::LanternEdge {
        from: asm1.clone(),
        to: dec1.clone(),
        kind: fabr_agent::lantern::EdgeKind::CausalParent,
    });
    initial_graph.nodes.insert(
        dec1.clone(),
        LanternNode {
            id: dec1.clone(),
            kind: NodeKind::Decision,
            status: NodeStatus::Resolved,
            depth: 0,
            timestamp: "2026-10-27T18:00:00Z".into(),
            label: "Migrate to full JWT token auth".into(),
            data: serde_json::json!({ "decision_id": dec1 }),
        },
    );

    // Step 10: Human Steering Intervenes:
    // User inspects the stopping rationale, notices assumption Asm1, and REJECTS it.
    let steer_report = apply_steering(
        &mut state,
        &mut initial_graph,
        &run_log,
        &effects_log,
        task_id,
        SteeringAction::RejectAssumption { assumption_id: asm1.clone() },
        "human_operator",
    )
    .expect("apply reject_assumption steering action");

    // Step 11: Verify causal invalidation:
    // - Dependent decision `dec1` must become stale!
    // - Affected question `q2` must be reopened to `Open`!
    assert!(steer_report.affected_nodes.contains(&asm1));
    assert!(steer_report.affected_nodes.contains(&dec1));
    let target_dec = state.decisions.iter().find(|d| d.id == dec1).expect("decision exists");
    assert!(target_dec.is_stale, "Decision dependent on rejected assumption must be stale");
    assert_eq!(
        state.questions.iter().find(|q| q.id == q2).unwrap().status,
        QuestionStatus::Open,
        "Question must reopen for further investigation"
    );

    // Step 12: Fabr conducts additional investigation to resolve the issue
    let b3 = state.open_branch(&q2, "Investigate clock-drift-tolerant authentication", 1);
    let ev4 = state.add_evidence(
        &b3,
        EvidenceSource::File { path: "src/server/time.rs".into() },
        "Found server-signed nonce with monotonic sequence counter",
        "pub fn verify_server_nonce(nonce: &ServerNonce) -> bool",
    );

    let h_nonce_jwt = state.add_hypothesis(&q2, "Server-timestamped JWT with nonce window (clock-drift tolerant)", vec![ev4.clone()]);

    // Declare no further unverified assumptions
    let dec2 = state.propose_decision(
        &q2,
        &h_nonce_jwt,
        "Migrate to Server-Nonce Timestamped JWT",
        vec![ev2.clone(), ev3.clone(), ev4.clone()],
        vec![], // No residual assumptions!
        "Eliminated clock drift vulnerability using server nonce window. 100% offline compliant.",
    );

    // Step 13: Convergence Engine re-evaluates
    let verdict2 = evaluate_convergence(&state);
    assert_eq!(verdict2.outcome, ConvergenceOutcome::Converged);
    assert!(state.decisions.iter().any(|d| d.id == dec2 && !d.is_stale));

    // Step 14: Reconstruct full Lantern graph from persistent run log
    let recorded_events = run_log.read_all().expect("read recorded events");
    let final_graph = LanternGraph::from_events(&recorded_events);

    // Step 15: Verify the Lantern DAG & timeline width
    assert!(!final_graph.nodes.is_empty(), "Graph must contain recorded nodes");
    assert!(!final_graph.timeline.is_empty(), "Timeline must capture event sequence");

    // Verify ASCII viewer renders cleanly
    let ascii_chart = final_graph.render_ascii();
    assert!(ascii_chart.contains("=== Fabr Lantern Decision Graph ==="));
    assert!(ascii_chart.contains("--- Lantern Shape Timeline ---"));
}

#[test]
fn test_lantern_lifecycle_end_to_end_integration() {
    let tmp = tempdir().unwrap();
    let node_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&node_dir).unwrap();

    let fabric_space = tmp.path().join(".fabric-space");
    std::fs::create_dir_all(&fabric_space).unwrap();
    std::fs::write(
        fabric_space.join("settings.toml"),
        format!(
            "default_folder = \"{}\"\nmodel = \"qwen2.5\"\nheartbeat_interval_s = 10\n\n[setup]\ncompleted_steps = [\"folder\", \"model\"]\ncompleted = true\n",
            node_dir.display()
        ),
    )
    .unwrap();

    // 1. Open a reminder
    let path = open_reminder(&node_dir, "Migrate Database Schema").unwrap();

    // 2. Delegate the task
    let mut rec = taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    rec.requests.delegate = true;
    let expected = rec.updated_at.clone();
    taskrec::write_record(&path, &rec, Some(&expected)).unwrap();

    // 3. Setup scripted model with a plan draft
    let plan = PlanDraft {
        spec: "Run schema migration script".into(),
        success: "migration passes".into(),
        plan: "1. run migration".into(),
        assertions: vec![DraftAssertion { id: "a1".into(), text: "schema is updated".into() }],
        reversibility: "reversible".into(),
        tools: vec!["write_file".into()],
        checks: vec![],
    };
    let model = ScriptedModel::new(plan, vec![]);

    // 4. Delegate reminder to task
    advance(&node_dir, &path, &fabric_space, 10, &model).unwrap();

    // 5. Run Gather stage
    advance(&node_dir, &path, &fabric_space, 10, &model).unwrap();

    // Verify Gather created RunLog and events on disk
    let task_id = &rec.id;
    let run_id = format!("run-{}", task_id);
    let run_log = RunLog::new(&fabric_space, task_id, &run_id).unwrap();
    let events_after_gather = run_log.read_all().unwrap();
    assert!(!events_after_gather.is_empty(), "Gather stage must write Lantern events");

    // 6. Run Define stage
    advance(&node_dir, &path, &fabric_space, 10, &model).unwrap();
    let events_after_define = run_log.read_all().unwrap();
    assert!(events_after_define.len() > events_after_gather.len(), "Define stage must append Lantern events");

    // 7. Run Plan stage
    advance(&node_dir, &path, &fabric_space, 10, &model).unwrap();
    let events_after_plan = run_log.read_all().unwrap();
    assert!(events_after_plan.len() > events_after_define.len(), "Plan stage must append Lantern events");

    // Verify Checkpoint was saved on disk
    let latest_cp = Checkpoint::load_latest(run_log.dir()).unwrap().expect("checkpoint must exist after plan");
    assert_eq!(latest_cp.checkpoint_id, "cp-planned");

    // 7. Verify LanternGraph reconstruction from events written by real lifecycle
    let graph = LanternGraph::from_events(&events_after_plan);
    assert!(!graph.nodes.is_empty(), "Reconstructed graph must contain nodes");
    assert!(!graph.timeline.is_empty(), "Timeline must capture lifecycle progress");

    // 8. Apply Human Steering to the real task run
    let effects_log = EffectsLog::new(run_log.dir());
    let mut state = latest_cp.exploration_state;
    let mut steer_graph = graph.clone();
    let report = apply_steering(
        &mut state,
        &mut steer_graph,
        &run_log,
        &effects_log,
        task_id,
        SteeringAction::ExploreFurther { question_id: "q1".into() },
        "human_operator",
    )
    .expect("apply steering to real lifecycle run");

    assert!(report.affected_nodes.contains(&"q1".to_string()));
    assert_eq!(state.questions.iter().find(|q| q.id == "q1").unwrap().status, QuestionStatus::Open);

    // 9. Re-verify event log has steering record
    let final_events = run_log.read_all().unwrap();
    assert!(final_events.iter().any(|e| matches!(e, fabr_agent::lantern::LanternEvent::SteeringApplied { .. })));
}
