use std::io::Write;

use anyhow::Result;
use hydra_common::users::UserSummary;

use super::Render;

pub struct UserRecords<'a>(pub &'a [UserSummary]);

impl Render for UserRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for user in self.0 {
            serde_json::to_writer(&mut *writer, user)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.0.is_empty() {
            writeln!(writer, "No users found.")?;
            writer.flush()?;
            return Ok(());
        }

        for (index, user) in self.0.iter().enumerate() {
            write_user_details(user, writer)?;
            if index + 1 < self.0.len() {
                writeln!(writer)?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

fn write_user_details<W: Write>(user: &UserSummary, writer: &mut W) -> Result<()> {
    writeln!(writer, "- {}", user.username)?;
    match user.github_user_id {
        Some(id) => writeln!(writer, "  github_user_id: {id}")?,
        None => writeln!(writer, "  github_user_id: N/A")?,
    }
    Ok(())
}
