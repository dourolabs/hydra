use std::io::Write;

use anyhow::Result;
use hydra_common::issues::{Issue, IssueSummary, IssueSummaryRecord, IssueVersionRecord};

use super::Render;

pub struct IssueRecords<'a>(pub &'a [IssueVersionRecord]);

pub struct IssueSummaryRecords<'a>(pub &'a [IssueSummaryRecord]);

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
            writeln!(writer, "Assignee: {}", assignee.as_deref().unwrap_or("-"))?;
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
            writeln!(writer, "Assignee: {}", assignee.as_deref().unwrap_or("-"))?;
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
