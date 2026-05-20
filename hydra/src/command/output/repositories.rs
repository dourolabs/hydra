use std::io::Write;

use anyhow::Result;
use hydra_common::repositories::RepositoryRecord;

use super::Render;

pub struct RepositoryRecords<'a>(pub &'a [RepositoryRecord]);

impl Render for RepositoryRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for repository in self.0 {
            serde_json::to_writer(&mut *writer, repository)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.0.is_empty() {
            writeln!(writer, "No repositories configured.")?;
            writer.flush()?;
            return Ok(());
        }

        for (index, repository) in self.0.iter().enumerate() {
            write_repository_details(repository, writer)?;
            if index + 1 < self.0.len() {
                writeln!(writer)?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

fn write_repository_details<W: Write>(repository: &RepositoryRecord, writer: &mut W) -> Result<()> {
    let config = &repository.repository;
    writeln!(writer, "- {}", repository.name)?;
    writeln!(writer, "  remote_url: {}", config.remote_url)?;
    writeln!(
        writer,
        "  default_branch: {}",
        config.default_branch.as_deref().unwrap_or("<none>")
    )?;
    writeln!(
        writer,
        "  default_image: {}",
        config.default_image.as_deref().unwrap_or("<none>")
    )?;
    if let Some(ref pw) = config.patch_workflow {
        if !pw.review_requests.is_empty() {
            let reviewers: Vec<&str> = pw
                .review_requests
                .iter()
                .map(|r| r.assignee.as_str())
                .collect();
            writeln!(writer, "  reviewers: {}", reviewers.join(", "))?;
        }
        if let Some(ref mr) = pw.merge_request {
            writeln!(
                writer,
                "  merger: {}",
                mr.assignee.as_deref().unwrap_or("<none>")
            )?;
        }
    }
    Ok(())
}
