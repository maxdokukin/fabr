use crate::verify::ALLOWED_COMMANDS;
use fabr_core::FabrError;
use fabr_core::setup::{Capabilities, Ctx, ExecuteMode, settings_read, settings_write};
use fabr_core::taskrec::{PlanMeta, PlanRequires};
use std::path::{Path, PathBuf};

pub const CLAUDE_TOOLS: [&str; 10] =
    ["Read", "Glob", "Grep", "Edit", "Write", "MultiEdit", "NotebookEdit", "WebFetch", "WebSearch", "Bash"];
pub const ALWAYS_GRANTED: [&str; 3] = ["Read", "Glob", "Grep"];

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
        let known: Vec<String> =
            CLAUDE_TOOLS.iter().filter(|k| self.caps.tools.iter().any(|t| t == **k)).map(|k| (*k).to_string()).collect();
        let tools = if known.is_empty() { CLAUDE_TOOLS.iter().map(|k| (*k).to_string()).collect() } else { known };
        let checks: Vec<String> =
            ALLOWED_COMMANDS.iter().filter(|k| self.caps.checks.iter().any(|c| c == **k)).map(|k| (*k).to_string()).collect();
        match self.mode {
            ExecuteMode::Full => Capabilities { tools, checks },
            ExecuteMode::ReadOnly => Capabilities { tools: ALWAYS_GRANTED.iter().map(|t| (*t).to_string()).collect(), checks: Vec::new() },
            ExecuteMode::Off => Capabilities::default(),
        }
    }

    pub fn planning(&self) -> Capabilities {
        let full = Harness { mode: ExecuteMode::Full, ..self.clone() }.effective();
        if full.checks.is_empty() { Capabilities { tools: full.tools, checks: static_caps().checks } } else { full }
    }

    pub fn read_only(&self) -> bool {
        self.mode != ExecuteMode::Full
    }

    pub fn missing(&self, meta: &PlanMeta) -> Option<String> {
        let need = requires_of(meta);
        let have = self.effective();
        let tools: Vec<String> = need
            .tools
            .iter()
            .filter(|rule| {
                let base = rule_tool(rule);
                !valid_rule(rule)
                    || (!ALWAYS_GRANTED.contains(&base) || self.mode == ExecuteMode::Off) && !have.tools.iter().any(|t| t == base)
            })
            .cloned()
            .collect();
        let checks: Vec<String> = need.checks.iter().filter(|c| !have.checks.contains(c)).cloned().collect();
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

pub fn rule_tool(rule: &str) -> &str {
    rule.split_once('(').map_or(rule, |(name, _)| name).trim()
}

pub fn valid_rule(rule: &str) -> bool {
    let (name, spec) = match rule.split_once('(') {
        None => (rule, None),
        Some((name, rest)) => match rest.strip_suffix(')') {
            Some(spec) => (name, Some(spec)),
            None => return false,
        },
    };
    if !CLAUDE_TOOLS.contains(&name) {
        return false;
    }
    match spec {
        None => name != "Bash",
        Some(spec) => {
            spec == spec.trim()
                && !spec.is_empty()
                && spec.len() <= 200
                && !spec.chars().any(|c| c.is_control() || c == '(' || c == ')')
                && spec.chars().next().is_some_and(|c| c != '*' && c != ':')
        }
    }
}

pub fn auto_safe(rule: &str) -> bool {
    if !valid_rule(rule) {
        return false;
    }
    let local = |spec: &str| {
        !spec.starts_with('/') && !spec.starts_with('~') && !spec.split('/').any(|p| p == "..") && !spec.contains(".fabric-space")
    };
    match rule.split_once('(') {
        None => matches!(rule, "Read" | "Glob" | "Grep" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit"),
        Some((name, rest)) => {
            matches!(name, "Read" | "Glob" | "Grep" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit") && local(rest.trim_end_matches(')'))
        }
    }
}

pub fn executor_rules(meta: Option<&PlanMeta>) -> Vec<String> {
    let mut out: Vec<String> = ALWAYS_GRANTED.iter().map(|t| (*t).to_string()).collect();
    for rule in meta.map(|m| m.tools.as_slice()).unwrap_or_default() {
        if valid_rule(rule) && !out.contains(rule) {
            out.push(rule.clone());
        }
    }
    out
}

pub fn static_caps() -> Capabilities {
    Capabilities {
        tools: CLAUDE_TOOLS.iter().map(|t| (*t).to_string()).collect(),
        checks: ALLOWED_COMMANDS.iter().map(|c| (*c).to_string()).collect(),
    }
}

pub fn detect() -> Capabilities {
    Capabilities {
        tools: CLAUDE_TOOLS.iter().map(|t| (*t).to_string()).collect(),
        checks: ALLOWED_COMMANDS.iter().filter(|c| on_path(c).is_some()).map(|c| (*c).to_string()).collect(),
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

pub(crate) fn on_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(bin)).find(|f| runnable(f))
}

pub(crate) fn runnable(file: &Path) -> bool {
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
    fn auto_mode_takes_only_local_file_rules() {
        for ok in ["Write", "Edit", "Edit(notes/*.md)", "Read(./a.md)"] {
            assert!(auto_safe(ok), "{ok}");
        }
        for no in [
            "Bash(npm test)",
            "WebFetch",
            "WebSearch",
            "Edit(//etc/hosts)",
            "Edit(/etc/x)",
            "Write(~/x)",
            "Edit(../x)",
            "Edit(.fabric-space/tasks/a.md)",
            "Bash",
        ] {
            assert!(!auto_safe(no), "{no}");
        }
    }
    use fabr_core::taskrec::Check;

    fn meta(tools: &[&str], checks: &[&str]) -> PlanMeta {
        PlanMeta {
            reversibility: Some("reversible".into()),
            tools: tools.iter().map(|t| (*t).to_string()).collect(),
            assumptions: vec![],
            checks: checks.iter().map(|c| Check { cmd: (*c).into(), args: vec![] }).collect(),
            requires: Default::default(),
        }
    }

    #[test]
    fn claude_code_rules_are_accepted_and_garbage_is_not() {
        for good in ["Read", "Edit", "Write", "MultiEdit", "NotebookEdit", "WebFetch", "WebSearch", "Bash(npm test:*)", "Bash(git status)"]
        {
            assert!(valid_rule(good), "{good}");
        }
        assert!(valid_rule("WebFetch(domain:example.com)") && valid_rule("Edit(//tmp/notes.md)"));
        for bad in
            ["Bash", "Bash()", "Bash(*)", "Bash(:*)", "Bash(rm -rf /", "write_file", "browser", "", "Edit(a)(b)", "Bash(a\nb)", " Edit"]
        {
            assert!(!valid_rule(bad), "{bad:?}");
        }
        assert_eq!(rule_tool("Bash(npm test:*)"), "Bash");
        assert_eq!(executor_rules(Some(&meta(&["Edit", "Read", "browser"], &[]))), ["Read", "Glob", "Grep", "Edit"]);
    }

    #[test]
    fn detection_is_written_back_and_the_effective_set_follows_the_mode() {
        let home = tempfile::tempdir().unwrap();
        let ctx = Ctx::new(home.path());
        std::fs::create_dir_all(ctx.space()).unwrap();
        std::fs::write(ctx.settings_path(), "model = \"sonnet\"\nauto_mode = true\n").unwrap();
        let h = load(&ctx).unwrap();
        assert_eq!(h.mode, ExecuteMode::Full);
        assert!(h.auto_mode && h.caps.tools == CLAUDE_TOOLS && h.caps.checks.iter().all(|c| ALLOWED_COMMANDS.contains(&c.as_str())));
        assert!(h.caps.checks.contains(&"cat".to_string()) || std::env::var_os("PATH").is_none());
        let saved = settings_read(&ctx).unwrap().settings;
        assert_eq!(saved.capabilities, h.caps);
        assert!(h.missing(&meta(&["Edit", "Bash(npm test:*)"], &["cat"])).is_none());
        let ro = Harness { mode: ExecuteMode::ReadOnly, ..h.clone() };
        assert_eq!(ro.effective(), Capabilities { tools: vec!["Read".into(), "Glob".into(), "Grep".into()], checks: vec![] });
        assert_eq!(ro.missing(&meta(&["Read", "Edit", "Bash(ls:*)"], &[])).as_deref(), Some("Edit, Bash(ls:*)"));
        assert!(ro.read_only() && !h.read_only());
        let off = Harness { mode: ExecuteMode::Off, ..h.clone() };
        assert!(off.effective().is_empty());
        assert_eq!(off.missing(&meta(&["Read"], &[])).as_deref(), Some("Read"));
        let legacy = Harness { caps: Capabilities { tools: vec!["read_file".into()], checks: vec!["cat".into()] }, ..h.clone() };
        assert_eq!(legacy.effective().tools, CLAUDE_TOOLS);
        assert_eq!(h.missing(&meta(&["browser"], &["git"])).as_deref(), Some("browser, checks: git"));
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
