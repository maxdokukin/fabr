//! Task 1.1 / 2.2 — Backend bug-condition test for **Bug A**
//! (agent-workspace-scope-and-notice-routing), FLIPPED to the fixed behavior.
//!
//! **Feature: agent-workspace-scope-and-notice-routing, Property 1: out-of-root
//! records are excluded from driving.**
//!
//! This file began (task 1.1) as the exploratory reproduction: it confirmed
//! that on the UNFIXED code `warn_out_of_root` only *logged* when a discovered
//! record's canonical path escaped the canonical search root while
//! `Runner::records` still returned it and `tick`/`process` still drove it
//! (detection-only containment). The task-2.1 fix factored a shared [`in_root`]
//! predicate so `records` now EXCLUDES records whose canonical path escapes the
//! canonical `self.root`, and `warn_out_of_root` warns on exactly those
//! excluded records.
//!
//! `Runner::records` is private, so the exclusion is observed *behaviorally*:
//! the out-of-root record stays in `Reminder`, its delegate request is left
//! uncollected, and the model is not called for it — while the in-root control
//! record is still driven (regression 3.1). This is the inverse of the original
//! exploratory assertion; the old "the escape is driven" expectation is gone.
//!
//! EXPECTED OUTCOME on fixed code: the out-of-root record is NOT driven (bug
//! fixed) and the in-root record still is.
//!
//! Counterexample this guards against (the original Bug A): a task-record file
//! that lives (lexically) in an in-root `.../.fabric-space/tasks/` dir but is a
//! *symlink* whose target is a real record under a sibling directory OUTSIDE
//! the canonical search root (`<root>/inside/.fabric-space/tasks/<id>.md ->
//! <outside>/<id>.md`). `fabric::list` follows the file symlink and lists it;
//! its canonical path escapes the canonical root, so the worker now excludes it.
//!
//! _Requirements: 1.2_ · _Validates: Property 1_

use fabr_agent::runner::{Options, Runner, Sink};
use fabr_core::Workspace;
use fabr_core::fabric::{self, FabricCreateReq};
use fabr_core::setup::Ctx;
use fabr_core::taskrec::{self, State, TaskRecord};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const KEY: &str = "sk-ant-api03-FAKEFAKEFAKEFAKEFAKE-x9Qa";

