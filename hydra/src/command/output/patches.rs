use std::io::Write;

use anyhow::Result;
use hydra_common::{
    patches::{PatchStatus, PatchSummaryRecord, PatchVersionRecord},
    PatchId,
};
use serde::Serialize;
use serde_json::json;

use super::Render;

pub struct DeletedPatchOutcome<'a>(pub &'a PatchId);

impl Render for DeletedPatchOutcome<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(
            &mut *writer,
            &json!({ "patch_id": self.0, "action": "deleted" }),
        )?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(writer, "Deleted patch '{}'", self.0)?;
        writer.flush()?;
        Ok(())
    }
}

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

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MergeOutcome {
    Github {
        patch_id: PatchId,
    },
    Local {
        patch_id: PatchId,
        base_branch: String,
    },
}

impl Render for MergeOutcome {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        match self {
            MergeOutcome::Github { patch_id } => {
                writeln!(
                    writer,
                    "Patch '{patch_id}' merged successfully via GitHub API."
                )?;
            }
            MergeOutcome::Local {
                patch_id,
                base_branch,
            } => {
                writeln!(
                    writer,
                    "Patch '{patch_id}' merged successfully onto '{base_branch}'."
                )?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ApplyOutcome<'a> {
    pub patch_id: &'a PatchId,
    pub title: &'a str,
}

impl Render for ApplyOutcome<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(
            writer,
            "Applying patch '{}' to current git repository...",
            self.title
        )?;
        writeln!(writer)?;
        writeln!(writer, "Patch applied successfully.")?;
        writer.flush()?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ReviewSubmittedOutcome<'a> {
    pub patch_id: &'a PatchId,
}

impl Render for ReviewSubmittedOutcome<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(writer, "{}", self.patch_id)?;
        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::output::{render, ResolvedOutputFormat},
        test_utils::ids::patch_id,
    };
    use serde_json::json;

    fn sample_patch_id() -> PatchId {
        patch_id("p-abcd")
    }

    #[test]
    fn deleted_patch_pretty_matches_legacy_wording() {
        let id = patch_id("p-doomed");
        let mut output = Vec::new();
        render(
            DeletedPatchOutcome(&id),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .expect("render");
        let output = String::from_utf8(output).expect("utf8");
        assert_eq!(output, format!("Deleted patch '{id}'\n"));
    }

    #[test]
    fn deleted_patch_jsonl_emits_structured_record() {
        let id = patch_id("p-doomed");
        let mut output = Vec::new();
        render(
            DeletedPatchOutcome(&id),
            ResolvedOutputFormat::Jsonl,
            &mut output,
        )
        .expect("render");
        let output = String::from_utf8(output).expect("utf8");
        assert_eq!(output.lines().count(), 1);
        let parsed: serde_json::Value = serde_json::from_str(output.trim_end()).expect("json");
        assert_eq!(parsed, json!({ "patch_id": id, "action": "deleted" }));
    }

    #[test]
    fn merge_outcome_github_renders_pretty_line() {
        let outcome = MergeOutcome::Github {
            patch_id: sample_patch_id(),
        };
        let mut buf = Vec::new();
        render(outcome, ResolvedOutputFormat::Pretty, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            format!(
                "Patch '{}' merged successfully via GitHub API.\n",
                sample_patch_id()
            )
        );
    }

    #[test]
    fn merge_outcome_local_renders_pretty_line() {
        let outcome = MergeOutcome::Local {
            patch_id: sample_patch_id(),
            base_branch: "main".to_string(),
        };
        let mut buf = Vec::new();
        render(outcome, ResolvedOutputFormat::Pretty, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            format!(
                "Patch '{}' merged successfully onto 'main'.\n",
                sample_patch_id()
            )
        );
    }

    #[test]
    fn merge_outcome_github_renders_jsonl_object() {
        let outcome = MergeOutcome::Github {
            patch_id: sample_patch_id(),
        };
        let mut buf = Vec::new();
        render(outcome, ResolvedOutputFormat::Jsonl, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.ends_with('\n'), "jsonl output must end with newline");
        let trimmed = output.trim_end_matches('\n');
        assert!(
            !trimmed.contains('\n'),
            "jsonl output must be exactly one line"
        );
        let parsed: serde_json::Value = serde_json::from_str(trimmed).unwrap();
        assert_eq!(parsed["kind"], "github");
        assert_eq!(parsed["patch_id"], sample_patch_id().to_string());
    }

    #[test]
    fn merge_outcome_local_renders_jsonl_object() {
        let outcome = MergeOutcome::Local {
            patch_id: sample_patch_id(),
            base_branch: "main".to_string(),
        };
        let mut buf = Vec::new();
        render(outcome, ResolvedOutputFormat::Jsonl, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let trimmed = output.trim_end_matches('\n');
        let parsed: serde_json::Value = serde_json::from_str(trimmed).unwrap();
        assert_eq!(parsed["kind"], "local");
        assert_eq!(parsed["patch_id"], sample_patch_id().to_string());
        assert_eq!(parsed["base_branch"], "main");
    }

    #[test]
    fn apply_outcome_renders_pretty_two_lines() {
        let patch_id = sample_patch_id();
        let outcome = ApplyOutcome {
            patch_id: &patch_id,
            title: "fix bug",
        };
        let mut buf = Vec::new();
        render(outcome, ResolvedOutputFormat::Pretty, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            "Applying patch 'fix bug' to current git repository...\n\nPatch applied successfully.\n"
        );
    }

    #[test]
    fn apply_outcome_renders_jsonl_object() {
        let patch_id = sample_patch_id();
        let outcome = ApplyOutcome {
            patch_id: &patch_id,
            title: "fix bug",
        };
        let mut buf = Vec::new();
        render(outcome, ResolvedOutputFormat::Jsonl, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.ends_with('\n'));
        let trimmed = output.trim_end_matches('\n');
        assert!(!trimmed.contains('\n'));
        let parsed: serde_json::Value = serde_json::from_str(trimmed).unwrap();
        assert_eq!(parsed["patch_id"], patch_id.to_string());
        assert_eq!(parsed["title"], "fix bug");
    }

    #[test]
    fn review_submitted_outcome_renders_pretty_patch_id() {
        let patch_id = sample_patch_id();
        let outcome = ReviewSubmittedOutcome {
            patch_id: &patch_id,
        };
        let mut buf = Vec::new();
        render(outcome, ResolvedOutputFormat::Pretty, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output, format!("{patch_id}\n"));
    }

    #[test]
    fn review_submitted_outcome_renders_jsonl_object() {
        let patch_id = sample_patch_id();
        let outcome = ReviewSubmittedOutcome {
            patch_id: &patch_id,
        };
        let mut buf = Vec::new();
        render(outcome, ResolvedOutputFormat::Jsonl, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.ends_with('\n'));
        let trimmed = output.trim_end_matches('\n');
        let parsed: serde_json::Value = serde_json::from_str(trimmed).unwrap();
        assert_eq!(parsed["patch_id"], patch_id.to_string());
    }
}
