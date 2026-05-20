use std::io::Write;

use anyhow::Result;
use hydra_common::patches::{PatchStatus, PatchSummaryRecord, PatchVersionRecord};

use super::Render;

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

pub struct PatchRecords<'a>(pub &'a [PatchVersionRecord]);

pub struct PatchSummaryRecords<'a>(pub &'a [PatchSummaryRecord]);

impl Render for PatchRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for patch in self.0 {
            serde_json::to_writer(&mut *writer, patch)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        for patch in self.0 {
            write_patch_record_pretty(patch, writer)?;
        }
        writer.flush()?;
        Ok(())
    }
}

impl Render for PatchSummaryRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for patch in self.0 {
            serde_json::to_writer(&mut *writer, patch)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        for record in self.0 {
            writeln!(
                writer,
                "Patch {} [{}]: {}",
                record.patch_id,
                format_patch_status(record.patch.status),
                record.patch.title
            )?;
            writeln!(
                writer,
                "Repository: {}",
                record.patch.service_repo_name.as_str()
            )?;
            if let Some(ref branch) = record.patch.branch_name {
                writeln!(writer, "Branch: {branch}")?;
            }
            writeln!(writer)?;
        }
        writer.flush()?;
        Ok(())
    }
}

fn write_patch_record_pretty<W: Write>(record: &PatchVersionRecord, writer: &mut W) -> Result<()> {
    let title = extract_patch_title(record);
    let status = extract_patch_status(record);
    let description = extract_patch_description(record);
    writeln!(
        writer,
        "Patch {} [{}]: {}",
        record.patch_id,
        format_patch_status(status),
        title
    )?;
    writeln!(
        writer,
        "Repository: {}",
        record.patch.service_repo_name.as_str()
    )?;
    if !description.trim().is_empty() {
        writeln!(writer, "{description}")?;
    }
    if record.patch.diff.trim().is_empty() {
        writeln!(writer, "[no diff available]")?;
    } else {
        writeln!(writer)?;
        pretty_print_patch(&record.patch.diff, writer)?;
    }
    writeln!(writer)?;
    Ok(())
}

fn pretty_print_patch<W: Write>(patch: &str, writer: &mut W) -> Result<()> {
    for line in patch.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            writeln!(writer, "{GREEN}{line}{RESET}")?;
        } else if line.starts_with('-') && !line.starts_with("---") {
            writeln!(writer, "{RED}{line}{RESET}")?;
        } else {
            writeln!(writer, "{line}")?;
        }
    }
    Ok(())
}

fn extract_patch_title(record: &PatchVersionRecord) -> &str {
    record.patch.title.as_str()
}

fn extract_patch_status(record: &PatchVersionRecord) -> PatchStatus {
    record.patch.status
}

fn extract_patch_description(record: &PatchVersionRecord) -> &str {
    record.patch.description.as_str()
}

fn format_patch_status(status: PatchStatus) -> &'static str {
    match status {
        PatchStatus::Open => "open",
        PatchStatus::Closed => "closed",
        PatchStatus::Merged => "merged",
        PatchStatus::ChangesRequested => "changes requested",
        _ => "unknown",
    }
}
