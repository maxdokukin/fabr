#![allow(dead_code)]
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SCRIPT: &str = r#"#!/bin/sh
d='@DIR@'
n=$(( $(cat "$d/count" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$d/count"
printf '%s\0' "$@" > "$d/argv.$n"
pwd > "$d/cwd.$n"
cat > "$d/stdin.$n"
sid=""
prev=""
for a in "$@"; do
  case "$prev" in --session-id|--resume) sid="$a";; esac
  prev="$a"
done
if [ ! -f "$d/out.$n" ]; then
  echo "fake claude has no canned output for call $n" >&2
  exit 3
fi
while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in
    "@SLEEP "*) sleep "${line#@SLEEP }";;
    "@SH "*) sh -c "${line#@SH }";;
    "@ERR "*) printf '%s\n' "${line#@ERR }" >&2;;
    *) printf '%s\n' "$line" | sed "s/@SID@/$sid/g";;
  esac
done < "$d/out.$n"
exit "$(cat "$d/exit.$n" 2>/dev/null || echo 0)"
"#;

pub struct FakeClaude {
    pub dir: PathBuf,
    pub bin: PathBuf,
    queued: std::cell::Cell<usize>,
}

impl FakeClaude {
    pub fn new(base: &Path) -> FakeClaude {
        let dir = base.join("fake-claude");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("claude");
        let tmp = dir.join("claude.tmp");
        std::fs::write(&tmp, SCRIPT.replace("@DIR@", &dir.to_string_lossy())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::rename(&tmp, &bin).unwrap();
        FakeClaude { dir, bin, queued: std::cell::Cell::new(0) }
    }

    pub fn push(&self, lines: Vec<String>) -> usize {
        let n = self.queued.get() + 1;
        self.queued.set(n);
        std::fs::write(self.dir.join(format!("out.{n}")), lines.join("\n") + "\n").unwrap();
        n
    }

    pub fn exit_code(&self, n: usize, code: i32) {
        std::fs::write(self.dir.join(format!("exit.{n}")), code.to_string()).unwrap();
    }

    pub fn skip(&self) -> usize {
        let n = self.queued.get() + 1;
        self.queued.set(n);
        n
    }

    pub fn calls(&self) -> usize {
        std::fs::read_to_string(self.dir.join("count")).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0)
    }

    pub fn argv(&self, n: usize) -> Vec<String> {
        let raw = std::fs::read(self.dir.join(format!("argv.{n}"))).unwrap_or_default();
        raw.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect()
    }

    pub fn stdin(&self, n: usize) -> String {
        std::fs::read_to_string(self.dir.join(format!("stdin.{n}"))).unwrap_or_default()
    }

    pub fn cwd(&self, n: usize) -> PathBuf {
        PathBuf::from(std::fs::read_to_string(self.dir.join(format!("cwd.{n}"))).unwrap_or_default().trim())
    }

    pub fn flag(&self, n: usize, name: &str) -> Option<String> {
        let argv = self.argv(n);
        argv.iter().position(|a| a == name).and_then(|i| argv.get(i + 1).cloned())
    }

    pub fn has(&self, n: usize, name: &str) -> bool {
        self.argv(n).iter().any(|a| a == name)
    }
}

pub fn init() -> String {
    json!({"type": "system", "subtype": "init", "session_id": "@SID@", "tools": ["Read"], "model": "fake"}).to_string()
}

pub fn tool_use(name: &str, input: Value) -> String {
    json!({"type": "assistant", "session_id": "@SID@", "message": {"role": "assistant", "content": [
        {"type": "tool_use", "id": "toolu_1", "name": name, "input": input}
    ]}})
    .to_string()
}

pub fn result(structured: Value, cost: f64, denials: Value) -> String {
    json!({"type": "result", "subtype": "success", "is_error": false, "session_id": "@SID@", "total_cost_usd": cost,
        "num_turns": 2, "result": structured.to_string(), "structured_output": structured, "permission_denials": denials,
        "usage": {"input_tokens": 100, "cache_read_input_tokens": 20, "output_tokens": 40}})
    .to_string()
}

pub fn ok(structured: Value, cost: f64) -> Vec<String> {
    vec![init(), result(structured, cost, json!([]))]
}

pub fn define_out(question: Option<&str>) -> Value {
    match question {
        Some(q) => json!({"question": q, "spec": "", "success": "", "assertions": []}),
        None => json!({"question": null, "spec": "Write greet.md containing hi", "success": "greet.md contains hi",
            "assertions": [{"id": "a1", "text": "greet.md contains hi"}]}),
    }
}

pub fn plan_out(tools: &[&str]) -> Value {
    json!({"plan": "### Inputs\nnothing yet\n\n### Outputs\ngreet.md\n\n### Steps\n1. Write greet.md with hi",
        "reversibility": "reversible", "tools": tools, "checks": [{"cmd": "test", "args": ["-f", "greet.md"]}]})
}

pub fn exec_out(status: &str, summary: &str) -> Value {
    json!({"status": status, "summary": summary, "question": null})
}

pub fn verdict_out(pass: bool) -> Value {
    json!({"items": [{"id": "a1", "verdict": if pass { "pass" } else { "fail" }, "evidence": "greet.md holds hi"}]})
}
