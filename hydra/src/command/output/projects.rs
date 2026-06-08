use std::io::Write;

use anyhow::Result;
use hydra_common::api::v1::projects::{ProjectRecord, ProjectStatusesResponse, StatusDefinition};

use super::Render;

pub struct ProjectRecords<'a>(pub &'a [ProjectRecord]);

impl Render for ProjectRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for project in self.0 {
            serde_json::to_writer(&mut *writer, project)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.0.is_empty() {
            writeln!(writer, "No projects configured.")?;
            writer.flush()?;
            return Ok(());
        }

        for (index, project) in self.0.iter().enumerate() {
            write_project_details(project, writer)?;
            if index + 1 < self.0.len() {
                writeln!(writer)?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

fn write_project_details<W: Write>(record: &ProjectRecord, writer: &mut W) -> Result<()> {
    let project = &record.project;
    writeln!(writer, "- {} ({})", project.key, record.project_id)?;
    writeln!(writer, "  name: {}", project.name)?;
    writeln!(writer, "  version: {}", record.version)?;
    writeln!(writer, "  creator: {}", project.creator)?;
    writeln!(
        writer,
        "  default_status_key: {}",
        project.default_status_key
    )?;
    if project.deleted {
        writeln!(writer, "  deleted: true")?;
    }
    writeln!(writer, "  statuses:")?;
    for status in &project.statuses {
        write_status_summary(status, writer, "    ")?;
    }
    Ok(())
}

fn write_status_summary<W: Write>(
    status: &StatusDefinition,
    writer: &mut W,
    indent: &str,
) -> Result<()> {
    writeln!(writer, "{indent}- {} ({})", status.key, status.label)?;
    writeln!(writer, "{indent}  color: {}", status.color)?;
    writeln!(
        writer,
        "{indent}  unblocks_parents: {}",
        status.unblocks_parents
    )?;
    writeln!(
        writer,
        "{indent}  unblocks_dependents: {}",
        status.unblocks_dependents
    )?;
    writeln!(
        writer,
        "{indent}  cascades_to_children: {}",
        status.cascades_to_children
    )?;
    if let Some(on_enter) = &status.on_enter {
        writeln!(writer, "{indent}  on_enter:")?;
        if let Some(assign_to) = &on_enter.assign_to {
            writeln!(writer, "{indent}    assign_to: {}", assign_to.to_path())?;
        }
        if let Some(attach_form) = &on_enter.attach_form {
            writeln!(writer, "{indent}    attach_form: {attach_form}")?;
        }
    }
    Ok(())
}

pub struct ProjectStatuses<'a>(pub &'a ProjectStatusesResponse);

impl Render for ProjectStatuses<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self.0)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(writer, "default_status_key: {}", self.0.default_status_key)?;
        if self.0.statuses.is_empty() {
            writeln!(writer, "statuses: <none>")?;
        } else {
            writeln!(writer, "statuses:")?;
            for status in &self.0.statuses {
                write_status_summary(status, writer, "  ")?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}
