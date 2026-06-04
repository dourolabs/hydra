use std::io::Write;

use anyhow::Result;
use chrono::{DateTime, Utc};
use hydra_common::triggers::{
    Action, CreateIssueAction, Schedule, ScheduleFiring, Trigger, TriggerVersionRecord,
};

use super::Render;

pub struct TriggerRecords<'a>(pub &'a [TriggerVersionRecord]);

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
