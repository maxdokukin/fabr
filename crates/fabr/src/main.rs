mod server;

use clap::{Parser, Subcommand};
use fabr_core::fabric::{FabricCreateReq, ImportTodosReq};
use fabr_core::workspace::{Workspace, today};
use fabr_core::{AppDirs, FabrError, InitChildrenReq, Item, ListDirsReq, Node, NodeRef, Phase, RemoveNodeReq, write_sample};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "fabr", version, about = "Tree of per-directory tasks")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    #[command(about = "Print the scanned tree")]
    Scan {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        all: bool,
        root: PathBuf,
    },
    #[command(about = "Serve the UI and API on loopback")]
    Serve {
        root: Option<PathBuf>,
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        no_open: bool,
        #[arg(long)]
        ui_dir: Option<PathBuf>,
    },
    #[command(about = "Write the sample tree")]
    Sample { dir: PathBuf },
    #[command(about = "Make a directory a node (`show_depth = 1`); optionally its subdirectories too")]
    Init {
        path: PathBuf,
        #[arg(long, help = "Also initialise the immediate subdirectories (not pruned, no dot-dirs)")]
        children: bool,
        #[arg(long, value_name = "N", help = "Initialise subdirectories down to N levels (implies --children; default 1 with --children)")]
        depth: Option<u32>,
    },
    #[command(about = "Remove a node: deletes its `.fabr/` only; the folder and its files stay")]
    Remove {
        path: PathBuf,
        #[arg(long, short = 'y', help = "Do not ask for confirmation")]
        yes: bool,
    },
    #[command(about = "Add a reminder to a node (a task record in state reminder)")]
    Add {
        path: PathBuf,
        title: String,
        #[arg(long, help = "YYYY-MM-DD or YYYY-MM-DDTHH:MM")]
        due: Option<String>,
        #[arg(long, help = "0-3")]
        pri: Option<u8>,
    },
    #[command(about = "Turn the lines of a node's old .fabr/.todo into reminders and rename it to .todo.imported")]
    Import { path: PathBuf },
    #[command(about = "Run one setup check and print its JSON result")]
    Check {
        #[command(subcommand)]
        what: CheckCmd,
    },
    #[command(about = "Telegram bot commands")]
    Telegram {
        #[command(subcommand)]
        what: TelegramCmd,
    },
    #[command(about = "Run the agent backend against the default folder until Ctrl-C")]
    Agent {
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..))]
        interval: u64,
        #[arg(long, help = "Run one tick and exit")]
        once: bool,
        #[arg(long, help = "Also print the log to stderr")]
        verbose: bool,
    },
    #[command(about = "List task records (state, step, what they wait for) under PATH or the default folder")]
    Status {
        #[arg(long)]
        json: bool,
        path: Option<PathBuf>,
    },
    #[command(about = "Show one task record: question, plan, assertions, verdict, cost, Claude Code sessions")]
    Show { record: String },
    #[command(about = "Ask the agent to take a reminder on")]
    Delegate { record: String },
    #[command(about = "Answer the open question of a task")]
    Answer {
        record: String,
        #[arg(required = true)]
        text: Vec<String>,
    },
    #[command(about = "Ask a task's Claude Code session a question, read-only")]
    Ask {
        record: String,
        #[arg(long, default_value = "plan", value_parser = ["plan", "execute", "evaluate"])]
        role: String,
        #[arg(required = true)]
        text: Vec<String>,
    },
    #[command(about = "Add a Claude Code session of a folder as a reminder and print the record path")]
    Track { folder: PathBuf, session_id: String },
    #[command(about = "Approve the pending plan")]
    Approve { record: String },
    #[command(about = "Refuse the pending plan")]
    Refuse {
        record: String,
        #[arg(long)]
        note: Option<String>,
    },
    #[command(about = "Cancel a running task")]
    Cancel { record: String },
    #[command(about = "Pause a task after its current step")]
    Pause { record: String },
    #[command(about = "Resume a paused task")]
    Resume { record: String },
    #[command(about = "List items")]
    Ls {
        #[arg(long, conflicts_with_all = ["overdue", "wip"])]
        all: bool,
        #[arg(long, conflicts_with = "wip")]
        overdue: bool,
        #[arg(long)]
        wip: bool,
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum CheckCmd {
    Folder { path: String },
    Claude,
    TelegramToken,
}

#[derive(Subcommand)]
enum TelegramCmd {
    #[command(about = "Wait for a message to the bot, reply connected with a Confirm button, wait for the tap")]
    Link {
        #[arg(long, default_value_t = fabr_core::setup::DEFAULT_LINK_TIMEOUT_S)]
        timeout: u64,
    },
}

fn print_check(v: serde_json::Value) -> ExitCode {
    println!("{v}");
    if v["ok"].as_bool() == Some(true) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn run_check(what: CheckCmd) -> ExitCode {
    use fabr_core::setup::{self, Ctx};
    let ctx = Ctx::from_env();
    print_check(match what {
        CheckCmd::Folder { path } => setup::check_folder(&ctx, &path),
        CheckCmd::Claude => setup::check_claude(&ctx),
        CheckCmd::TelegramToken => setup::check_telegram_token(&ctx),
    })
}

fn run_agent(interval_s: u64, once: bool, verbose: bool) -> ExitCode {
    use fabr_agent::runner::{Options, Runner, Sink, install_signals, start_gate};
    let ctx = fabr_core::setup::Ctx::from_env();
    match fabr_core::logging::init_default(once || verbose) {
        Ok(p) => log::info!("fabr {} (agent) log {}", env!("CARGO_PKG_VERSION"), p.display()),
        Err(e) => eprintln!("fabr: logging disabled: {}", e.message()),
    }
    let started = start_gate(&ctx)
        .and_then(|gate| AppDirs::resolve().workspace(Some(gate.default_folder)))
        .and_then(|ws| Runner::new(&ctx, ws, Options { interval_s, once }, Sink::default()));
    match started.and_then(|mut runner| runner.run(install_signals())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let msg = fabr_core::redact::redact(&e.message());
            log::error!("{msg}");
            eprintln!("fabr: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn workspace(root: Option<PathBuf>) -> Result<Workspace, FabrError> {
    AppDirs::resolve().workspace(root)
}

fn print_tree(n: &Node, depth: usize, out: &mut String) {
    let open = n.items.iter().filter(|i| i.phase != Phase::Done).count();
    let mut line = format!("{}{}", "  ".repeat(depth), if n.name.is_empty() { "." } else { &n.name });
    if !n.managed {
        line.push('/');
    }
    if let Some(v) = &n.via {
        line.push_str(&format!("  (via {v})"));
    }
    if !n.items.is_empty() {
        line.push_str(&format!("  [{open} open / {}]", n.items.len()));
    }
    if !n.warnings.is_empty() {
        line.push_str(&format!("  ({} warnings)", n.warnings.len()));
    }
    out.push_str(&line);
    out.push('\n');
    for c in &n.children {
        print_tree(c, depth + 1, out);
    }
}

fn cmd_scan(root: PathBuf, json: bool, all: bool) -> Result<(), FabrError> {
    let ws = workspace(Some(root))?;
    let tree = ws.get_tree(all);
    if let Some(r) = &tree.truncated_reason {
        eprintln!("fabr: truncated: {r}");
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&tree).map_err(|e| FabrError::Io(e.to_string()))?);
    } else {
        let mut s = String::new();
        for r in &tree.root.children {
            print_tree(r, 0, &mut s);
        }
        print!("{s}");
        if tree.truncated {
            println!("(truncated)");
        }
    }
    for e in &tree.errors {
        eprintln!("fabr: {}: {}", e.source, e.message);
    }
    Ok(())
}

fn cmd_serve(root: Option<PathBuf>, port: u16, no_open: bool, ui_dir: Option<PathBuf>) -> Result<(), FabrError> {
    if let Some(d) = &ui_dir
        && !server::ui_dir_ok(d)
    {
        return Err(FabrError::NotFound(format!("{} has no index.html", d.display())));
    }
    let ws = Arc::new(workspace(root)?);
    let (srv, port) = server::bind(port).map_err(|e| FabrError::Io(format!("cannot bind: {e}")))?;
    let url = format!("http://127.0.0.1:{port}/");
    println!("fabr serving {url}  (Ctrl-C to stop)");
    if !no_open {
        server::open_browser(&url);
    }
    for h in server::spawn_workers(srv, ws, Arc::new(server::Ui::new(ui_dir)), 4) {
        let _ = h.join();
    }
    Ok(())
}

fn managed(ws: &Workspace, path: &Path) -> Result<(Node, String), FabrError> {
    let node = ws.get_node(NodeRef { source: "local".into(), path: String::new() })?;
    match node.version.clone().filter(|_| node.managed) {
        Some(version) => Ok((node, version)),
        None => Err(FabrError::NotFound(format!("{} is not a fabr node (no .fabr/)", path.display()))),
    }
}

fn absolute(path: &Path) -> Result<String, FabrError> {
    let p = path.canonicalize().map_err(|e| FabrError::NotFound(format!("{}: {e}", path.display())))?;
    Ok(p.to_string_lossy().into_owned())
}

fn cmd_add(path: PathBuf, title: String, due: Option<String>, pri: Option<u8>) -> Result<(), FabrError> {
    let ws = workspace(Some(path.clone()))?;
    managed(&ws, &path)?;
    let req = FabricCreateReq { folder: absolute(&path)?, title, due, pri, context: None, spec: None, success: None };
    let made = fabr_core::fabric::create(&req)?;
    println!("{}", made.id);
    Ok(())
}

fn cmd_import(path: PathBuf) -> Result<(), FabrError> {
    let made = fabr_core::fabric::import_todos(&ImportTodosReq { path: absolute(&path)? })?;
    println!("imported {} reminders", made.created);
    Ok(())
}

fn collect_rows<'a>(n: &'a Node, out: &mut Vec<(&'a Node, &'a Item)>) {
    for i in &n.items {
        out.push((n, i));
    }
    for c in &n.children {
        collect_rows(c, out);
    }
}

fn cmd_ls(path: Option<PathBuf>, all: bool, overdue: bool, wip: bool) -> Result<(), FabrError> {
    let root = path.unwrap_or_else(|| PathBuf::from("."));
    let ws = workspace(Some(root))?;
    let tree = ws.get_tree(false);
    let today = today();
    let mut rows = Vec::new();
    collect_rows(&tree.root, &mut rows);
    rows.sort_by_key(|(_, i)| (fabr_core::due_sort_key(i.due.as_deref()), fabr_core::item::pri_rank(i.pri)));
    let now = fabr_core::now_minute();
    for (n, i) in rows {
        let keep = if all {
            true
        } else if overdue {
            i.phase != Phase::Done && i.due.as_deref().is_some_and(|d| fabr_core::is_overdue_at(d, &today, &now))
        } else if wip {
            !matches!(i.phase, Phase::Todo | Phase::Done)
        } else {
            i.phase != Phase::Done
        };
        if keep {
            let marks = "!".repeat(i.pri.min(3) as usize);
            let node = if n.path.is_empty() { "." } else { &n.path };
            println!("{:<10} {:<16} {:<3} {}  [{}]", i.phase.as_str(), i.due.as_deref().unwrap_or("-"), marks, i.title, node);
        }
    }
    Ok(())
}

fn cmd_init(path: PathBuf, children: bool, depth: Option<u32>) -> Result<(), FabrError> {
    let levels = depth.unwrap_or(if children { 1 } else { 0 });
    let ws = workspace(Some(path.clone()))?;
    let at = |p: &str| NodeRef { source: "local".into(), path: p.into() };
    let shown = |p: &str| if p.is_empty() { path.display().to_string() } else { path.join(p).display().to_string() };
    match ws.init_node(at("")) {
        Ok(_) => println!("initialised {}", shown("")),
        Err(FabrError::Conflict { .. }) if levels > 0 => {}
        Err(FabrError::Conflict { .. }) => {
            return Err(FabrError::Conflict { message: format!("{} is already a fabr node", path.display()), node: None });
        }
        Err(e) => return Err(e),
    }
    let mut level = vec![String::new()];
    for _ in 0..levels {
        let mut next = Vec::new();
        for dir in &level {
            let dirs = ws.list_dirs(ListDirsReq { source: "local".into(), path: dir.clone() })?;
            let names: Vec<String> = dirs.iter().filter(|d| !d.pruned).map(|d| d.name.clone()).collect();
            for d in dirs.iter().filter(|d| !d.pruned && !d.managed) {
                println!("initialised {}", shown(&fabr_core::node::join(dir, &d.name)));
            }
            if !names.is_empty() {
                ws.init_children(InitChildrenReq { source: "local".into(), path: dir.clone(), names: names.clone() })?;
            }
            next.extend(names.iter().map(|n| fabr_core::node::join(dir, n)));
        }
        level = next;
    }
    Ok(())
}

fn cmd_remove(path: PathBuf, yes: bool) -> Result<(), FabrError> {
    let ws = workspace(Some(path.clone()))?;
    let (_, version) = managed(&ws, &path)?;
    if !yes {
        eprint!("Remove node {}? the folder stays [y/N] ", path.display());
        let _ = std::io::Write::flush(&mut std::io::stderr());
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("not removed");
            return Ok(());
        }
    }
    ws.remove_node(RemoveNodeReq { source: "local".into(), path: String::new(), version })?;
    println!("removed node {}", path.display());
    Ok(())
}

