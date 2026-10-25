//! Feature: agent-workspace-scope-and-notice-routing, Property 1: Scope
//! containment — out-of-root records are dropped from driving, in-root records
//! (at any depth) are kept, and an in-scope-only tree is driven unchanged.
//!
//! Task 2.3. This is the property-based companion to the `in_root` unit tests
//! in `runner/mod.rs` (task 2.2) and the single behavioral case in
//! `tests/out_of_root_driven.rs` (task 1.1 / 2.2). It generates *random* trees
//! — in-root records at varied depths plus some symlink-escapes whose canonical
//! path lands outside the canonical search root — and asserts the containment
//! partition `Runner::records` computes.
//!
//! `Runner::records` and the shared `in_root` predicate are private, so the
//! partition is observed through the public surface, exactly as the task
//! permits:
//!
//! * `records_original` = every path `fabric::list` discovers under the search
//!   root (the set `records` filters). This is the public discovery surface.
//! * The excluded set is read from `Sink::capture()`: `Runner::start` calls
//!   `warn_out_of_root`, which emits one WARN per *excluded* record using the
//!   SAME `in_root` predicate `records` filters with (so detection and
//!   exclusion cannot drift — pinned by the task-2.2 unit test). Each WARN names
//!   the excluded listed path verbatim.
//! * `records_fixed` = `records_original` minus the excluded set — precisely the
//!   set `Runner::records` returns to `tick`/`process`.
//!
//! Property, over ≥ 100 random trees:
//!   1. `records_fixed` partitions exactly on canonical containment: it contains
//!      every discovered path that canonicalizes under the canonical root and
//!      none that escapes it (Requirement 1.2, preservation 3.5 — records at any
//!      depth are kept).
//!   2. For in-scope-only trees (no escapes) `records_fixed == records_original`
//!      and NO exclusion WARN fires (preservation 3.1 / 3.5).
//!
//! `start` alone drives the partition; no model call or `tick` is needed, so the
//! property runs without a network stand-in and stays fast across many cases.
//!
//! _Requirements: 1.2, 3.1, 3.5_ · _Validates: Property 1_

use fabr_agent::runner::{Options, Runner, Sink};
use fabr_core::Workspace;
use fabr_core::fabric::{self, FabricCreateReq, FabricListReq};
use fabr_core::setup::Ctx;
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const KEY: &str = "sk-ant-api03-FAKEFAKEFAKEFAKEFAKE-x9Qa";

/// A generated tree: some in-root records (at varied depths) and some
/// symlink-escapes that canonicalize outside the search root.
#[derive(Debug, Clone)]
struct Spec {
    /// Node paths (relative, `/`-joined, possibly empty = root node) for the
    /// in-root records. Each becomes `<root>/<node>/.fabric-space/tasks/<id>.md`.
    in_root_nodes: Vec<String>,
    /// How many out-of-root escapes to plant (a real record under a sibling
    /// `outside/` dir, symlinked into an in-root node's `tasks/`).
    escapes: usize,
}

/// A single relative-path segment: short, lowercase-ascii, filesystem-safe and
/// never a reserved name (`.`, `..`, the space dir, `tasks`).
fn segment() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9]{0,4}".prop_filter("reserved segment", |s| s != "tasks" && s != ".fabric-space" && s != "outside" && s != "inside")
}

/// A node path: 0..=3 segments joined with `/` (0 segments = the root node).
fn node_path() -> impl Strategy<Value = String> {
    prop::collection::vec(segment(), 0..=3).prop_map(|parts| parts.join("/"))
}

fn spec() -> impl Strategy<Value = Spec> {
    (prop::collection::vec(node_path(), 0..=4), 0usize..=3usize).prop_map(|(in_root_nodes, escapes)| Spec { in_root_nodes, escapes })
}

struct Env {
    _tmp: tempfile::TempDir,
    ctx: Ctx,
    root: PathBuf,
    outside: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let root = home.join("search-root");
    let outside = home.join("outside");
    std::fs::create_dir_all(&root).unwrap();
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

    let ctx = Ctx::new(&home);
    Env { _tmp: tmp, ctx, root, outside }
}

fn workspace(e: &Env) -> Workspace {
    let cfg = e._tmp.path().join("cfg");
    let cache = e._tmp.path().join("cache");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    Workspace::from_config(&cfg, &cache, Some(e.root.clone())).unwrap()
}

/// Create a record under `<folder>/.fabric-space/tasks/<id>.md` and return its
/// path. Titles are unique per call so ids never collide.
fn create_record(e: &Env, folder: &Path, title: &str) -> PathBuf {
    let req = FabricCreateReq {
        folder: folder.to_string_lossy().into_owned(),
        title: title.into(),
        due: Some("2026-11-01".into()),
        pri: None,
        context: Some("ctx".into()),
        spec: None,
        success: None,
    };
    PathBuf::from(fabric::create_in(&req, &e.ctx.home).unwrap().path)
}

fn symlink_file(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(target, link).unwrap();
}

/// Build the tree described by `spec`, returning (env, in_root_paths,
/// out_of_root_paths). `in_root_paths` are the listed paths that canonicalize
/// under the root; `out_of_root_paths` are the in-root *symlink* paths that
/// canonicalize outside it (what `fabric::list` discovers for an escape).
fn build(spec: &Spec) -> (Env, Vec<PathBuf>, Vec<PathBuf>) {
    let e = env();

    let mut in_root = Vec::new();
    for (i, node) in spec.in_root_nodes.iter().enumerate() {
        let folder = if node.is_empty() { e.root.clone() } else { e.root.join(node) };
        std::fs::create_dir_all(&folder).unwrap();
        in_root.push(create_record(&e, &folder, &format!("in root {i}")));
    }

    // Each escape is a real record under the sibling `outside/` dir, symlinked
    // into a dedicated in-root node's `tasks/` so `fabric::list` discovers the
    // in-root (lexically-contained) path while it canonicalizes outside.
    let mut out_of_root = Vec::new();
    for i in 0..spec.escapes {
        let real = create_record(&e, &e.outside, &format!("escape {i}"));
        let inside_tasks = e.root.join(format!("esc{i}")).join(".fabric-space").join("tasks");
        std::fs::create_dir_all(&inside_tasks).unwrap();
        let link = inside_tasks.join(real.file_name().unwrap());
        symlink_file(&real, &link);
        out_of_root.push(link);
    }

    (e, in_root, out_of_root)
}

