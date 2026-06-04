use std::io::Write;

use anyhow::Result;
use chrono::{DateTime, Utc};
use hydra_common::{
    triggers::{
        Action, CreateIssueAction, Schedule, ScheduleFiring, Trigger, TriggerVersionRecord,
        UpsertTriggerResponse,
    },
    TriggerId,
};
use serde_json::json;

use super::Render;
use crate::command::triggers::RenderedAction;

pub struct TriggerRecords<'a>(pub &'a [TriggerVersionRecord]);

pub struct DeletedTriggerOutcome<'a>(pub &'a TriggerId);

impl Render for DeletedTriggerOutcome<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(
            &mut *writer,
            &json!({ "trigger_id": self.0, "action": "deleted" }),
        )?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(writer, "Deleted trigger '{}'", self.0)?;
        writer.flush()?;
        Ok(())
    }
}

pub struct TriggerUpsertOutcome<'a>(pub(crate) &'a UpsertTriggerResponse);

impl Render for TriggerUpsertOutcome<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self.0)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(
            writer,
            "Trigger {} (version {})",
            self.0.trigger_id, self.0.version
        )?;
        writer.flush()?;
        Ok(())
    }
}

pub struct TriggerTestRecords<'a>(pub(crate) &'a [RenderedAction]);

impl Render for TriggerTestRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for action in self.0 {
            serde_json::to_writer(&mut *writer, action)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.0.is_empty() {
            writeln!(writer, "(trigger has no actions)")?;
        }
        for action in self.0 {
            render_test_action(writer, action)?;
        }
        writer.flush()?;
        Ok(())
    }
}

fn render_test_action<W: Write>(writer: &mut W, action: &RenderedAction) -> Result<()> {
    match action {
        RenderedAction::CreateIssue(rendered) => {
            writeln!(
                writer,
                "action {} create_issue ({})",
                rendered.action_index, rendered.issue_type
            )?;
            writeln!(writer, "  title: {}", rendered.title)?;
            if rendered.description.trim().is_empty() {
                writeln!(writer, "  description: -")?;
            } else {
                writeln!(writer, "  description:")?;
                for line in rendered.description.lines() {
                    writeln!(writer, "    {line}")?;
                }
            }
            if let Some(assignee) = &rendered.assignee {
                writeln!(writer, "  assignee: {assignee}")?;
            }
            if let Some(status) = &rendered.status {
                writeln!(writer, "  status: {status}")?;
            }
            if let Some(repo) = &rendered.repo_name {
                writeln!(writer, "  repo: {repo}")?;
            }
        }
    }
    Ok(())
}

impl Render for TriggerRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for trigger in self.0 {
            serde_json::to_writer(&mut *writer, trigger)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        let now = Utc::now();
        for (index, record) in self.0.iter().enumerate() {
            render_one(writer, record, now)?;
            if index + 1 < self.0.len() {
                writeln!(writer)?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

fn render_one<W: Write>(
    writer: &mut W,
    record: &TriggerVersionRecord,
    now: DateTime<Utc>,
) -> Result<()> {
    let Trigger {
        enabled,
        schedule,
        actions,
        creator,
        last_fired_at,
        deleted,
        ..
    } = &record.trigger;

    let status = if *deleted {
        "deleted"
    } else if *enabled {
        "enabled"
    } else {
        "disabled"
    };
    writeln!(
        writer,
        "Trigger {} (v{}, {status})",
        record.trigger_id, record.version
    )?;
    writeln!(writer, "Creator: {}", creator.as_ref())?;
    writeln!(writer, "Schedule: {}", format_schedule(schedule))?;
    writeln!(
        writer,
        "Last fired: {}",
        last_fired_at
            .as_ref()
            .map(|ts| ts.to_rfc3339())
            .unwrap_or_else(|| "never".to_string())
    )?;
    writeln!(
        writer,
        "Next fire: {}",
        schedule
            .next_fire_after(now)
            .map(|ts| ts.to_rfc3339())
            .unwrap_or_else(|| "n/a".to_string())
    )?;
    if actions.is_empty() {
        writeln!(writer, "Actions: none")?;
    } else {
        writeln!(writer, "Actions:")?;
        for (idx, action) in actions.iter().enumerate() {
            render_action(writer, idx, action)?;
        }
    }
    Ok(())
}

fn format_schedule(schedule: &Schedule) -> String {
    match schedule {
        Schedule::Cron {
            expression,
            timezone,
        } => match timezone.as_deref() {
            Some(tz) => format!("cron \"{expression}\" ({tz})"),
            None => format!("cron \"{expression}\" (UTC)"),
        },
        Schedule::Once { at } => format!("once at {}", at.to_rfc3339()),
    }
}

fn render_action<W: Write>(writer: &mut W, idx: usize, action: &Action) -> Result<()> {
    match action {
        Action::CreateIssue(create) => {
            let CreateIssueAction {
                issue_type,
                title,
                description,
                assignee,
                status,
                session_settings,
                ..
            } = create;
            writeln!(writer, "  {idx}. create_issue ({issue_type})")?;
            writeln!(writer, "     title: {title}")?;
            if !description.trim().is_empty() {
                writeln!(writer, "     description:")?;
                for line in description.lines() {
                    writeln!(writer, "       {line}")?;
                }
            }
            if let Some(assignee) = assignee.as_deref() {
                writeln!(writer, "     assignee: {assignee}")?;
            }
            if let Some(status) = status {
                writeln!(writer, "     status: {status}")?;
            }
            if let Some(repo) = session_settings.repo_name.as_ref() {
                writeln!(writer, "     repo: {repo}")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::output::{render, ResolvedOutputFormat},
        test_utils::ids::trigger_id,
    };
    use serde_json::json;

    #[test]
    fn deleted_trigger_pretty_matches_legacy_wording() {
        let id = trigger_id("t-stale");
        let mut output = Vec::new();
        render(
            DeletedTriggerOutcome(&id),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .expect("render");
        let output = String::from_utf8(output).expect("utf8");
        assert_eq!(output, format!("Deleted trigger '{id}'\n"));
    }

    #[test]
    fn deleted_trigger_jsonl_emits_structured_record() {
        let id = trigger_id("t-stale");
        let mut output = Vec::new();
        render(
            DeletedTriggerOutcome(&id),
            ResolvedOutputFormat::Jsonl,
            &mut output,
        )
        .expect("render");
        let output = String::from_utf8(output).expect("utf8");
        assert_eq!(output.lines().count(), 1);
        let parsed: serde_json::Value = serde_json::from_str(output.trim_end()).expect("json");
        assert_eq!(parsed, json!({ "trigger_id": id, "action": "deleted" }));
    }
}
