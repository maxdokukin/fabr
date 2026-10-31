use fabr_agent::runner::{Options, Runner, Sink, start_gate, write_failure};
use fabr_core::FabrError;
use fabr_core::redact::redact;
use fabr_core::setup::Ctx;
use fabr_core::workspace::Workspace;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const INTERVAL_S: u64 = 5;
const MAX_RESTARTS: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(600);
const STOP_WAIT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub running: bool,
    pub reason: Option<String>,
}

struct Live {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
    done: Receiver<()>,
}

pub struct AgentHost {
    live: Mutex<Option<Live>>,
    reason: Arc<Mutex<Option<String>>>,
    restart_backoff: Duration,
    stop_wait: Duration,
}

impl Default for AgentHost {
    fn default() -> Self {
        AgentHost {
            live: Mutex::new(None),
            reason: Arc::new(Mutex::new(None)),
            restart_backoff: Duration::from_secs(2),
            stop_wait: STOP_WAIT,
        }
    }
}

fn set(reason: &Mutex<Option<String>>, v: Option<String>) {
    *reason.lock().unwrap_or_else(PoisonError::into_inner) = v;
}

fn pause(total: Duration, stop: &AtomicBool) {
    let until = Instant::now() + total;
    while Instant::now() < until && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(20).min(total));
    }
}

fn supervise(
    ctx: Ctx,
    folder: PathBuf,
    workspace: fn(PathBuf) -> Result<Workspace, FabrError>,
    stop: Arc<AtomicBool>,
    reason: Arc<Mutex<Option<String>>>,
    backoff: Duration,
) {
    let mut failures: VecDeque<Instant> = VecDeque::new();
    while !stop.load(Ordering::SeqCst) {
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut runner = workspace(folder.clone())
                .and_then(|ws| Runner::new(&ctx, ws, Options { interval_s: INTERVAL_S, once: false }, Sink::default()))?;
            set(&reason, None);
            runner.run(&stop)
        }));
        let failure = match run {
            Ok(Ok(())) => return,
            Ok(Err(e)) => e.message(),
            Err(_) => "the agent crashed unexpectedly".to_string(),
        };
        let msg = redact(&failure);
        log::error!("agent stopped: {msg}");
        set(&reason, Some(msg.clone()));
        let now = Instant::now();
        failures.push_back(now);
        while failures.front().is_some_and(|t| now.duration_since(*t) > RESTART_WINDOW) {
            failures.pop_front();
        }
        if failures.len() > MAX_RESTARTS {
            let text = format!("gave up after {MAX_RESTARTS} restarts in {} minutes: {msg}", RESTART_WINDOW.as_secs() / 60);
            set(&reason, Some(text.clone()));
            if let Err(e) = write_failure(&ctx.space(), &text) {
                log::error!("could not write the backend error: {e}");
            }
            return;
        }
        pause(backoff * failures.len() as u32, &stop);
    }
}

impl AgentHost {
    #[cfg(test)]
    fn with_timing(restart_backoff: Duration, stop_wait: Duration) -> AgentHost {
        AgentHost { restart_backoff, stop_wait, ..AgentHost::default() }
    }

    pub fn status(&self) -> Status {
        let live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        let running = live.as_ref().is_some_and(|l| !l.handle.is_finished());
        Status { running, reason: self.reason.lock().unwrap_or_else(PoisonError::into_inner).clone() }
    }