fn cmd_status(path: Option<PathBuf>, json: bool) -> Result<(), FabrError> {
    use fabr_core::cli_tasks;
    let ctx = fabr_core::setup::Ctx::from_env();
    let root = match path {
        Some(p) => p,
        None => cli_tasks::default_root(&ctx)?,
    };
    let rows = cli_tasks::status(&root, &ctx)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&rows).map_err(|e| FabrError::Io(e.to_string()))?);
    } else {
        print!("{}", cli_tasks::status_text(&rows));
    }
    Ok(())
}

fn cmd_task(cmd: Cmd) -> Result<(), FabrError> {
    use fabr_core::cli_tasks as t;
    type Run = Box<dyn FnOnce(&Path) -> Result<String, FabrError>>;
    let ctx = fabr_core::setup::Ctx::from_env();
    let (record, run): (String, Run) = match cmd {
        Cmd::Show { record } => (record, Box::new(t::show)),
        Cmd::Delegate { record } => (record, Box::new(t::delegate)),
        Cmd::Answer { record, text } => (record, Box::new(move |p| t::answer(p, &text.join(" ")))),
        Cmd::Ask { record, role, text } => {
            let ctx = ctx.clone();
            (record, Box::new(move |p| t::ask(&ctx, p, &role, &text.join(" "))))
        }
        Cmd::Approve { record } => (record, Box::new(t::approve)),
        Cmd::Refuse { record, note } => (record, Box::new(move |p| t::refuse(p, note))),
        Cmd::Cancel { record } => (record, Box::new(|p| t::request(p, "cancel"))),
        Cmd::Pause { record } => (record, Box::new(|p| t::request(p, "pause"))),
        Cmd::Resume { record } => (record, Box::new(|p| t::request(p, "resume"))),
        _ => unreachable!(),
    };
    let out = run(&t::resolve(&ctx, &record)?)?;
    print!("{}", if out.ends_with('\n') { out } else { format!("{out}\n") });
    Ok(())
}

