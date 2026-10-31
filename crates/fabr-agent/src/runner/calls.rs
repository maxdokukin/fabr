use crate::engine::Usage;
use crate::now_utc;
use fabr_core::redact::redact;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepUsage {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub usd: f64,
}

#[derive(Clone, Default)]
pub struct Sink {
    captured: Option<Arc<Mutex<Vec<String>>>>,
}

impl Sink {
    pub fn capture() -> (Sink, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        (Sink { captured: Some(lines.clone()) }, lines)
    }

    fn keep(&self, line: &str) {
        if let Some(lines) = &self.captured {
            lines.lock().unwrap_or_else(PoisonError::into_inner).push(line.to_string());
        }
    }

    pub fn info(&self, msg: impl AsRef<str>) {
        let line = redact(msg.as_ref());
        log::info!("{line}");
        self.keep(&line);
    }

    pub fn warn(&self, msg: impl AsRef<str>) {
        let line = redact(msg.as_ref());
        log::warn!("{line}");
        self.keep(&line);
    }
}

pub struct CallLog {
    sink: Sink,
    model: String,
    state: Mutex<HashMap<String, BTreeMap<String, StepUsage>>>,
}

impl CallLog {
    pub fn new(sink: Sink, model: String) -> Arc<CallLog> {
        Arc::new(CallLog { sink, model, state: Mutex::new(HashMap::new()) })
    }

    pub fn usage(&self, record_id: &str) -> BTreeMap<String, StepUsage> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).get(record_id).cloned().unwrap_or_default()
    }

    pub fn forget(&self, record_id: &str) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).remove(record_id);
    }

    pub fn record(&self, record_id: &str, step: &str, session: Option<&str>, usage: &Usage, usd: f64) {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let slot = state.entry(record_id.to_string()).or_default().entry(step.to_string()).or_default();
            slot.calls += 1;
            slot.input_tokens += usage.input_tokens;
            slot.output_tokens += usage.output_tokens;
            slot.usd += usd;
        }
        let model = if self.model.is_empty() { "default" } else { self.model.as_str() };
        self.sink.info(format!(
            "claude run at={} record={record_id} step={step} session={} model={model} turns={} in={} out={} cost_usd={usd:.6}",
            now_utc(),
            session.unwrap_or("-"),
            usage.turns,
            usage.input_tokens,
            usage.output_tokens
        ));
    }
}
