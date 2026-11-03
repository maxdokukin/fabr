use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    No,
    Last,
    Any,
}

pub struct Sandbox {
    root: PathBuf,
    /// Optional workspace root, an ancestor of `root`. When set, read operations
    /// (`resolve_read`) may ascend out of the node as long as they stay inside it.
    /// Writes always stay contained to `root`. `None` keeps the historical behavior.
    read_root: Option<PathBuf>,
}

pub fn reserved_name(name: &str) -> bool {
    name == ".fabr" || name == ".fabric-space"
}

/// Sponsor writes fail closed. A symlink is not followed, even when it stays inside the node.
pub fn reject_symlink(root: &Path, rel: &str) -> Result<(), String> {
    if rel.is_empty() || rel.contains('\0') || Path::new(rel).is_absolute() {
        return Err("path is not authorized".into());
    }
    let mut cur = root.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if reserved_name(&name) {
                    return Err("path is reserved".into());
                }
                let next = cur.join(name.as_ref());
                if std::fs::symlink_metadata(&next).is_ok_and(|meta| meta.file_type().is_symlink()) {
                    return Err("symlinked path is not authorized".into());
                }
                cur = next;
            }
            _ => return Err("path is not authorized".into()),
        }
    }
    Ok(())
}

impl Sandbox {
    pub fn new(node_dir: &Path) -> Result<Sandbox, String> {
        let root = node_dir.canonicalize().map_err(|e| format!("node root: {e}"))?;
        Ok(Sandbox { root, read_root: None })
    }

    /// Broaden the READ scope to the workspace root (an ancestor of the node).
    /// Writes stay contained to the node. If `read_root` does not canonicalize or
    /// is not an ancestor of the node, the read scope silently stays at the node.
    pub fn with_read_root(node_dir: &Path, read_root: &Path) -> Result<Sandbox, String> {
        let mut sb = Sandbox::new(node_dir)?;
        #[allow(clippy::collapsible_if)]
        if let Ok(canon) = read_root.canonicalize() {
            if sb.root.starts_with(&canon) {
                sb.read_root = Some(canon);
            }
        }
        Ok(sb)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The boundary for reads: the workspace root when set, else the node.
    pub fn read_root(&self) -> &Path {
        self.read_root.as_deref().unwrap_or(&self.root)
    }

    fn under_reserved(&self, boundary: &Path, path: &Path) -> bool {
        path.strip_prefix(boundary)
            .is_ok_and(|rel| rel.components().any(|c| matches!(c, Component::Normal(s) if reserved_name(&s.to_string_lossy()))))
    }

    fn outside_boundary(&self, boundary: &Path, path: &Path) -> bool {
        !path.starts_with(boundary) || self.under_reserved(boundary, path)
    }

    /// Resolve a path for READING. Relative paths still resolve from the node, but
    /// `..` may ascend up to the workspace read root. Symlink escape past the read
    /// root and reserved names stay rejected. With no read root this equals `resolve`.
    pub fn resolve_read(&self, rel: &str, missing: Missing) -> Result<PathBuf, String> {
        self.resolve_within(self.read_root().to_path_buf(), rel, missing)
    }

    pub fn resolve(&self, rel: &str, missing: Missing) -> Result<PathBuf, String> {
        self.resolve_within(self.root.clone(), rel, missing)
    }

    fn resolve_within(&self, boundary: PathBuf, rel: &str, missing: Missing) -> Result<PathBuf, String> {
        if rel.is_empty() || rel.contains('\0') {
            return Err("bad path".into());
        }
        let rel_path = Path::new(rel);
        if rel_path.is_absolute() {
            return Err("path is outside the node".into());
        }
        let mut cur = self.root.clone();
        let mut fresh = false;
        let comps: Vec<_> = rel_path.components().collect();
        for (i, comp) in comps.iter().enumerate() {
            let last = i + 1 == comps.len();
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    if fresh || cur == boundary {
                        return Err("path is outside the node".into());
                    }
                    cur.pop();
                    if self.outside_boundary(&boundary, &cur) {
                        return Err("path is outside the node".into());
                    }
                }
                Component::Normal(name) => {
                    let name = name.to_string_lossy();
                    if reserved_name(&name) {
                        return Err("path is reserved".into());
                    }
                    let next = cur.join(name.as_ref());
                    if fresh {
                        cur = next;
                        continue;
                    }
                    match std::fs::symlink_metadata(&next) {
                        Ok(meta) => {
                            let link = meta.file_type().is_symlink();
                            let escape = if link { "symlink is outside the node" } else { "path is outside the node" };
                            let canon = std::fs::canonicalize(&next).map_err(|e| if link { escape.to_string() } else { e.to_string() })?;
                            if self.outside_boundary(&boundary, &canon) {
                                return Err(escape.into());
                            }
                            cur = canon;
                        }
                        Err(e)
                            if e.kind() == std::io::ErrorKind::NotFound
                                && (missing == Missing::Any || (last && missing == Missing::Last)) =>
                        {
                            fresh = true;
                            cur = next;
                        }
                        Err(_) => return Err(format!("no such path {rel}")),
                    }
                }
                _ => return Err("bad path".into()),
            }
        }
        if self.outside_boundary(&boundary, &cur) {
            return Err("path is outside the node".into());
        }
        Ok(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let node = tmp.path().join("node");
        std::fs::create_dir_all(&node).unwrap();
        (tmp, node)
    }

    #[test]
    fn reserved_paths_and_escapes_are_refused() {
        let (tmp, node) = node();
        std::fs::create_dir_all(node.join(".fabr")).unwrap();
        let sb = Sandbox::new(&node).unwrap();
        assert!(sb.resolve(".fabr/meta.toml", Missing::Last).is_err());
        assert!(sb.resolve(".fabric-space/x", Missing::Any).is_err());
        assert!(sb.resolve("../escaped.md", Missing::Last).is_err());
        assert!(sb.resolve("/etc/passwd", Missing::No).is_err());
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, node.join("out")).unwrap();
        std::os::unix::fs::symlink(outside.join("landed.md"), node.join("leak")).unwrap();
        assert!(sb.resolve("out/file.md", Missing::Last).is_err());
        assert!(sb.resolve("leak", Missing::Last).is_err());
        assert!(sb.resolve("a/b/c", Missing::Any).is_ok());
        assert!(sb.resolve("a/b/../../../x", Missing::Any).is_err());
    }

    #[test]
    fn read_root_broadens_reads_but_keeps_writes_in_the_node() {
        // workspace/ (read_root) > workspace/node (write-contained)
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let node = workspace.join("node");
        std::fs::create_dir_all(&node).unwrap();
        // A sibling file above the node but inside the workspace root.
        std::fs::write(workspace.join("ABOVE.md"), b"up").unwrap();
        let sb = Sandbox::with_read_root(&node, &workspace).unwrap();

        // Read ascends to the workspace root.
        assert!(sb.resolve_read("../ABOVE.md", Missing::No).is_ok());
        // ...but not past it.
        assert!(sb.resolve_read("../../escape.md", Missing::No).is_err());
        // Writes stay contained to the node even with a read root present.
        assert!(sb.resolve("../ABOVE.md", Missing::Last).is_err());
        // Reserved names stay hidden across the broadened read scope.
        std::fs::create_dir_all(workspace.join(".fabr")).unwrap();
        assert!(sb.resolve_read("../.fabr/meta.toml", Missing::No).is_err());

        // Back-compat: with no read root, reads stay contained to the node.
        let plain = Sandbox::new(&node).unwrap();
        assert!(plain.resolve_read("../ABOVE.md", Missing::No).is_err());
    }
}
