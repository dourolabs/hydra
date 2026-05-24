use std::io::Write;

use anyhow::Result;
use hydra_common::documents::{DocumentSummaryRecord, DocumentVersionRecord};

use crate::util::truncate_lines;

use super::Render;

const MAX_DOCUMENT_BODY_LINES: usize = 20;
const MAX_DOCUMENT_BODY_WIDTH: usize = 120;

pub struct DocumentRecordsView<'a> {
    pub records: &'a [DocumentVersionRecord],
    pub full_output: bool,
}

pub struct DocumentSummaryRecords<'a>(pub &'a [DocumentSummaryRecord]);

impl Render for DocumentRecordsView<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for document in self.records {
            serde_json::to_writer(&mut *writer, document)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.records.is_empty() {
            writeln!(writer, "No documents found.")?;
            writer.flush()?;
            return Ok(());
        }

        for (index, record) in self.records.iter().enumerate() {
            writeln!(writer, "Document {}", record.document_id)?;
            writeln!(writer, "Title: {}", record.document.title)?;
            let path = record.document.path.as_deref().unwrap_or("-");
            writeln!(writer, "Path: {path}")?;
            writeln!(writer, "Body:")?;

            let lines: Vec<String> = record
                .document
                .body_markdown
                .lines()
                .map(|line| line.to_string())
                .collect();
            if lines.is_empty() {
                writeln!(writer, "  -")?;
            } else {
                let output_lines = if self.full_output {
                    lines
                } else {
                    truncate_lines(lines, MAX_DOCUMENT_BODY_LINES, MAX_DOCUMENT_BODY_WIDTH)
                };
                for line in output_lines {
                    if line.is_empty() {
                        writeln!(writer, "  ")?;
                    } else {
                        writeln!(writer, "  {line}")?;
                    }
                }
            }

            if index + 1 < self.records.len() {
                writeln!(writer)?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

impl Render for DocumentSummaryRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for document in self.0 {
            serde_json::to_writer(&mut *writer, document)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.0.is_empty() {
            writeln!(writer, "No documents found.")?;
            writer.flush()?;
            return Ok(());
        }

        for (index, record) in self.0.iter().enumerate() {
            writeln!(writer, "Document {}", record.document_id)?;
            writeln!(writer, "Title: {}", record.document.title)?;
            let path = record.document.path.as_deref().unwrap_or("-");
            writeln!(writer, "Path: {path}")?;

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
    use crate::command::output::{render, ResolvedOutputFormat};
    use chrono::Utc;
    use hydra_common::{
        documents::{Document, DocumentVersionRecord},
        DocumentId,
    };

    #[test]
    fn render_document_records_truncates_body_by_default() {
        let mut body_lines = Vec::new();
        for index in 0..25 {
            body_lines.push(format!("line {index:02} {}", "x".repeat(10)));
        }
        let document = Document::new(
            "Doc".to_string(),
            body_lines.join("\n"),
            Some("docs/runbook.md".to_string()),
            false,
        )
        .unwrap();
        let record = DocumentVersionRecord::new(
            DocumentId::new(),
            0,
            Utc::now(),
            document,
            None,
            Utc::now(),
            Vec::new(),
        );
        let mut output = Vec::new();
        render(
            DocumentRecordsView {
                records: &[record],
                full_output: false,
            },
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .unwrap();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("Document"));
        assert!(rendered.contains("line 19"));
        assert!(rendered.contains("..."));
        assert!(!rendered.contains("line 24"));
    }

    #[test]
    fn render_document_records_shows_full_body_when_requested() {
        let mut body_lines = Vec::new();
        for index in 0..25 {
            body_lines.push(format!("line {index:02} {}", "x".repeat(10)));
        }
        let document = Document::new(
            "Doc".to_string(),
            body_lines.join("\n"),
            Some("docs/runbook.md".to_string()),
            false,
        )
        .unwrap();
        let record = DocumentVersionRecord::new(
            DocumentId::new(),
            0,
            Utc::now(),
            document,
            None,
            Utc::now(),
            Vec::new(),
        );
        let mut output = Vec::new();
        render(
            DocumentRecordsView {
                records: &[record],
                full_output: true,
            },
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .unwrap();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("Document"));
        assert!(rendered.contains("line 00"));
        assert!(rendered.contains("line 24"));
        assert!(!rendered.contains("..."));
    }
}