    pub fn start(&self, ctx: Ctx, workspace: fn(PathBuf) -> Result<Workspace, FabrError>) -> Status {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        if live.as_ref().is_some_and(|l| !l.handle.is_finished()) {
            drop(live);
            return self.status();
        }
        if let Some(old) = live.take() {
            let _ = old.handle.join();
        }
        set(&self.reason, None);
        let gate = match start_gate(&ctx) {
            Ok(g) => g,
            Err(e) => {
                let msg = redact(&e.message());
                log::warn!("agent not started: {msg}");
                set(&self.reason, Some(msg));
                drop(live);
                return self.status();
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        fabr_agent::claude::allow_runs();
        let (flag, reason, backoff) = (stop.clone(), self.reason.clone(), self.restart_backoff);
        let (finished, done) = channel::<()>();
        let spawned = std::thread::Builder::new().name("fabr-agent".into()).spawn(move || {
            supervise(ctx, gate.default_folder, workspace, flag, reason, backoff);
            let _ = finished.send(());
        });
        match spawned {
            Ok(handle) => *live = Some(Live { stop, handle, done }),
            Err(e) => set(&self.reason, Some(format!("could not start the agent thread: {e}"))),
        }
        drop(live);
        self.status()
    }

    pub fn stop(&self) {
        let live = self.live.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(l) = live {
            l.stop.store(true, Ordering::SeqCst);
            fabr_agent::claude::abort_all();
            if l.done.recv_timeout(self.stop_wait).is_ok() {
                let _ = l.handle.join();
            } else {
                log::warn!("the agent is still busy; it is abandoned on exit");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabr_core::AppDirs;

    fn ws(root: PathBuf) -> Result<Workspace, FabrError> {
        let base = root.with_file_name("cfgdir");
        AppDirs { config: base.join("c"), cache: base.join("k") }.workspace(Some(root))
    }

    #[test]
    fn refuses_before_setup_and_runs_after() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("work");
        std::fs::create_dir_all(&folder).unwrap();
        let ctx = Ctx::new(dir.path());
        let host = AgentHost::default();

        let s = host.start(ctx.clone(), ws);
        assert!(!s.running);
        assert!(s.reason.unwrap().contains("Setup has not been started"));

        std::fs::create_dir_all(ctx.space()).unwrap();
        settings(&ctx, &folder);

        let s = host.start(ctx.clone(), ws);
        assert_eq!(s, Status { running: true, reason: None });
        assert_eq!(host.start(ctx.clone(), ws), s);
        let beat = ctx.space().join("registry").join("_backend.yaml");
        for _ in 0..50 {
            if beat.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(beat.exists());
        host.stop();
        assert!(!host.status().running);
        assert!(!beat.exists());
    }

    fn ready(dir: &std::path::Path) -> (Ctx, PathBuf) {
        let folder = dir.join("work");
        std::fs::create_dir_all(&folder).unwrap();
        let ctx = Ctx::new(dir);
        std::fs::create_dir_all(ctx.space()).unwrap();
        settings(&ctx, &folder);
        (ctx, folder)
    }

    fn settings(ctx: &Ctx, folder: &std::path::Path) {
        let claude = ctx.home.join("fake-claude");
        std::fs::write(&claude, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(
            ctx.settings_path(),
            format!(
                "default_folder = \"{}\"\nmodel = \"haiku\"\nclaude_bin = \"{}\"\n\n[setup]\ncompleted = true\n",
                folder.display(),
                claude.display()
            ),
        )
        .unwrap();
    }

    fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..200 {
            if ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for {what}");
    }

    static FAILING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn always_fails(_: PathBuf) -> Result<Workspace, FabrError> {
        FAILING.fetch_add(1, Ordering::SeqCst);
        Err(FabrError::Io("workspace unavailable".into()))
    }

    #[test]
    fn startup_errors_restart_five_times_then_the_error_lands_in_the_backend_file() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = ready(dir.path());
        let host = AgentHost::with_timing(Duration::from_millis(1), Duration::from_secs(1));
        host.start(ctx.clone(), always_fails);
        wait_until("the supervisor to give up", || !host.status().running);
        assert_eq!(FAILING.load(Ordering::SeqCst), 6);
        let reason = host.status().reason.unwrap();
        assert!(reason.contains("gave up after 5 restarts") && reason.contains("workspace unavailable"), "{reason}");
        let beat = std::fs::read_to_string(ctx.space().join("registry/_backend.yaml")).unwrap();
        assert!(beat.contains("last_error:") && beat.contains("workspace unavailable"), "{beat}");
    }

    static PANICS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn panics_twice(root: PathBuf) -> Result<Workspace, FabrError> {
        if PANICS.fetch_add(1, Ordering::SeqCst) < 2 {
            panic!("boom");
        }
        ws(root)
    }

    #[test]
    fn a_panic_restarts_the_runner_and_it_then_keeps_running() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = ready(dir.path());
        let host = AgentHost::with_timing(Duration::from_millis(1), Duration::from_secs(2));
        host.start(ctx.clone(), panics_twice);
        let beat = ctx.space().join("registry/_backend.yaml");
        wait_until("the third attempt to run", || PANICS.load(Ordering::SeqCst) >= 3 && beat.exists());
        wait_until("the failure reason to clear", || host.status().reason.is_none());
        assert!(host.status().running);
        host.stop();
        assert!(!host.status().running);
        assert!(!beat.exists());
    }

    fn blocks_forever(_: PathBuf) -> Result<Workspace, FabrError> {
        std::thread::sleep(Duration::from_secs(30));
        Err(FabrError::Io("never".into()))
    }

    #[test]
    fn stop_does_not_wait_for_a_blocked_runner() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _) = ready(dir.path());
        let host = AgentHost::with_timing(Duration::from_millis(1), Duration::from_millis(300));
        assert!(host.start(ctx, blocks_forever).running);
        std::thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        host.stop();
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    }
}