/// Every record path `fabric::list` discovers under the search root. This is the
/// public discovery surface `Runner::records` filters, i.e. `records_original`.
fn listed(root: &Path) -> BTreeSet<PathBuf> {
    fabric::list(&FabricListReq { root: root.to_string_lossy().into_owned(), home: None })
        .unwrap()
        .records
        .into_iter()
        .map(|f| PathBuf::from(f.path))
        .collect()
}

/// Run `Runner::start` with a capturing sink and return the set of listed paths
/// `warn_out_of_root` reported as *excluded from driving*. The WARN names the
/// discovered (listed) path verbatim before "resolves to", so we match each
/// candidate path by that substring — the same `in_root` verdict `records`
/// filters with (pinned disjoint by the task-2.2 unit test).
fn excluded(e: &Env, candidates: &BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
    let (sink, lines) = Sink::capture();
    let mut r = Runner::new(&e.ctx, workspace(e), Options { interval_s: 5, once: false }, sink).unwrap();
    r.start().unwrap();
    r.shutdown(true);
    let warnings = lines.lock().unwrap().clone();
    let excluded_lines: Vec<&String> = warnings.iter().filter(|l| l.contains("excluded from driving")).collect();
    candidates
        .iter()
        .filter(|p| {
            let needle = format!("record {} resolves to", p.display());
            excluded_lines.iter().any(|l| l.contains(&needle))
        })
        .cloned()
        .collect()
}

/// The canonical-containment partition the property checks `records` against:
/// a discovered path is in-scope iff it canonicalizes under the canonical root
/// (any depth). Un-canonicalizable paths are kept in-scope — but the generator
/// only plants on-disk files, so every candidate canonicalizes here.
fn partition(root: &Path, listed: &BTreeSet<PathBuf>) -> (BTreeSet<PathBuf>, BTreeSet<PathBuf>) {
    let root_canon = root.canonicalize().unwrap();
    let mut keep = BTreeSet::new();
    let mut drop = BTreeSet::new();
    for p in listed {
        let in_scope = match p.canonicalize() {
            Ok(c) => c.starts_with(&root_canon),
            Err(_) => true,
        };
        if in_scope {
            keep.insert(p.clone());
        } else {
            drop.insert(p.clone());
        }
    }
    (keep, drop)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// Property 1: `records_fixed` partitions the discovered set exactly on
    /// canonical containment (in-root at any depth kept, escapes dropped), and
    /// an in-scope-only tree is driven unchanged (`records_fixed ==
    /// records_original`, no exclusion WARN).
    ///
    /// _Requirements: 1.2, 3.1, 3.5_
    #[test]
    fn records_partition_on_canonical_containment(spec in spec()) {
        let (e, in_root, out_of_root) = build(&spec);

        let records_original = listed(&e.root);

        // Sanity on the fixture: every planted in-root record is discovered, and
        // every escape is discovered as a lexically-in-root path that
        // canonicalizes outside the canonical root.
        let root_canon = e.root.canonicalize().unwrap();
        for p in &in_root {
            prop_assert!(records_original.contains(p), "in-root record must be discovered: {}", p.display());
        }
        for p in &out_of_root {
            prop_assert!(records_original.contains(p), "escape must be discovered as an in-root path: {}", p.display());
            prop_assert!(p.starts_with(&e.root), "escape must be lexically in-root: {}", p.display());
            prop_assert!(
                !p.canonicalize().unwrap().starts_with(&root_canon),
                "escape must canonicalize outside the root: {}",
                p.display()
            );
        }

        // The containment partition `records` should compute.
        let (expected_keep, expected_drop) = partition(&e.root, &records_original);

        // Observe exclusion through the shared `in_root`/`warn_out_of_root`
        // channel; `records_fixed` is what survives (what `tick`/`process`
        // drive).
        let dropped = excluded(&e, &records_original);
        let records_fixed: BTreeSet<PathBuf> = records_original.difference(&dropped).cloned().collect();

        // 1. Exact partition on canonical containment.
        prop_assert_eq!(
            &dropped, &expected_drop,
            "excluded set must be exactly the out-of-root (escaping) records"
        );
        prop_assert_eq!(
            &records_fixed, &expected_keep,
            "records_fixed must keep exactly the in-root records (any depth) and drop the escapes"
        );
        // No escape survives; no in-root record is dropped.
        for p in &out_of_root {
            prop_assert!(!records_fixed.contains(p), "an escaping record must be dropped: {}", p.display());
        }
        for p in &in_root {
            prop_assert!(records_fixed.contains(p), "an in-root record (any depth) must be kept: {}", p.display());
        }

        // 2. Preservation: an in-scope-only tree is driven unchanged.
        if out_of_root.is_empty() {
            prop_assert!(dropped.is_empty(), "no exclusion must fire for an in-scope-only tree (3.1)");
            prop_assert_eq!(
                &records_fixed, &records_original,
                "records_fixed must equal records_original for an in-scope-only tree (3.1/3.5)"
            );
        }
    }
}
