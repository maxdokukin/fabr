use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    No,
    Last,
    Any,
}

pub struct Sandbox {
    root: PathBuf,
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
        Ok(Sandbox { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn under_reserved(&self, path: &Path) -> bool {
        path.strip_prefix(&self.root)
            .is_ok_and(|rel| rel.components().any(|c| matches!(c, Component::Normal(s) if reserved_name(&s.to_string_lossy()))))
    }

    fn outside(&self, path: &Path) -> bool {
        !path.starts_with(&self.root) || self.under_reserved(path)
    }

    pub fn resolve(&self, rel: &str, missing: Missing) -> Result<PathBuf, String> {
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
                    if fresh || cur == self.root {
                        return Err("path is outside the node".into());
                    }
                    cur.pop();
                    if self.outside(&cur) {
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
                            if self.outside(&canon) {
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
        if self.outside(&cur) {
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
}
