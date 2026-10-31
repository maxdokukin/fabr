use crate::meta::MetaValue;
use crate::node::{META_FILE, NODE_DIR};
use crate::taskrec::{self, Grants, Links, Requests, SCHEMA, State, TaskRecord};
use std::io;
use std::path::Path;

#[derive(Clone, Copy)]
enum Status {
    Todo,
    Wip,
    Done,
}

type Todo = (Status, Option<i64>, String, Option<u8>);

struct Spec {
    rel: &'static str,
    meta: Option<Vec<(&'static str, MetaValue)>>,
    todos: Option<Vec<Todo>>,
    files: &'static [&'static str],
}

fn s(v: &str) -> MetaValue {
    MetaValue::Str(v.into())
}

fn t(status: Status, days: Option<i64>, title: &str) -> Todo {
    (status, days, title.to_string(), None)
}

fn tp(status: Status, days: Option<i64>, title: &str, pri: u8) -> Todo {
    (status, days, title.to_string(), Some(pri))
}

fn tree() -> Vec<Spec> {
    use MetaValue::{Bool, Int};
    use Status::{Done, Todo, Wip};
    let kind = |name: &'static str, k: &'static str| Some(vec![("name", s(name)), ("kind", s(k))]);
    vec![
        Spec {
            rel: "",
            meta: Some(vec![
                ("name", s("fabr")),
                ("kind", s("root")),
                ("description", s("Personal workspace root")),
                ("show_depth", Int(1)),
            ]),
            todos: Some(vec![
                t(Wip, Some(3), "Define fabr spec v0"),
                t(Todo, Some(14), "Quarterly review of all nodes"),
                t(Done, Some(-5), "Pick a name"),
            ]),
            files: &["README.md"],
        },
        Spec {
            rel: "work",
            meta: kind("work", "area"),
            todos: Some(vec![t(Todo, Some(1), "Submit timesheet"), t(Todo, Some(-2), "Reply to vendor contract email")]),
            files: &[],
        },
        Spec {
            rel: "work/api-server",
            meta: Some(vec![("name", s("api-server")), ("kind", s("project")), ("color", s("teal"))]),
            todos: Some(vec![
                tp(Wip, Some(2), "Migrate auth to OAuth2", 2),
                tp(Todo, Some(10), "Add rate limiting", 3),
                t(Todo, None, "Investigate flaky integration test"),
                t(Done, Some(-12), "Set up CI pipeline"),
                t(Done, Some(-1), "Fix N+1 query on /users"),
            ]),
            files: &["main.py", "pyproject.toml"],
        },
        Spec {
            rel: "work/api-server/docs",
            meta: kind("docs", "folder"),
            todos: Some(vec![t(Todo, Some(7), "Write OpenAPI descriptions"), t(Todo, Some(7), "Add auth flow diagram")]),
            files: &["index.md"],
        },
        Spec { rel: "work/api-server/tests", meta: kind("tests", "folder"), todos: Some(vec![]), files: &["test_auth.py"] },
        Spec {
            rel: "work/frontend",
            meta: Some(vec![("name", s("frontend")), ("kind", s("project")), ("show_depth", Int(1))]),
            todos: Some(vec![
                t(Wip, Some(0), "Ship dark mode"),
                t(Todo, Some(21), "Upgrade to React 20"),
                t(Done, Some(-30), "Design system tokens"),
            ]),
            files: &["package.json"],
        },
        Spec { rel: "work/frontend/node_modules", meta: None, todos: None, files: &["junk.js"] },
        Spec { rel: "work/meetings", meta: kind("meetings", "folder"), todos: None, files: &["2026-10-01.md", "2026-10-08.md"] },
        Spec { rel: "personal", meta: kind("personal", "area"), todos: Some(vec![t(Todo, Some(30), "Renew passport")]), files: &[] },
        Spec {
            rel: "personal/health",
            meta: kind("health", "folder"),
            todos: Some(vec![t(Todo, Some(5), "Book dentist"), t(Wip, None, "Run 3x per week"), t(Done, Some(-3), "Blood test")]),
            files: &[],
        },
        Spec {
            rel: "personal/finance",
            meta: Some(vec![("name", s("finance")), ("kind", s("folder")), ("show_depth", Int(1))]),
            todos: Some(vec![
                tp(Todo, Some(-10), "File tax return", 1),
                t(Todo, Some(60), "Rebalance portfolio"),
                t(Done, Some(-40), "Close old bank account"),
            ]),
            files: &["budget.ods"],
        },
        Spec { rel: "personal/finance/receipts", meta: None, todos: None, files: &["r1.pdf", "r2.pdf"] },
        Spec {
            rel: "personal/finance/receipts/2026",
            meta: kind("2026", "folder"),
            todos: Some(vec![t(Todo, Some(75), "Scan Q4 receipts")]),
            files: &[],
        },
        Spec {
            rel: "research",
            meta: Some(vec![("name", s("research")), ("kind", s("area")), ("color", s("pink"))]),
            todos: Some(vec![t(Wip, Some(9), "Lit review: tree layout algorithms")]),
            files: &[],
        },
        Spec {
            rel: "research/paper-a",
            meta: kind("paper-a", "project"),
            todos: Some(vec![
                t(Todo, Some(4), "Address reviewer 2"),
                t(Wip, Some(4), "Rerun ablations"),
                t(Todo, Some(4), "Update related work"),
                t(Done, Some(-20), "Submit draft"),
            ]),
            files: &["paper.tex"],
        },
        Spec {
            rel: "research/paper-a/figures",
            meta: kind("figures", "folder"),
            todos: Some(vec![t(Todo, Some(3), "Regenerate fig 3 at 300dpi")]),
            files: &["fig1.svg"],
        },
        Spec {
            rel: "research/paper-a/figures/raw",
            meta: kind("raw", "folder"),
            todos: Some(vec![t(Done, Some(-25), "Export raw data")]),
            files: &[],
        },
        Spec {
            rel: "research/ideas",
            meta: kind("ideas", "folder"),
            todos: Some((1..13).map(|i| t(Todo, None, &format!("Idea #{i}: something worth exploring"))).collect()),
            files: &[],
        },
        Spec {
            rel: "archive",
            meta: Some(vec![("name", s("archive")), ("kind", s("area")), ("hidden", Bool(true))]),
            todos: Some(vec![t(Done, Some(-200), "Old project wrap-up"), t(Done, Some(-180), "Hand off repo")]),
            files: &[],
        },
    ]
}

