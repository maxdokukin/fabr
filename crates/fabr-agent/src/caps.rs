use crate::toolset::{ReadOnly, ToolSet, default_tools};
use crate::verify::ALLOWED_COMMANDS;
use fabr_core::FabrError;
use fabr_core::setup::{Capabilities, Ctx, ExecuteMode, settings_read, settings_write};
use fabr_core::taskrec::{PlanMeta, PlanRequires};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Harness {
    pub mode: ExecuteMode,
    pub caps: Capabilities,
    pub auto_mode: bool,
}

impl Harness {
    pub fn standard() -> Harness {
        Harness { mode: ExecuteMode::Full, caps: static_caps(), auto_mode: false }
    }

    pub fn effective(&self) -> Capabilities {
        let keep = |list: &[String], known: &[&str]| -> Vec<String> {
            known.iter().filter(|k| list.iter().any(|l| l == **k)).map(|k| (*k).to_string()).collect()
        };
        let all = default_tools();
        let names = all.names();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let tools = keep(&self.caps.tools, &names);
        let checks = keep(&self.caps.checks, &ALLOWED_COMMANDS);
        match self.mode {
            ExecuteMode::Full => Capabilities { tools, checks },
            ExecuteMode::ReadOnly => {
                let read_only = ReadOnly(&*all).names();
                Capabilities { tools: tools.into_iter().filter(|t| read_only.contains(t)).collect(), checks: Vec::new() }
            }
            ExecuteMode::Off => Capabilities::default(),
        }
    }

    pub fn planning(&self) -> Capabilities {
        let full = Harness { mode: ExecuteMode::Full, ..self.clone() }.effective();
        if full.is_empty() { static_caps() } else { full }
    }

    pub fn missing(&self, meta: &PlanMeta) -> Option<String> {
        self.missing_with(meta, &[])
    }

    pub fn missing_with(&self, meta: &PlanMeta, also: &[String]) -> Option<String> {
        let need = requires_of(meta);
        let mut have = self.effective();
        if self.mode == ExecuteMode::Full {
            for name in also {
                if !have.tools.contains(name) {
                    have.tools.push(name.clone());
                }
            }
        }
        let lacks = |wanted: &[String], got: &[String]| -> Vec<String> { wanted.iter().filter(|w| !got.contains(w)).cloned().collect() };
        let tools = lacks(&need.tools, &have.tools);
        let checks = lacks(&need.checks, &have.checks);
        let mut parts = Vec::new();
        if !tools.is_empty() {
            parts.push(tools.join(", "));
        }
        if !checks.is_empty() {
            parts.push(format!("checks: {}", checks.join(", ")));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

pub fn static_caps() -> Capabilities {
    Capabilities { tools: default_tools().names(), checks: ALLOWED_COMMANDS.iter().map(|c| (*c).to_string()).collect() }
}

pub fn detect() -> Capabilities {
    Capabilities {
        tools: default_tools().names(),
        checks: ALLOWED_COMMANDS.iter().filter(|c| on_path(c)).map(|c| (*c).to_string()).collect(),
    }
}

pub fn requires_of(meta: &PlanMeta) -> PlanRequires {
    let mut tools: Vec<String> = Vec::new();
    for t in &meta.tools {
        if !tools.contains(t) {
            tools.push(t.clone());
        }
    }
    let mut checks: Vec<String> = Vec::new();
    for c in &meta.checks {
        if !checks.contains(&c.cmd) {
            checks.push(c.cmd.clone());
        }
    }
    PlanRequires { tools, checks }
}

fn on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|dir| runnable(&dir.join(bin)))
}

fn runnable(file: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(file) else { return false };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn load(ctx: &Ctx) -> Result<Harness, FabrError> {
    let read = settings_read(ctx)?;
    let mut settings = read.settings;
    if read.exists && settings.capabilities.is_empty() {
        settings.capabilities = detect();
        if let Err(e) = settings_write(ctx, fabr_core::setup::SettingsWriteReq { settings: settings.clone() }) {
            log::warn!("could not save detected capabilities: {}", e.message());
        }
    }
    Ok(Harness { mode: settings.execute, caps: settings.capabilities, auto_mode: settings.auto_mode })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_is_written_back_and_the_effective_set_follows_the_mode() {
        let home = tempfile::tempdir().unwrap();
        let ctx = Ctx::new(home.path());
        std::fs::create_dir_all(ctx.space()).unwrap();
        std::fs::write(ctx.settings_path(), "model = \"claude-sonnet-5-5\"\nauto_mode = true\n").unwrap();
        let h = load(&ctx).unwrap();
        assert_eq!(h.mode, ExecuteMode::Full);
        assert_eq!(h.caps.tools, ["read_file", "list_dir", "write_file", "make_dir"]);
        assert!(
            h.auto_mode && h.caps.tools == default_tools().names() && h.caps.checks.iter().all(|c| ALLOWED_COMMANDS.contains(&c.as_str()))
        );
        assert!(h.caps.checks.contains(&"cat".to_string()) || std::env::var_os("PATH").is_none());
        let saved = settings_read(&ctx).unwrap().settings;
        assert_eq!(saved.capabilities, h.caps);
        assert!(std::fs::read_to_string(ctx.settings_path()).unwrap().contains("[capabilities]"));
        let ro = Harness { mode: ExecuteMode::ReadOnly, ..h.clone() };
        assert_eq!(ro.effective(), Capabilities { tools: vec!["read_file".into(), "list_dir".into()], checks: vec![] });
        assert_eq!(ro.planning(), h.effective());
        assert!(Harness { mode: ExecuteMode::Off, ..h.clone() }.effective().is_empty());
        let mut edited = saved;
        edited.capabilities.checks = vec!["cat".into(), "git".into()];
        edited.execute = ExecuteMode::ReadOnly;
        settings_write(&ctx, fabr_core::setup::SettingsWriteReq { settings: edited }).unwrap();
        let again = load(&ctx).unwrap();
        assert_eq!(again.mode, ExecuteMode::ReadOnly);
        assert_eq!(again.caps.checks, vec!["cat".to_string(), "git".to_string()]);
        assert_eq!(Harness { mode: ExecuteMode::Full, ..again }.effective().checks, vec!["cat".to_string()]);
    }
}
