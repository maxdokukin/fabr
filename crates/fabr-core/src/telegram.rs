use crate::error::FabrError;
use crate::setup::{Ctx, read_secret, settings_read};
use crate::taskrec::{Answer, Approval, Grant, GuiPatch, State, TaskRecord, apply_gui, approval_digest, modify_record, parse};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LINK: &str = "Open fabr for details.";
const LATE: &str = "Already answered in the GUI.";
const LIMIT: usize = 4096;
const MSG_CAP: usize = 200;
const TOK_CAP: usize = 400;
const LIST_CAP: usize = 50;
const TIMEOUT_S: u64 = 10;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Sent {
    #[serde(default)]
    questions: Vec<String>,
    #[serde(default)]
    approvals: Vec<String>,
    #[serde(default)]
    events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Msg {
    id: i64,
    path: String,
    qid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tok {
    path: String,
    action: String,
    key: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    offset: i64,
    #[serde(default)]
    counter: u64,
    #[serde(default)]
    sent: BTreeMap<String, Sent>,
    #[serde(default)]
    msgs: Vec<Msg>,
    #[serde(default)]
    tokens: BTreeMap<u64, Tok>,
    #[serde(default)]
    follow: Option<Tok>,
}

pub struct Bot {
    base: String,
    uid: i64,
    file: PathBuf,
    st: Saved,
    agent: ureq::Agent,
}

fn now_utc() -> String {
    format!("{}Z", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S"))
}

fn push_capped(v: &mut Vec<String>, s: String) {
    v.push(s);
    if v.len() > LIST_CAP {
        v.remove(0);
    }
}

fn list(v: &[String], sep: &str) -> String {
    if v.is_empty() { "none".into() } else { v.join(sep) }
}

fn approval_text(rec: &TaskRecord, with_plan: bool) -> String {
    let m = rec.plan_meta.clone().unwrap_or_default();
    let assumed: Vec<String> = m.assumptions.iter().map(|a| a.text.clone()).collect();
    let checks: Vec<String> = rec.assertions.iter().map(|a| a.text.clone()).collect();
    let rev = m.reversibility.clone().unwrap_or_default();
    let mut t = format!("{}\n", rec.title);
    if with_plan {
        t.push_str(&format!("Plan:\n{}\n", rec.plan.trim()));
    }
    t.push_str(&format!(
        "Checks: {}\nNeeds: {}\nTools: {}\nAssumed: {}\nReversible: {rev}\n",
        list(&checks, "; "),
        list(&rec.grants.requested, ", "),
        list(&m.tools, ", "),
        list(&assumed, "; ")
    ));
    if rev == "irreversible" {
        t.push_str("This cannot be undone.\n");
    }
    t.push_str(LINK);
    t
}

fn approval_message(rec: &TaskRecord) -> String {
    let full = approval_text(rec, true);
    if full.chars().count() <= LIMIT {
        return full;
    }
    approval_text(rec, false).chars().take(LIMIT).collect()
}

fn applicable(rec: &TaskRecord, action: &str, key: &str) -> bool {
    match action {
        "approve" | "refuse" => {
            rec.approval.as_ref().is_some_and(|a| a.status == "pending" && a.digest.clone().unwrap_or_else(|| approval_digest(rec)) == key)
        }
        _ => rec.question.as_ref().is_some_and(|q| q.id == key && q.answer.is_none() && (action != "answer" || q.kind == "open")),
    }
}

fn patch(rec: &TaskRecord, action: &str, text: Option<&str>, now: &str) -> GuiPatch {
    let by = Some("telegram".to_string());
    let at = Some(now.to_string());
    let status = |s: &str, note: Option<String>| Approval {
        status: s.into(),
        by: by.clone(),
        at: at.clone(),
        note,
        digest: rec.approval.as_ref().and_then(|a| a.digest.clone()),
    };
    let answer = |value: Option<&str>, replacement: Option<&str>, granted: Option<Vec<Grant>>| {
        Some(Answer { value: value.map(String::from), replacement: replacement.map(String::from), granted, by: by.clone(), at: at.clone() })
    };
    let mut p = GuiPatch::default();
    match action {
        "approve" => p.approval = Some(status("approved", None)),
        "refuse" => p.approval = Some(status("refused", text.map(String::from))),
        "once" | "always" => p.answer = answer(Some(action), None, None),
        "no" => p.answer = answer(Some("no"), text, None),
        "allow" => {
            let fields = rec.question.as_ref().and_then(|q| q.fields.clone()).unwrap_or_default();
            let granted = fields.into_iter().map(|field| Grant { field, scope: "task".into() }).collect();
            p.answer = answer(None, None, Some(granted));
        }
        _ => p.answer = answer(text, None, None),
    }
    p
}

impl Bot {
    pub fn from_ctx(ctx: &Ctx) -> Option<Bot> {
        let token = read_secret(ctx, "telegram_bot_token")?;
        let uid = settings_read(ctx).ok()?.settings.telegram_user_id?;
        let file = ctx.space().join("telegram.yaml");
        let st = std::fs::read_to_string(&file).ok().and_then(|t| serde_yaml::from_str(&t).ok()).unwrap_or_default();
        let base = format!("{}/bot{token}", ctx.telegram_url.trim_end_matches('/'));
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(TIMEOUT_S)).build();
        Some(Bot { base, uid, file, st, agent })
    }

    fn call(&self, method: &str, body: &Value) -> Result<Value, FabrError> {
        match self.agent.post(&format!("{}/{method}", self.base)).send_json(body) {
            Ok(r) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                if v["ok"].as_bool() == Some(true) { Ok(v) } else { Err(FabrError::Remote(format!("telegram {method} was refused"))) }
            }
            Err(ureq::Error::Status(c, _)) => Err(FabrError::Remote(format!("telegram {method} answered HTTP {c}"))),
            Err(_) => Err(FabrError::Timeout(format!("telegram {method} did not answer"))),
        }
    }

    fn save(&self) -> Result<(), FabrError> {
        let parent = self.file.parent().ok_or_else(|| FabrError::Io("bad path".into()))?;
        std::fs::create_dir_all(parent)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        tmp.write_all(serde_yaml::to_string(&self.st).unwrap_or_default().as_bytes())?;
        tmp.persist(&self.file).map_err(|e| FabrError::Io(e.to_string()))?;
        Ok(())
    }

    fn token(&mut self, path: &str, action: &str, key: &str) -> u64 {
        self.st.counter += 1;
        let n = self.st.counter;
        self.st.tokens.insert(n, Tok { path: path.into(), action: action.into(), key: key.into() });
        while self.st.tokens.len() > TOK_CAP {
            self.st.tokens.pop_first();
        }
        n
    }

    fn keyboard(&mut self, path: &str, key: &str, row: &[(&str, &str)]) -> Value {
        let buttons: Vec<Value> = row
            .iter()
            .map(|(label, action)| json!({ "text": label, "callback_data": self.token(path, action, key).to_string() }))
            .collect();
        json!({ "inline_keyboard": [buttons] })
    }

    fn send(&self, text: &str, markup: Option<Value>) -> Result<i64, FabrError> {
        let mut body = json!({ "chat_id": self.uid, "text": text });
        if let Some(m) = markup {
            body["reply_markup"] = m;
        }
        let v = self.call("sendMessage", &body)?;
        Ok(v["result"]["message_id"].as_i64().unwrap_or(0))
    }

    fn remember(&mut self, id: i64, path: &str, qid: &str) {
        self.st.msgs.push(Msg { id, path: path.into(), qid: qid.into() });
        while self.st.msgs.len() > MSG_CAP {
            self.st.msgs.remove(0);
        }
    }

    pub fn sync_record(&mut self, record_path: &Path, rec: &TaskRecord) -> Result<(), FabrError> {
        let path = record_path.to_string_lossy().into_owned();
        let seen =
            |st: &Saved, pick: fn(&Sent) -> &Vec<String>, k: &str| st.sent.get(&path).is_some_and(|s| pick(s).iter().any(|x| x == k));
        if let Some(q) = &rec.question
            && q.answer.is_none()
            && !seen(&self.st, |s| &s.questions, &q.id)
        {
            let (text, markup) = match q.kind.as_str() {
                "open" => (format!("{}\nQuestion: {}\nReply to this message with your answer.\n{LINK}", rec.title, q.text), None),
                "choice" => {
                    let last = q.prior.as_ref().map(|p| format!("Last time: {}\n", p.text)).unwrap_or_default();
                    let kb = self.keyboard(&path, &q.id, &[("Once", "once"), ("Always", "always"), ("No, something else", "no")]);
                    (format!("{}\n{}\n{last}{LINK}", rec.title, q.text), Some(kb))
                }
                "grant" => {
                    let fields = list(q.fields.as_deref().unwrap_or_default(), ", ");
                    let kb = self.keyboard(&path, &q.id, &[("Allow for this task", "allow"), ("Review in GUI", "open")]);
                    (format!("{}\n{}\nFields: {fields}\n{LINK}", rec.title, q.text), Some(kb))
                }
                _ => return Ok(()),
            };
            let id = self.send(&text, markup)?;
            self.remember(id, &path, &q.id);
            push_capped(&mut self.st.sent.entry(path.clone()).or_default().questions, q.id.clone());
            self.save()?;
        }
        if rec.state == State::Defined
            && let Some(a) = &rec.approval
            && a.status == "pending"
        {
            let digest = a.digest.clone().unwrap_or_else(|| approval_digest(rec));
            if !seen(&self.st, |s| &s.approvals, &digest) {
                let kb = self.keyboard(&path, &digest, &[("Approve", "approve"), ("Refuse", "refuse"), ("Open in GUI", "open")]);
                let id = self.send(&approval_message(rec), Some(kb))?;
                self.remember(id, &path, "");
                push_capped(&mut self.st.sent.entry(path).or_default().approvals, digest);
                self.save()?;
            }
        }
        Ok(())
    }

    pub fn notify_event(&mut self, kind: &str, record_path: &Path, title: &str, text: &str) {
        let label = match kind {
            "done" => "Done",
            "failed" => "Failed",
            "stuck" => "Stuck",
            _ => return,
        };
        let path = record_path.to_string_lossy().into_owned();
        let key = format!("{kind} {text}");
        if self.st.sent.get(&path).is_some_and(|s| s.events.contains(&key)) {
            return;
        }
        let kb = self.keyboard(&path, "", &[("Open in GUI", "open")]);
        match self.send(&format!("{title}\n{label}: {text}\n{LINK}"), Some(kb)) {
            Ok(_) => {
                push_capped(&mut self.st.sent.entry(path).or_default().events, key);
                if let Err(e) = self.save() {
                    log::warn!("telegram state: {}", e.message());
                }
            }
            Err(e) => log::warn!("telegram notice: {}", e.message()),
        }
    }

    fn commit(&self, path: &str, action: &str, key: &str, text: Option<&str>) -> Result<bool, FabrError> {
        let p = Path::new(path);
        let rec = parse(&std::fs::read_to_string(p).map_err(|e| FabrError::Io(format!("{path}: {e}")))?)?;
        if !applicable(&rec, action, key) {
            return Ok(false);
        }
        let now = now_utc();
        let (_, ok) = modify_record(p, &now, |r| {
            let ok = applicable(r, action, key);
            if ok {
                apply_gui(r, &patch(r, action, text, &now));
            }
            ok
        })?;
        Ok(ok)
    }

    fn tap(&mut self, cb: &Value) -> Result<(bool, Option<String>), FabrError> {
        let n = cb["data"].as_str().and_then(|d| d.parse::<u64>().ok());
        let Some(tok) = n.and_then(|n| self.st.tokens.get(&n).cloned()) else { return Ok((false, None)) };
        match tok.action.as_str() {
            "open" => Ok((false, Some(LINK.into()))),
            "refuse" | "no" => {
                let rec = parse(&std::fs::read_to_string(&tok.path).map_err(|e| FabrError::Io(format!("{}: {e}", tok.path)))?)?;
                if !applicable(&rec, &tok.action, &tok.key) {
                    self.send(LATE, None)?;
                    return Ok((false, Some(LATE.into())));
                }
                let prompt = if tok.action == "refuse" {
                    "Why refuse? Your note goes into the log.".to_string()
                } else {
                    let q = rec.question.as_ref();
                    let prior = q.map(|q| q.prior.as_ref().map_or(q.text.clone(), |p| p.text.clone()));
                    format!("What should I do instead of: {}?", prior.unwrap_or_default())
                };
                self.st.follow = Some(tok);
                self.save()?;
                self.send(&prompt, None)?;
                Ok((false, None))
            }
            _ => {
                if self.commit(&tok.path, &tok.action, &tok.key, None)? {
                    Ok((true, None))
                } else {
                    self.send(LATE, None)?;
                    Ok((false, Some(LATE.into())))
                }
            }
        }
    }

    fn text(&mut self, m: &Value) -> Result<bool, FabrError> {
        let Some(text) = m["text"].as_str() else { return Ok(false) };
        let target = match self.st.follow.take() {
            Some(f) => {
                self.save()?;
                Some((f.path, f.action, f.key))
            }
            None => {
                let id = m["reply_to_message"]["message_id"].as_i64();
                id.and_then(|id| self.st.msgs.iter().find(|x| x.id == id && !x.qid.is_empty()))
                    .map(|x| (x.path.clone(), "answer".to_string(), x.qid.clone()))
            }
        };
        let Some((path, action, key)) = target else { return Ok(false) };
        if self.commit(&path, &action, &key, Some(text))? {
            Ok(true)
        } else {
            self.send(LATE, None)?;
            Ok(false)
        }
    }

    pub fn poll(&mut self) -> Result<usize, FabrError> {
        let body = json!({ "offset": self.st.offset, "timeout": 0, "allowed_updates": ["message", "callback_query"] });
        let v = self.call("getUpdates", &body)?;
        let mut applied = 0;
        for u in v["result"].as_array().cloned().unwrap_or_default() {
            self.st.offset = self.st.offset.max(u["update_id"].as_i64().unwrap_or(0) + 1);
            let cb = &u["callback_query"];
            let res = if cb.is_object() {
                if cb["from"]["id"].as_i64() != Some(self.uid) {
                    continue;
                }
                let r = self.tap(cb);
                let mut ack = json!({ "callback_query_id": cb["id"] });
                if let Ok((_, Some(t))) = &r {
                    ack["text"] = json!(t);
                }
                let _ = self.call("answerCallbackQuery", &ack);
                r.map(|(ok, _)| ok)
            } else if u["message"]["from"]["id"].as_i64() == Some(self.uid) {
                self.text(&u["message"])
            } else {
                continue;
            };
            match res {
                Ok(true) => applied += 1,
                Ok(false) => {}
                Err(e) => log::warn!("telegram update: {}", e.message()),
            }
        }
        self.save()?;
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::{FabricSettings, SecretsWriteReq, SettingsWriteReq, secrets_write, settings_write};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    const UID: i64 = 7;

    struct Fake {
        url: String,
        log: Arc<Mutex<Vec<(String, Value)>>>,
        queue: Arc<Mutex<VecDeque<Value>>>,
        stop: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn fake() -> Fake {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let log: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let queue: Arc<Mutex<VecDeque<Value>>> = Arc::default();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (l, q, s) = (log.clone(), queue.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut next = 100;
            while !s.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok(Some(mut rq)) = server.recv_timeout(Duration::from_millis(50)) else { continue };
                let mut text = String::new();
                let _ = std::io::Read::read_to_string(rq.as_reader(), &mut text);
                let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let method = rq.url().rsplit('/').next().unwrap_or("").to_string();
                l.lock().unwrap().push((method.clone(), body));
                let result = match method.as_str() {
                    "getUpdates" => json!(q.lock().unwrap().pop_front().map(|u| vec![u]).unwrap_or_default()),
                    "sendMessage" => {
                        next += 1;
                        json!({ "message_id": next })
                    }
                    _ => json!(true),
                };
                let resp = tiny_http::Response::from_string(json!({ "ok": true, "result": result }).to_string())
                    .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap());
                let _ = rq.respond(resp);
            }
        });
        Fake { url, log, queue, stop }
    }

    impl Fake {
        fn push(&self, u: Value) {
            self.queue.lock().unwrap().push_back(u);
        }
        fn sent(&self) -> Vec<Value> {
            self.log.lock().unwrap().iter().filter(|(m, _)| m == "sendMessage").map(|(_, b)| b.clone()).collect()
        }
        fn count(&self, method: &str) -> usize {
            self.log.lock().unwrap().iter().filter(|(m, _)| m == method).count()
        }
        fn button(&self, msg: usize, label: &str) -> String {
            let m = &self.sent()[msg];
            let row = m["reply_markup"]["inline_keyboard"][0].as_array().unwrap().clone();
            row.iter().find(|b| b["text"] == label).unwrap()["callback_data"].as_str().unwrap().to_string()
        }
    }

    fn ctx(f: &Fake) -> (tempfile::TempDir, Ctx) {
        let d = tempfile::tempdir().unwrap();
        let mut c = Ctx::new(d.path());
        c.telegram_url = f.url.clone();
        secrets_write(&c, SecretsWriteReq { telegram_bot_token: Some("123456:FAKE-TOKEN".into()) }).unwrap();
        let mut s = FabricSettings::defaults(d.path());
        s.telegram_user_id = Some(UID);
        settings_write(&c, SettingsWriteReq { settings: s }).unwrap();
        (d, c)
    }

    fn record(question: &str, approval: &str, state: &str) -> String {
        format!(
            "---\nschema: 7\nid: t1\ntitle: Order PLA\nstate: {state}\nrequests: {{delegate: false, cancel: false, redelegate: false}}\n\
             question: {question}\ngrants: {{requested: [name.first], granted: []}}\n\
             plan_meta: {{reversibility: irreversible, tools: [browser], assumptions: []}}\n\
             assertions: [{{id: a1, text: Cart is full}}]\napproval: {approval}\nlinks: {{}}\n---\n## Plan\nBuy it\n"
        )
    }

    fn open_q() -> &'static str {
        "{id: q1, kind: open, text: Which shop?, answer: null}"
    }

    fn setup(text: &str) -> (tempfile::TempDir, PathBuf, TaskRecord) {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("p/.fabric-space/tasks");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t1.md");
        std::fs::write(&p, text).unwrap();
        let rec = parse(text).unwrap();
        (d, p, rec)
    }

    fn reload(p: &Path) -> TaskRecord {
        parse(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn tap(id: i64, uid: i64, data: &str) -> Value {
        json!({ "update_id": id, "callback_query": { "id": "cb", "from": { "id": uid }, "data": data } })
    }

    fn say(id: i64, uid: i64, text: &str, reply: Option<i64>) -> Value {
        let mut m = json!({ "from": { "id": uid }, "chat": { "id": uid }, "text": text });
        if let Some(r) = reply {
            m["reply_to_message"] = json!({ "message_id": r });
        }
        json!({ "update_id": id, "message": m })
    }

    #[test]
    fn question_is_sent_once_across_syncs_and_restarts() {
        let f = fake();
        let (_d, c) = ctx(&f);
        let (_t, p, rec) = setup(&record(open_q(), "null", "task"));
        let mut bot = Bot::from_ctx(&c).unwrap();
        bot.sync_record(&p, &rec).unwrap();
        bot.sync_record(&p, &rec).unwrap();
        let mut again = Bot::from_ctx(&c).unwrap();
        again.sync_record(&p, &rec).unwrap();
        let sent = f.sent();
        assert_eq!(sent.len(), 1);
        let t = sent[0]["text"].as_str().unwrap();
        assert!(t.starts_with("Order PLA\nQuestion: Which shop?\nReply to this message") && t.ends_with("Open fabr for details."));
        assert_eq!(sent[0]["chat_id"], UID);
        assert!(c.space().join("telegram.yaml").exists());
    }

    #[test]
    fn approval_has_three_buttons_and_approve_writes_the_record() {
        let f = fake();
        let (_d, c) = ctx(&f);
        let (_t, p, rec) = setup(&record("null", "{status: pending, digest: abc}", "defined"));
        let mut bot = Bot::from_ctx(&c).unwrap();
        bot.sync_record(&p, &rec).unwrap();
        bot.sync_record(&p, &rec).unwrap();
        assert_eq!(f.sent().len(), 1);
        let m = &f.sent()[0];
        let t = m["text"].as_str().unwrap();
        assert!(t.contains("Plan:\nBuy it") && t.contains("Reversible: irreversible\nThis cannot be undone.\n"));
        let row = m["reply_markup"]["inline_keyboard"][0].as_array().unwrap();
        let labels: Vec<&str> = row.iter().map(|b| b["text"].as_str().unwrap()).collect();
        assert_eq!(labels, ["Approve", "Refuse", "Open in GUI"]);
        assert!(row.iter().all(|b| b["callback_data"].as_str().unwrap().len() <= 64));
        f.push(tap(1, UID, &f.button(0, "Approve")));
        assert_eq!(bot.poll().unwrap(), 1);
        let a = reload(&p).approval.unwrap();
        assert_eq!((a.status.as_str(), a.by.as_deref(), a.digest.as_deref()), ("approved", Some("telegram"), Some("abc")));
        assert!(a.at.is_some());
        assert_eq!(f.count("answerCallbackQuery"), 1);
    }

    #[test]
    fn long_plan_is_dropped_from_the_approval() {
        let f = fake();
        let (_d, c) = ctx(&f);
        let text = record("null", "{status: pending, digest: abc}", "defined").replace("Buy it", &"x".repeat(5000));
        let (_t, p, rec) = setup(&text);
        Bot::from_ctx(&c).unwrap().sync_record(&p, &rec).unwrap();
        let t = f.sent()[0]["text"].as_str().unwrap().to_string();
        assert!(!t.contains("Plan:") && t.contains("Checks: Cart is full"));
    }

    #[test]
    fn refuse_asks_for_a_note_and_the_next_text_lands_in_it() {
        let f = fake();
        let (_d, c) = ctx(&f);
        let (_t, p, rec) = setup(&record("null", "{status: pending, digest: abc}", "defined"));
        let mut bot = Bot::from_ctx(&c).unwrap();
        bot.sync_record(&p, &rec).unwrap();
        f.push(tap(1, UID, &f.button(0, "Refuse")));
        assert_eq!(bot.poll().unwrap(), 0);
        assert_eq!(f.sent()[1]["text"], "Why refuse? Your note goes into the log.");
        assert_eq!(reload(&p).approval.unwrap().status, "pending");
        let mut bot = Bot::from_ctx(&c).unwrap();
        f.push(say(2, UID, "too pricey", None));
        assert_eq!(bot.poll().unwrap(), 1);
        let a = reload(&p).approval.unwrap();
        assert_eq!((a.status.as_str(), a.note.as_deref(), a.by.as_deref()), ("refused", Some("too pricey"), Some("telegram")));
    }

    #[test]
    fn text_reply_answers_an_open_question() {
        let f = fake();
        let (_d, c) = ctx(&f);
        let (_t, p, rec) = setup(&record(open_q(), "null", "task"));
        let mut bot = Bot::from_ctx(&c).unwrap();
        bot.sync_record(&p, &rec).unwrap();
        f.push(say(1, UID, "no reply target", None));
        f.push(say(2, UID, "shop.test", Some(101)));
        assert_eq!(bot.poll().unwrap(), 0);
        assert_eq!(bot.poll().unwrap(), 1);
        let a = reload(&p).question.unwrap().answer.unwrap();
        assert_eq!((a.value.as_deref(), a.by.as_deref()), (Some("shop.test"), Some("telegram")));
    }

    #[test]
    fn choice_and_grant_buttons_write_their_fields() {
        let f = fake();
        let (_d, c) = ctx(&f);
        let choice = "{id: q2, kind: choice, text: Ship home?, options: [once, always, 'no'], prior: {text: Home}, answer: null}";
        let (_t, p, rec) = setup(&record(choice, "null", "task"));
        let mut bot = Bot::from_ctx(&c).unwrap();
        bot.sync_record(&p, &rec).unwrap();
        assert!(f.sent()[0]["text"].as_str().unwrap().contains("Last time: Home"));
        f.push(tap(1, UID, &f.button(0, "No, something else")));
        f.push(say(2, UID, "ship to office", None));
        assert_eq!(bot.poll().unwrap() + bot.poll().unwrap(), 1);
        assert_eq!(f.sent()[1]["text"], "What should I do instead of: Home?");
        let a = reload(&p).question.unwrap().answer.unwrap();
        assert_eq!((a.value.as_deref(), a.replacement.as_deref()), (Some("no"), Some("ship to office")));

        let grant = "{id: q3, kind: grant, text: Need name, fields: [name.first, name.last], answer: null}";
        let (_t2, p2, rec2) = setup(&record(grant, "null", "task"));
        bot.sync_record(&p2, &rec2).unwrap();
        f.push(tap(3, UID, &f.button(2, "Allow for this task")));
        assert_eq!(bot.poll().unwrap(), 1);
        let g = reload(&p2).question.unwrap().answer.unwrap().granted.unwrap();
        assert_eq!(g.len(), 2);
        assert!(g.iter().all(|x| x.scope == "task"));
    }

    #[test]
    fn other_users_are_ignored() {
        let f = fake();
        let (_d, c) = ctx(&f);
        let (_t, p, rec) = setup(&record(open_q(), "null", "task"));
        let mut bot = Bot::from_ctx(&c).unwrap();
        bot.sync_record(&p, &rec).unwrap();
        f.push(say(1, 99, "evil", Some(101)));
        f.push(tap(2, 99, "1"));
        assert_eq!(bot.poll().unwrap(), 0);
        assert_eq!(bot.poll().unwrap(), 0);
        assert!(reload(&p).question.unwrap().answer.is_none());
        assert_eq!(f.sent().len(), 1);
        assert_eq!(f.count("answerCallbackQuery"), 0);
    }

    #[test]
    fn already_answered_gets_the_gui_notice() {
        let f = fake();
        let (_d, c) = ctx(&f);
        let (_t, p, rec) = setup(&record(open_q(), "null", "task"));
        let mut bot = Bot::from_ctx(&c).unwrap();
        bot.sync_record(&p, &rec).unwrap();
        let answered = open_q().replace("answer: null", "answer: {value: gui, by: gui}");
        std::fs::write(&p, record(&answered, "null", "task")).unwrap();
        f.push(say(1, UID, "late", Some(101)));
        assert_eq!(bot.poll().unwrap(), 0);
        assert_eq!(f.sent()[1]["text"], "Already answered in the GUI.");
        assert_eq!(reload(&p).question.unwrap().answer.unwrap().value.as_deref(), Some("gui"));
    }

    #[test]
    fn notify_event_sends_once() {
        let f = fake();
        let (_d, c) = ctx(&f);
        let p = PathBuf::from("/x/.fabric-space/tasks/t1.md");
        let mut bot = Bot::from_ctx(&c).unwrap();
        bot.notify_event("done", &p, "Order PLA", "in the cart");
        bot.notify_event("done", &p, "Order PLA", "in the cart");
        bot.notify_event("waiting", &p, "Order PLA", "q");
        Bot::from_ctx(&c).unwrap().notify_event("done", &p, "Order PLA", "in the cart");
        let sent = f.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["text"], "Order PLA\nDone: in the cart\nOpen fabr for details.");
    }
}