fn cmd_sample(dir: &Path) -> Result<(), FabrError> {
    let n = write_sample(dir).map_err(|e| FabrError::Io(format!("{}: {e}", dir.display())))?;
    println!("wrote {n} nodes to {}", dir.display());
    Ok(())
}

fn main() -> ExitCode {
    if let Some(code) = fabr_core::remote::askpass::run_helper_if_invoked() {
        return ExitCode::from(code as u8);
    }
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Check { what } => return run_check(what),
        Cmd::Agent { interval, once, verbose } => return run_agent(interval, once, verbose),
        Cmd::Telegram { what: TelegramCmd::Link { timeout } } => {
            return print_check(fabr_core::setup::telegram_link(&fabr_core::setup::Ctx::from_env(), timeout));
        }
        _ => {}
    }
    if matches!(cli.cmd, Cmd::Serve { .. } | Cmd::Scan { .. }) {
        let serve = matches!(cli.cmd, Cmd::Serve { .. });
        match fabr_core::logging::init_default(serve) {
            Ok(p) => log::info!("fabr {} ({}) log {}", env!("CARGO_PKG_VERSION"), if serve { "serve" } else { "scan" }, p.display()),
            Err(e) => eprintln!("fabr: logging disabled: {}", e.message()),
        }
    }
    let r = match cli.cmd {
        Cmd::Scan { json, all, root } => cmd_scan(root, json, all),
        Cmd::Serve { root, port, no_open, ui_dir } => cmd_serve(root, port, no_open, ui_dir),
        Cmd::Sample { dir } => cmd_sample(&dir),
        Cmd::Init { path, children, depth } => cmd_init(path, children, depth),
        Cmd::Remove { path, yes } => cmd_remove(path, yes),
        Cmd::Add { path, title, due, pri } => cmd_add(path, title, due, pri),
        Cmd::Import { path } => cmd_import(path),
        Cmd::Ls { all, overdue, wip, path } => cmd_ls(path, all, overdue, wip),
        Cmd::Status { json, path } => cmd_status(path, json),
        Cmd::Track { folder, session_id } => {
            fabr_core::ccsession::track(&fabr_core::ccsession::config_dir(), &folder, &session_id).map(|p| println!("{p}"))
        }
        c @ (Cmd::Show { .. }
        | Cmd::Delegate { .. }
        | Cmd::Answer { .. }
        | Cmd::Ask { .. }
        | Cmd::Approve { .. }
        | Cmd::Refuse { .. }
        | Cmd::Cancel { .. }
        | Cmd::Pause { .. }
        | Cmd::Resume { .. }) => cmd_task(c),
        Cmd::Check { .. } | Cmd::Telegram { .. } | Cmd::Agent { .. } => unreachable!(),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{}", e.message());
            eprintln!("fabr: {}", e.message());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
