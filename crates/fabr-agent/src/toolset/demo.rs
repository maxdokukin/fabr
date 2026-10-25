use super::sandbox::{Missing, Sandbox, reserved_name};
use super::{ToolOutcome, ToolSet, ToolSpec};
use crate::model::ToolCall;
use std::path::Path;

const READ_CAP: usize = 256 * 1024;

pub struct DemoTools;

fn spec(name: &str, description: &str, properties: serde_json::Value, required: &[&str], read_only: bool) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema: serde_json::json!({"type": "object", "properties": properties, "required": required}),
        read_only,
    }
}

impl ToolSet for DemoTools {
    fn specs(&self) -> Vec<ToolSpec> {
        let path = serde_json::json!({"type": "string"});
        vec![
            spec("read_file", "Read a file inside the node (up to 256 KiB)", serde_json::json!({"path": path}), &["path"], true),
            spec("list_dir", "List a directory inside the node; folders end with /", serde_json::json!({"path": path}), &[], true),
            spec(
                "write_file",
                "Write a Markdown (.md) file inside the node; the parent folder must exist",
                serde_json::json!({"path": path, "content": {"type": "string"}}),
                &["path", "content"],
                false,
            ),
            spec("make_dir", "Create a folder, or nested folders, inside the node", serde_json::json!({"path": path}), &["path"], false),
        ]
    }

    fn call(&self, node_dir: &Path, call: &ToolCall) -> ToolOutcome {
        let sb = Sandbox::new(node_dir)?;
        match call.name.as_str() {
            "read_file" => read_file(&sb, &call.path),
            "list_dir" => list_dir(&sb, &call.path),
            "write_file" => write_file(&sb, &call.path, call.content.as_deref()),
            "make_dir" => make_dir(&sb, &call.path),
            other => Err(format!("unknown tool {other}")),
        }
    }
}

fn read_file(sb: &Sandbox, rel: &str) -> ToolOutcome {
    let path = sb.resolve(rel, Missing::No)?;
    if !path.is_file() {
        return Err(format!("no such file {rel}"));
    }
    let mut bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
    bytes.truncate(READ_CAP);
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn list_dir(sb: &Sandbox, rel: &str) -> ToolOutcome {
    let path = if rel.is_empty() || rel == "." { sb.root().to_path_buf() } else { sb.resolve(rel, Missing::No)? };
    if !path.is_dir() {
        return Err(format!("no such directory {rel}"));
    }
    let mut names = Vec::new();
    for ent in std::fs::read_dir(&path).map_err(|e| format!("{rel}: {e}"))? {
        let ent = ent.map_err(|e| e.to_string())?;
        let Ok(name) = ent.file_name().into_string() else { continue };
        if reserved_name(&name) {
            continue;
        }
        let dir = ent.path().is_dir();
        names.push(if dir { format!("{name}/") } else { name });
    }
    names.sort();
    Ok(if names.is_empty() { "(empty)".to_string() } else { names.join("\n") })
}

fn write_file(sb: &Sandbox, rel: &str, content: Option<&str>) -> ToolOutcome {
    let is_md = Path::new(rel).extension().is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if !is_md {
        return Err(format!("write_file only writes .md files; got {rel}"));
    }
    let Some(body) = content else {
        return Err("write_file needs content".into());
    };
    if let Some(parent) = Path::new(rel).parent().filter(|p| !p.as_os_str().is_empty()) {
        let parent = parent.to_string_lossy();
        sb.resolve(&parent, Missing::No).map_err(|e| {
            if e.starts_with("no such path") { format!("folder {parent} does not exist; create it with make_dir first") } else { e }
        })?;
    }
    let path = sb.resolve(rel, Missing::Last)?;
    if path.is_dir() {
        return Err(format!("{rel} is a directory"));
    }
    std::fs::write(&path, body).map_err(|e| format!("{rel}: {e}"))?;
    Ok(format!("wrote {} bytes to {rel}", body.len()))
}

fn make_dir(sb: &Sandbox, rel: &str) -> ToolOutcome {
    let path = sb.resolve(rel, Missing::Any)?;
    std::fs::create_dir_all(&path).map_err(|e| format!("{rel}: {e}"))?;
    Ok(format!("folder {rel} is ready"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let node = tmp.path().join("node");
        std::fs::create_dir_all(&node).unwrap();
        (tmp, node)
    }

    fn run(node: &Path, name: &str, path: &str, content: Option<&str>) -> ToolOutcome {
        DemoTools.call(node, &ToolCall { name: name.into(), path: path.into(), content: content.map(str::to_string), params: None })
    }

    #[test]
    fn write_file_takes_markdown_only_and_needs_the_folder() {
        let (_tmp, node) = node();
        assert_eq!(run(&node, "write_file", "notes.txt", Some("x")).unwrap_err(), "write_file only writes .md files; got notes.txt");
        assert!(!node.join("notes.txt").exists());
        assert!(run(&node, "write_file", "docs/a.md", Some("x")).unwrap_err().contains("make_dir first"));
        assert_eq!(run(&node, "write_file", "A.MD", Some("hi")).unwrap(), "wrote 2 bytes to A.MD");
        assert!(run(&node, "write_file", ".fabr/x.md", Some("no")).is_err());
    }

    #[test]
    fn make_dir_nests_is_idempotent_and_refuses_escapes() {
        let (_tmp, node) = node();
        run(&node, "make_dir", "a/b/c", None).unwrap();
        run(&node, "make_dir", "a/b/c", None).unwrap();
        assert!(node.join("a/b/c").is_dir());
        assert!(run(&node, "make_dir", "../out", None).is_err());
        assert!(run(&node, "make_dir", ".fabric-space", None).is_err());
        assert!(run(&node, "make_dir", "a/.fabr/x", None).is_err());
        assert!(!node.join(".fabric-space").exists());
    }

    #[test]
    fn read_and_list_see_any_file_and_hide_reserved_dirs() {
        let (_tmp, node) = node();
        std::fs::write(node.join("data.csv"), b"a,b\xff").unwrap();
        std::fs::create_dir_all(node.join(".fabr")).unwrap();
        std::fs::create_dir_all(node.join("sub")).unwrap();
        assert!(run(&node, "read_file", "data.csv", None).unwrap().starts_with("a,b"));
        assert!(run(&node, "read_file", ".fabr", None).is_err());
        assert_eq!(run(&node, "list_dir", "", None).unwrap(), "data.csv\nsub/");
    }
}