/// Minimal fake Anthropic server. Every define call returns no question and a
/// spec/success/assertions payload, so a delegated record advances out of
/// `Reminder` on a single tick. Counts calls so a driven record is observable.
/// (Mirrors the self-contained `Fake` in `tests/search_root.rs`.)
struct Fake {
    url: String,
    stop: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    fn new() -> Fake {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let (st, cl) = (stop.clone(), calls.clone());
        let handle = std::thread::spawn(move || {
            while !st.load(Ordering::SeqCst) {
                let Ok(Some(mut req)) = server.recv_timeout(std::time::Duration::from_millis(50)) else { continue };
                let mut body = String::new();
                std::io::Read::read_to_string(req.as_reader(), &mut body).unwrap();
                cl.fetch_add(1, Ordering::SeqCst);
                let inner = if body.contains("keys spec, success, assertions, question") {
                    json!({"spec": "write greet.txt", "success": "greet.txt exists", "assertions": [{"id": "a1", "text": "greet.txt exists"}], "question": null})
                } else {
                    json!({"spec": "write greet.txt", "success": "greet.txt exists", "plan": "1. write it", "assertions": [{"id": "a1", "text": "greet.txt exists"}],
                        "plan_meta": {"reversibility": "reversible", "tools": ["write_file"], "checks": [{"cmd": "test", "args": ["-f", "greet.txt"]}]}})
                };
                let text =
                    json!({"content": [{"type": "text", "text": inner.to_string()}], "usage": {"input_tokens": 10, "output_tokens": 5}})
                        .to_string();
                let resp = tiny_http::Response::from_string(text)
                    .with_status_code(200u16)
                    .with_header(tiny_http::Header::from_bytes("content-type", "application/json").unwrap());
                let _ = req.respond(resp);
            }
        });
        Fake { url, stop, calls, handle: Some(handle) }
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn load(p: &Path) -> TaskRecord {
    taskrec::parse(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn edit(p: &Path, f: impl FnOnce(&mut TaskRecord)) {
    let mut rec = load(p);
    let expected = rec.updated_at.clone();
    f(&mut rec);
    taskrec::write_record(p, &rec, Some(&expected)).unwrap();
}

/// Environment with an in-root node whose `tasks/` dir holds a record file that
/// is a *symlink* escaping to a sibling directory outside the canonical root.
///
/// Layout under the tempdir:
/// ```text
/// home/
///   .fabric-space/settings.toml   (default_folder = <home>/search-root)
///   search-root/                  (the broad root the worker scans)
///     inside/.fabric-space/tasks/ (an in-root managed node; holds the escape symlink)
///   outside/.fabric-space/tasks/  (sibling dir; the real record file lives here)
/// ```
/// The in-root `tasks/<id>.md` is a symlink to `<home>/outside/.fabric-space/
/// tasks/<id>.md`. `fabric::list` follows the file symlink (via `std::fs::
/// metadata`) and lists the in-root path, but `path.canonicalize()` resolves to
/// the outside file — a genuine escape of the canonical search root.
struct Env {
    _tmp: tempfile::TempDir,
    ctx: Ctx,
    root: PathBuf,
    inside: PathBuf,
    outside: PathBuf,
}

fn escape_env(url: &str) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = home.join("search-root");
    let inside = root.join("inside");
    let outside = home.join("outside");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    let space = home.join(".fabric-space");
    std::fs::create_dir_all(&space).unwrap();
    std::fs::write(
        space.join("settings.toml"),
        format!(
            "default_folder = \"{}\"\nmodel = \"claude-sonnet-5-5\"\nheartbeat_interval_s = 10\n\n[setup]\ncompleted_steps = [\"folder\", \"api_key\", \"model\"]\ncompleted = true\n",
            root.display()
        ),
    )
    .unwrap();
    std::fs::write(space.join("secrets.toml"), format!("anthropic_api_key = \"{KEY}\"\n")).unwrap();

    let mut ctx = Ctx::new(&home);
    ctx.anthropic_url = url.to_string();
    Env { _tmp: tmp, ctx, root, inside, outside }
}

fn workspace(e: &Env) -> Workspace {
    let cfg = e._tmp.path().join("cfg");
    let cache = e._tmp.path().join("cache");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    Workspace::from_config(&cfg, &cache, Some(e.root.clone())).unwrap()
}

/// Create a delegated record under `folder`'s `.fabric-space/tasks/`.
fn delegated_record(e: &Env, folder: &Path, title: &str) -> PathBuf {
    let req = FabricCreateReq {
        folder: folder.to_string_lossy().into_owned(),
        title: title.into(),
        due: Some("2026-11-01".into()),
        pri: None,
        context: Some("some context".into()),
        spec: None,
        success: None,
    };
    let path = PathBuf::from(fabric::create_in(&req, &e.ctx.home).unwrap().path);
    edit(&path, |r| r.requests.delegate = true);
    path
}

fn symlink_file(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(target, link).unwrap();
}

/// FIXED `Runner::records`/`process` EXCLUDES an out-of-root record reached via
/// a symlink escape. The in-root control record is still driven (regression
/// 3.1), while the out-of-root record stays in `Reminder` with its delegate
/// request uncollected — `records` drops it via the shared `in_root` predicate
/// that `warn_out_of_root` also reports on. This assertion encodes the fixed
/// behavior and MUST pass on fixed code (it is the inverse of the original
/// exploratory assertion).
///
/// _Requirements: 1.2_ · _Validates: Property 1_
#[test]
fn out_of_root_record_is_excluded_from_driving() {
    let fake = Fake::new();
    let e = escape_env(&fake.url);

    // Control: an in-root record the worker is expected to drive.
    let inside_rec = delegated_record(&e, &e.inside, "inside task");

    // The escape: create a REAL delegated record under the sibling `outside`
    // dir, then symlink it into the in-root node's `tasks/` dir. `fabric::list`
    // follows the file symlink and lists the IN-ROOT path, but that path
    // canonicalizes to the outside file — a genuine search-root escape.
    let real_outside = delegated_record(&e, &e.outside, "outside task");
    let in_root_tasks = e.inside.join(".fabric-space").join("tasks");
    std::fs::create_dir_all(&in_root_tasks).unwrap();
    let outside_rec = in_root_tasks.join(real_outside.file_name().unwrap());
    symlink_file(&real_outside, &outside_rec);

    // Document the counterexample path: the discovered (listed) path lies
    // lexically under the in-root node, yet its canonical form is under the
    // sibling `outside` dir — NOT under the canonical search root.
    let discovered_canon = outside_rec.canonicalize().unwrap();
    let root_canon = e.root.canonicalize().unwrap();
    let outside_canon = e.outside.canonicalize().unwrap();
    assert!(outside_rec.starts_with(&e.root), "the listed path must be lexically inside the search root: {}", outside_rec.display());
    assert!(
        !discovered_canon.starts_with(&root_canon),
        "counterexample must escape the canonical root: {} is under {}",
        discovered_canon.display(),
        root_canon.display()
    );
    assert!(
        discovered_canon.starts_with(&outside_canon),
        "counterexample must resolve under the sibling outside dir: {}",
        discovered_canon.display()
    );
    eprintln!(
        "COUNTEREXAMPLE (Bug A): out-of-root record listed at {} canonicalizes to {} \
         which is OUTSIDE the canonical search root {}",
        outside_rec.display(),
        discovered_canon.display(),
        root_canon.display()
    );

    // Sanity: the worker's discovery (`fabric::list` over the search root) lists
    // the in-root symlink path, so `records()`/`process` will drive it.
    let listed = fabric::list(&fabric::FabricListReq { root: e.root.to_string_lossy().into_owned(), home: None })
        .unwrap()
        .records
        .into_iter()
        .map(|f| f.path)
        .collect::<Vec<_>>();
    eprintln!("LISTED RECORDS: {listed:#?}");
    assert!(
        listed.iter().any(|p| Path::new(p) == outside_rec),
        "fabric::list must discover the in-root symlink path {} (listed: {listed:?})",
        outside_rec.display()
    );

    // Drive one tick.
    let (sink, _lines) = Sink::capture();
    let mut r = Runner::new(&e.ctx, workspace(&e), Options { interval_s: 5, once: false }, sink).unwrap();
    r.start().unwrap();
    r.tick();
    r.shutdown(true);

    // Regression 3.1: the in-root control record is STILL driven past Reminder.
    let inside = load(&inside_rec);
    assert_ne!(inside.state, State::Reminder, "control in-root record should still be driven: {inside:?}");

    // FIXED: the OUT-OF-ROOT record was NOT driven. The worker listed the
    // in-root symlink path (`outside_rec`) but `records` excluded it because it
    // canonicalizes OUTSIDE the search root, so `process` never touched it: its
    // delegate request is still uncollected and it stays in `Reminder`. The
    // symlink is left in place (never replaced by an atomic write), so reading
    // through it still observes the untouched real record under `outside`. This
    // assertion is the inverse of the original exploratory one and MUST pass on
    // fixed code.
    let outside = load(&outside_rec);
    assert!(
        outside.requests.delegate,
        "FIXED: the out-of-root record's delegate request must stay uncollected (it was not driven): {outside:?}"
    );
    assert_eq!(
        outside.state,
        State::Reminder,
        "FIXED: the out-of-root record must stay in Reminder — it was excluded from driving for escaping the search root: {outside:?}"
    );
    // The real record file under `outside` was never rewritten: the in-root
    // symlink still resolves to it unchanged.
    let outside_direct = load(&real_outside);
    assert!(outside_direct.requests.delegate, "FIXED: the real out-of-root record must be untouched: {outside_direct:?}");
    assert_eq!(outside_direct.state, State::Reminder, "FIXED: the real out-of-root record must stay in Reminder: {outside_direct:?}");
    // The model was still driven for the in-root control record (a single record
    // advancing Reminder -> Define -> Plan makes more than one call), so a bare
    // "no calls" could not distinguish exclusion from a dead worker. The
    // authoritative exclusion signal is the untouched out-of-root record above;
    // here we only confirm the worker actually ran (drove the in-root record).
    assert!(fake.calls.load(Ordering::SeqCst) >= 1, "FIXED: the worker must still drive the in-root control record (>= 1 model call)");
}