fn meta_text(meta: &[(&str, MetaValue)]) -> String {
    let lines: Vec<String> = meta
        .iter()
        .map(|(k, v)| {
            let v = match v {
                MetaValue::Bool(b) => b.to_string(),
                MetaValue::Int(i) => i.to_string(),
                MetaValue::Float(f) => f.to_string(),
                MetaValue::Str(s) => format!("\"{s}\""),
            };
            format!("{k} = {v}")
        })
        .collect();
    lines.join("\n") + "\n"
}

fn write_records(node: &Path, todos: &[Todo], today: chrono::NaiveDate) -> io::Result<()> {
    if todos.is_empty() {
        return Ok(());
    }
    let dir = node.join(".fabric-space/tasks");
    std::fs::create_dir_all(&dir)?;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    for (status, days, title, pri) in todos {
        let id = format!("{}-{}", today.format("%Y-%m-%d"), crate::fabric::slugify(title));
        let rec = TaskRecord {
            schema: SCHEMA,
            id: id.clone(),
            source: None,
            title: title.clone(),
            due: days.map(|d| (today + chrono::Duration::days(d)).format("%Y-%m-%d").to_string()),
            pri: pri.unwrap_or(0),
            state: match status {
                Status::Todo => State::Reminder,
                Status::Wip => State::InProgress,
                Status::Done => State::Succeeded,
            },
            step: None,
            waiting: false,
            paused: false,
            suspended: false,
            suspend_until: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            requests: Requests {
                delegate: false,
                cancel: false,
                redelegate: false,
                pause: false,
                resume: false,
                complete: false,
                redo: None,
            },
            question: None,
            answers: Vec::new(),
            grants: Grants { requested: Vec::new(), granted: Vec::new() },
            plan_meta: None,
            assertions: Vec::new(),
            approval: None,
            verdict: None,
            cost_usd: 0.0,
            links: Links { redelegated_from: None, redelegated_to: None },
            by: Default::default(),
            needs: None,
            sessions: Default::default(),
            context: String::new(),
            spec: String::new(),
            success: String::new(),
            plan: String::new(),
            log: String::new(),
        };
        std::fs::write(dir.join(format!("{id}.md")), taskrec::render(&rec))?;
    }
    Ok(())
}

pub fn write_fixture_tree(root: &Path) -> io::Result<usize> {
    let today = chrono::Local::now().date_naive();
    if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    let specs = tree();
    for spec in &specs {
        let node = if spec.rel.is_empty() { root.to_path_buf() } else { root.join(spec.rel) };
        std::fs::create_dir_all(&node)?;
        for f in spec.files {
            std::fs::write(node.join(f), "")?;
        }
        let Some(meta) = &spec.meta else { continue };
        let xs = node.join(NODE_DIR);
        std::fs::create_dir(&xs)?;
        std::fs::write(xs.join(META_FILE), meta_text(meta))?;
        if let Some(todos) = &spec.todos {
            write_records(&node, todos, today)?;
        }
    }
    Ok(specs.len())
}
