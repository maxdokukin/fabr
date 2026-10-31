use crate::now_utc;
use fabr_core::FabrError;
use fabr_core::taskrec::{Author, TaskRecord, modify_record};
use std::path::Path;

fn take<T: PartialEq + Clone>(disk: &mut T, base: &T, mine: &T) {
    if mine != base {
        *disk = mine.clone();
    }
}

fn take_section(disk: &mut String, disk_by: &mut Option<Author>, base: (&str, Option<Author>), mine: (&String, Option<Author>)) {
    let human_appeared = *disk_by == Some(Author::Human) && base.1 != Some(Author::Human);
    if human_appeared {
        return;
    }
    if mine.0 != base.0 {
        *disk = mine.0.clone();
    }
    if mine.1 != base.1 {
        *disk_by = mine.1;
    }
}

pub fn merge(disk: &mut TaskRecord, base: &TaskRecord, mine: &TaskRecord) {
    take(&mut disk.source, &base.source, &mine.source);
    take(&mut disk.title, &base.title, &mine.title);
    take(&mut disk.due, &base.due, &mine.due);
    take(&mut disk.pri, &base.pri, &mine.pri);
    take(&mut disk.state, &base.state, &mine.state);
    take(&mut disk.step, &base.step, &mine.step);
    take(&mut disk.waiting, &base.waiting, &mine.waiting);
    take(&mut disk.paused, &base.paused, &mine.paused);
    take(&mut disk.requests.delegate, &base.requests.delegate, &mine.requests.delegate);
    take(&mut disk.requests.cancel, &base.requests.cancel, &mine.requests.cancel);
    take(&mut disk.requests.complete, &base.requests.complete, &mine.requests.complete);
    take(&mut disk.requests.redelegate, &base.requests.redelegate, &mine.requests.redelegate);
    take(&mut disk.grants.requested, &base.grants.requested, &mine.grants.requested);
    take(&mut disk.grants.granted, &base.grants.granted, &mine.grants.granted);
    take(&mut disk.plan_meta, &base.plan_meta, &mine.plan_meta);
    take(&mut disk.assertions, &base.assertions, &mine.assertions);
    take(&mut disk.approval, &base.approval, &mine.approval);
    take(&mut disk.verdict, &base.verdict, &mine.verdict);
    take(&mut disk.cost_usd, &base.cost_usd, &mine.cost_usd);
    take(&mut disk.links, &base.links, &mine.links);
    take(&mut disk.sessions, &base.sessions, &mine.sessions);
    take_section(&mut disk.context, &mut disk.by.context, (&base.context, base.by.context), (&mine.context, mine.by.context));
    take_section(&mut disk.spec, &mut disk.by.spec, (&base.spec, base.by.spec), (&mine.spec, mine.by.spec));
    take_section(&mut disk.success, &mut disk.by.success, (&base.success, base.by.success), (&mine.success, mine.by.success));
    take_section(&mut disk.plan, &mut disk.by.plan, (&base.plan, base.by.plan), (&mine.plan, mine.by.plan));
    take(&mut disk.needs, &base.needs, &mine.needs);
    take(&mut disk.requests.redo, &base.requests.redo, &mine.requests.redo);
    if mine.question != base.question {
        let answered_meanwhile = mine.question.is_none()
            && disk.question.as_ref().is_some_and(|q| q.answer.is_some())
            && base.question.as_ref().is_none_or(|q| q.answer.is_none());
        if !answered_meanwhile {
            disk.question = mine.question.clone();
        }
    }
    if mine.answers != base.answers {
        for a in &mine.answers {
            if !disk.answers.iter().any(|d| d.id == a.id) {
                disk.answers.push(a.clone());
            }
        }
    }
    if mine.log != base.log {
        match mine.log.strip_prefix(base.log.as_str()) {
            Some(suffix) => {
                if !disk.log.is_empty() && !disk.log.ends_with('\n') && !suffix.starts_with('\n') {
                    disk.log.push('\n');
                }
                disk.log.push_str(suffix);
            }
            None => disk.log = mine.log.clone(),
        }
    }
}

pub fn update(path: &Path, mine: &mut TaskRecord, base: &mut TaskRecord) -> Result<(), FabrError> {
    let (merged, ()) = modify_record(path, &now_utc(), |disk| merge(disk, base, mine))?;
    let merged = fabr_core::taskrec::parse(&fabr_core::taskrec::render(&merged))?;
    *base = merged.clone();
    *mine = merged;
    Ok(())
}

pub fn mutate<T>(path: &Path, f: impl FnOnce(&mut TaskRecord) -> T) -> Result<(TaskRecord, T), FabrError> {
    modify_record(path, &now_utc(), f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_human_section_on_disk_beats_a_stale_pipeline_copy_but_an_explicit_redo_clears_it() {
        let base = crate::link::test_reminder("t");
        let mut disk = base.clone();
        disk.spec = "typed by a person".into();
        disk.by.spec = Some(Author::Human);
        let mut mine = base.clone();
        mine.spec = "written by the agent".into();
        mine.by.spec = Some(Author::Agent);
        mine.needs = Some("harness: write_file".into());
        merge(&mut disk, &base, &mine);
        assert_eq!((disk.spec.as_str(), disk.by.spec), ("typed by a person", Some(Author::Human)));
        assert_eq!(disk.needs.as_deref(), Some("harness: write_file"));
        let mut known = base.clone();
        known.plan = "human plan".into();
        known.by.plan = Some(Author::Human);
        let mut disk = known.clone();
        let mut cleared = known.clone();
        cleared.plan.clear();
        cleared.by.plan = None;
        merge(&mut disk, &known, &cleared);
        assert_eq!((disk.plan.as_str(), disk.by.plan), ("", None));
    }
}
