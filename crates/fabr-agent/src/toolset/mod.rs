mod demo;
mod sandbox;

pub use demo::DemoTools;
pub use sandbox::Sandbox;

pub fn read_node_bytes(node_dir: &Path, rel: &str) -> Result<Vec<u8>, String> {
    let sb = Sandbox::new(node_dir)?;
    sandbox::reject_symlink(sb.root(), rel)?;
    let path = sb.resolve(rel, sandbox::Missing::No)?;
    std::fs::read(path).map_err(|e| e.to_string())
}

pub fn read_node_text(node_dir: &Path, rel: &str) -> Result<String, String> {
    let sb = Sandbox::new(node_dir)?;
    sandbox::reject_symlink(sb.root(), rel)?;
    let path = sb.resolve(rel, sandbox::Missing::No)?;
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

pub fn write_node_bytes(node_dir: &Path, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let sb = Sandbox::new(node_dir)?;
    sandbox::reject_symlink(sb.root(), rel)?;
    if let Some(parent) = Path::new(rel).parent().filter(|p| !p.as_os_str().is_empty()) {
        sandbox::reject_symlink(sb.root(), &parent.to_string_lossy())?;
        sb.resolve(&parent.to_string_lossy(), sandbox::Missing::No)?;
    }
    let path = sb.resolve(rel, sandbox::Missing::Last)?;
    if path.is_dir() {
        return Err(format!("{rel} is a directory"));
    }
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

use crate::model::ToolCall;
use std::path::Path;
use std::sync::Arc;

pub type ToolOutcome = Result<String, String>;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub read_only: bool,
}

pub trait ToolSet: Send + Sync {
    fn specs(&self) -> Vec<ToolSpec>;

    fn call(&self, node_dir: &Path, call: &ToolCall) -> ToolOutcome;

    fn names(&self) -> Vec<String> {
        self.specs().into_iter().map(|s| s.name).collect()
    }
}

pub fn default_tools() -> Arc<dyn ToolSet> {
    Arc::new(DemoTools)
}

pub struct ReadOnly<'a>(pub &'a dyn ToolSet);

impl ToolSet for ReadOnly<'_> {
    fn specs(&self) -> Vec<ToolSpec> {
        self.0.specs().into_iter().filter(|s| s.read_only).collect()
    }

    fn call(&self, node_dir: &Path, call: &ToolCall) -> ToolOutcome {
        if self.names().contains(&call.name) { self.0.call(node_dir, call) } else { Err(format!("tool {} is not available", call.name)) }
    }
}

pub struct Only<'a> {
    inner: &'a dyn ToolSet,
    names: Vec<String>,
}

impl<'a> Only<'a> {
    pub fn new(inner: &'a dyn ToolSet, names: Vec<String>) -> Self {
        Only { inner, names }
    }
}

impl ToolSet for Only<'_> {
    fn specs(&self) -> Vec<ToolSpec> {
        self.inner.specs().into_iter().filter(|s| self.names.contains(&s.name)).collect()
    }

    fn call(&self, node_dir: &Path, call: &ToolCall) -> ToolOutcome {
        if self.names.contains(&call.name) { self.inner.call(node_dir, call) } else { Err(format!("tool {} is not allowed", call.name)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_hides_the_mutating_tools_and_refuses_their_calls() {
        let all = DemoTools;
        let ro = ReadOnly(&all);
        assert_eq!(all.names(), ["read_file", "list_dir", "write_file", "make_dir"]);
        assert_eq!(ro.names(), ["read_file", "list_dir"]);
        let tmp = tempfile::tempdir().unwrap();
        let call = ToolCall { name: "make_dir".into(), path: "x".into(), content: None, params: None };
        assert!(ro.call(tmp.path(), &call).is_err());
        assert!(!tmp.path().join("x").exists());
    }
}
