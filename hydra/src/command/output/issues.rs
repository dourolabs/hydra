use std::io::Write;

use anyhow::Result;
use hydra_common::{
    issues::{Issue, IssueSummary, IssueSummaryRecord, IssueVersionRecord, SubmitFormResponse},
    IssueId,
};
use serde_json::json;

use super::Render;

pub struct IssueRecords<'a>(pub &'a [IssueVersionRecord]);

pub struct IssueSummaryRecords<'a>(pub &'a [IssueSummaryRecord]);

pub struct SubmitFormOutcome<'a>(pub &'a SubmitFormResponse);

pub struct DeletedIssueOutcome<'a>(pub &'a IssueId);

impl Render for DeletedIssueOutcome<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(
            &mut *writer,
            &json!({ "issue_id": self.0, "action": "deleted" }),
        )?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(writer, "Deleted issue '{}'", self.0)?;
        writer.flush()?;
        Ok(())
    }
}

impl Render for IssueRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for issue in self.0 {
            serde_json::to_writer(&mut *writer, issue)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        for (index, issue_record) in self.0.iter().enumerate() {
            let Issue {
                issue_type,
                title,
                description,
                creator,
                progress,
                status,
                assignee,
                dependencies,
                ..
            } = &issue_record.issue;

            writeln!(
                writer,
                "Issue {} ({issue_type}, {status})",
                issue_record.issue_id
            )?;
            if !title.is_empty() {
                writeln!(writer, "Title: {title}")?;
            }
            writeln!(writer, "Creator: {}", creator.as_ref())?;
            writeln!(
                writer,
                "Assignee: {}",
                assignee
                    .as_ref()
                    .map(|p| p.to_path())
                    .as_deref()
                    .unwrap_or("-")
            )?;
            writeln!(writer, "Description:")?;
            if description.trim().is_empty() {
                writeln!(writer, "  -")?;
            } else {
                for line in description.lines() {
                    writeln!(writer, "  {line}")?;
                }
            }

            writeln!(writer, "Progress:")?;
            if progress.trim().is_empty() {
                writeln!(writer, "  -")?;
            } else {
                for line in progress.lines() {
                    writeln!(writer, "  {line}")?;
                }
            }

            if dependencies.is_empty() {
                writeln!(writer, "Dependencies: none")?;
            } else {
                writeln!(writer, "Dependencies:")?;
                for dependency in dependencies {
                    writeln!(
                        writer,
                        "  - {} {}",
                        dependency.dependency_type, dependency.issue_id
                    )?;
                }
            }

            if index + 1 < self.0.len() {
                writeln!(writer)?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

impl Render for SubmitFormOutcome<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self.0)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(
            writer,
            "Submitted form for issue '{}' (action: '{}', version: {})",
            self.0.issue_id, self.0.form_response.action_id, self.0.version,
        )?;
        writer.flush()?;
        Ok(())
    }
}

impl Render for IssueSummaryRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for issue in self.0 {
            serde_json::to_writer(&mut *writer, issue)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        for (index, issue_record) in self.0.iter().enumerate() {
            let IssueSummary {
                issue_type,
                title,
                description,
                creator,
                status,
                assignee,
                dependencies,
                ..
            } = &issue_record.issue;

            writeln!(
                writer,
                "Issue {} ({issue_type}, {status})",
                issue_record.issue_id
            )?;
            if !title.is_empty() {
                writeln!(writer, "Title: {title}")?;
            }
            writeln!(writer, "Creator: {}", creator.as_ref())?;
            writeln!(
                writer,
                "Assignee: {}",
                assignee
                    .as_ref()
                    .map(|p| p.to_path())
                    .as_deref()
                    .unwrap_or("-")
            )?;
            writeln!(writer, "Description:")?;
            if description.trim().is_empty() {
                writeln!(writer, "  -")?;
            } else {
                for line in description.lines() {
                    writeln!(writer, "  {line}")?;
                }
            }

            if dependencies.is_empty() {
                writeln!(writer, "Dependencies: none")?;
            } else {
                writeln!(writer, "Dependencies:")?;
                for dependency in dependencies {
                    writeln!(
                        writer,
                        "  - {} {}",
                        dependency.dependency_type, dependency.issue_id
                    )?;
                }
            }

            if index + 1 < self.0.len() {
                writeln!(writer)?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        command::output::{render, ResolvedOutputFormat},
        test_utils::ids::issue_id,
    };
    use serde_json::json;

    #[test]
    fn deleted_issue_pretty_matches_legacy_wording() {
        let id = issue_id("i-doomed");
        let mut output = Vec::new();
        render(
            DeletedIssueOutcome(&id),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .expect("render");
        let output = String::from_utf8(output).expect("utf8");
        assert_eq!(output, format!("Deleted issue '{id}'\n"));
    }

    #[test]
    fn deleted_issue_jsonl_emits_structured_record() {
        let id = issue_id("i-doomed");
        let mut output = Vec::new();
        render(
            DeletedIssueOutcome(&id),
            ResolvedOutputFormat::Jsonl,
            &mut output,
        )
        .expect("render");
        let output = String::from_utf8(output).expect("utf8");
        assert_eq!(output.lines().count(), 1);
        let parsed: serde_json::Value = serde_json::from_str(output.trim_end()).expect("json");
        assert_eq!(parsed, json!({ "issue_id": id, "action": "deleted" }));
    }
}
