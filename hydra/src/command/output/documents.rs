use std::io::Write;

use anyhow::Result;
use hydra_common::{
    documents::{DocumentSummaryRecord, DocumentVersionRecord},
    DocumentId, VersionNumber,
};
use serde::Serialize;

use crate::util::truncate_lines;

use super::Render;

const MAX_DOCUMENT_BODY_LINES: usize = 20;
const MAX_DOCUMENT_BODY_WIDTH: usize = 120;

pub struct DocumentRecordsView<'a> {
    pub records: &'a [DocumentVersionRecord],
    pub full_output: bool,
}

pub struct DocumentSummaryRecords<'a>(pub &'a [DocumentSummaryRecord]);

/// Streaming event emitted while syncing or pushing documents.
///
/// Each variant has a JSON-tagged shape so jsonl consumers can dispatch on
/// `type`; the pretty rendering preserves the human-readable strings that the
/// commands historically printed.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent<'a> {
    Skipping {
        path: &'a str,
        document_id: &'a DocumentId,
        server_version: VersionNumber,
        manifest_version: VersionNumber,
    },
    Warning {
        path: &'a str,
        document_id: &'a DocumentId,
        server_version: VersionNumber,
        manifest_version: VersionNumber,
    },
    WouldUpdate {
        path: &'a str,
        document_id: &'a DocumentId,
    },
    Updated {
        path: &'a str,
        document_id: &'a DocumentId,
    },
    WouldCreate {
        path: &'a str,
        title: &'a str,
    },
    Created {
        path: &'a str,
        document_id: &'a DocumentId,
        title: &'a str,
    },
    UpdatedConflict {
        path: &'a str,
        document_id: &'a DocumentId,
        title: &'a str,
    },
    WouldDelete {
        path: &'a str,
        document_id: &'a DocumentId,
    },
    Deleted {
        path: &'a str,
        document_id: &'a DocumentId,
    },
}

/// Final summary emitted by `hydra documents sync` (`Synced`) or
/// `hydra documents push` (`Pushed`).
#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SyncSummary {
    Synced {
        directory: String,
        total: u64,
        written: u64,
        unchanged: u64,
        removed: u64,
    },
    Pushed {
        directory: String,
        total: u64,
        updated: u64,
        created: u64,
        deleted: u64,
        unchanged: u64,
        skipped: u64,
        conflicts: u64,
        dry_run: bool,
    },
}

impl Render for SyncEvent<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        match self {
            SyncEvent::Skipping {
                path,
                document_id,
                server_version,
                manifest_version,
            } => {
                writeln!(
                    writer,
                    "Skipping: '{path}' ({document_id}) — server has newer version (v{server_version} > v{manifest_version}), local unchanged"
                )?;
            }
            SyncEvent::Warning {
                path,
                document_id,
                server_version,
                manifest_version,
            } => {
                writeln!(
                    writer,
                    "Warning: server document '{path}' ({document_id}) has changed since last sync (v{server_version} > v{manifest_version}); pushing local version anyway"
                )?;
            }
            SyncEvent::WouldUpdate { path, document_id } => {
                writeln!(writer, "Would update: {path} ({document_id})")?;
            }
            SyncEvent::Updated { path, document_id } => {
                writeln!(writer, "Updated: {path} ({document_id})")?;
            }
            SyncEvent::WouldCreate { path, title } => {
                writeln!(writer, "Would create: {path} (title: \"{title}\")")?;
            }
            SyncEvent::Created {
                path,
                document_id,
                title,
            } => {
                writeln!(
                    writer,
                    "Created: {path} ({document_id}, title: \"{title}\")"
                )?;
            }
            SyncEvent::UpdatedConflict {
                path,
                document_id,
                title,
            } => {
                writeln!(
                    writer,
                    "Updated (conflict): {path} ({document_id}, title: \"{title}\")"
                )?;
            }
            SyncEvent::WouldDelete { path, document_id } => {
                writeln!(writer, "Would delete: {path} ({document_id})")?;
            }
            SyncEvent::Deleted { path, document_id } => {
                writeln!(writer, "Deleted: {path} ({document_id})")?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

impl Render for SyncSummary {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        match self {
            SyncSummary::Synced {
                directory,
                total,
                written,
                unchanged,
                removed,
            } => {
                writeln!(
                    writer,
                    "Synced {total} document(s) to '{directory}' ({written} written, {unchanged} unchanged, {removed} removed)"
                )?;
            }
            SyncSummary::Pushed {
                directory,
                total,
                updated,
                created,
                deleted,
                unchanged,
                skipped,
                conflicts,
                dry_run,
            } => {
                let prefix = if *dry_run { "Dry run: " } else { "" };
                writeln!(
                    writer,
                    "{prefix}Pushed {total} document(s) from '{directory}' ({updated} updated, {created} created, {deleted} deleted, {unchanged} unchanged, {skipped} skipped, {conflicts} conflicts)"
                )?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

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

    fn render_to_string<R: Render>(value: R, format: ResolvedOutputFormat) -> String {
        let mut buffer = Vec::new();
        render(value, format, &mut buffer).expect("render");
        String::from_utf8(buffer).expect("utf8")
    }

    #[test]
    fn sync_event_pretty_matches_legacy_wording() {
        let doc_id = DocumentId::new();
        let pretty = render_to_string(
            SyncEvent::Skipping {
                path: "docs/guide.md",
                document_id: &doc_id,
                server_version: 2,
                manifest_version: 1,
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(
            pretty,
            format!(
                "Skipping: 'docs/guide.md' ({doc_id}) — server has newer version (v2 > v1), local unchanged\n"
            )
        );

        let pretty = render_to_string(
            SyncEvent::Warning {
                path: "docs/guide.md",
                document_id: &doc_id,
                server_version: 3,
                manifest_version: 1,
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(
            pretty,
            format!(
                "Warning: server document 'docs/guide.md' ({doc_id}) has changed since last sync (v3 > v1); pushing local version anyway\n"
            )
        );

        let pretty = render_to_string(
            SyncEvent::WouldUpdate {
                path: "docs/guide.md",
                document_id: &doc_id,
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(pretty, format!("Would update: docs/guide.md ({doc_id})\n"));

        let pretty = render_to_string(
            SyncEvent::Updated {
                path: "docs/guide.md",
                document_id: &doc_id,
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(pretty, format!("Updated: docs/guide.md ({doc_id})\n"));

        let pretty = render_to_string(
            SyncEvent::WouldCreate {
                path: "docs/guide.md",
                title: "Guide",
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(pretty, "Would create: docs/guide.md (title: \"Guide\")\n");

        let pretty = render_to_string(
            SyncEvent::Created {
                path: "docs/guide.md",
                document_id: &doc_id,
                title: "Guide",
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(
            pretty,
            format!("Created: docs/guide.md ({doc_id}, title: \"Guide\")\n")
        );

        let pretty = render_to_string(
            SyncEvent::UpdatedConflict {
                path: "docs/guide.md",
                document_id: &doc_id,
                title: "Guide",
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(
            pretty,
            format!("Updated (conflict): docs/guide.md ({doc_id}, title: \"Guide\")\n")
        );

        let pretty = render_to_string(
            SyncEvent::WouldDelete {
                path: "docs/old.md",
                document_id: &doc_id,
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(pretty, format!("Would delete: docs/old.md ({doc_id})\n"));

        let pretty = render_to_string(
            SyncEvent::Deleted {
                path: "docs/old.md",
                document_id: &doc_id,
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(pretty, format!("Deleted: docs/old.md ({doc_id})\n"));
    }

    #[test]
    fn sync_event_jsonl_emits_tagged_object_per_event() {
        let doc_id = DocumentId::new();
        let line = render_to_string(
            SyncEvent::Created {
                path: "docs/guide.md",
                document_id: &doc_id,
                title: "Guide",
            },
            ResolvedOutputFormat::Jsonl,
        );
        assert!(line.ends_with('\n'));
        let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("json");
        assert_eq!(parsed["type"], "created");
        assert_eq!(parsed["path"], "docs/guide.md");
        assert_eq!(parsed["title"], "Guide");
        assert_eq!(parsed["document_id"], doc_id.as_ref());

        let line = render_to_string(
            SyncEvent::Skipping {
                path: "docs/guide.md",
                document_id: &doc_id,
                server_version: 4,
                manifest_version: 2,
            },
            ResolvedOutputFormat::Jsonl,
        );
        let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("json");
        assert_eq!(parsed["type"], "skipping");
        assert_eq!(parsed["server_version"], 4);
        assert_eq!(parsed["manifest_version"], 2);
    }

    #[test]
    fn sync_summary_pretty_matches_legacy_wording() {
        let pretty = render_to_string(
            SyncSummary::Synced {
                directory: "/tmp/docs".to_string(),
                total: 5,
                written: 3,
                unchanged: 1,
                removed: 1,
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(
            pretty,
            "Synced 5 document(s) to '/tmp/docs' (3 written, 1 unchanged, 1 removed)\n"
        );

        let pretty = render_to_string(
            SyncSummary::Pushed {
                directory: "/tmp/docs".to_string(),
                total: 4,
                updated: 2,
                created: 1,
                deleted: 1,
                unchanged: 0,
                skipped: 0,
                conflicts: 0,
                dry_run: false,
            },
            ResolvedOutputFormat::Pretty,
        );
        assert_eq!(
            pretty,
            "Pushed 4 document(s) from '/tmp/docs' (2 updated, 1 created, 1 deleted, 0 unchanged, 0 skipped, 0 conflicts)\n"
        );

        let pretty = render_to_string(
            SyncSummary::Pushed {
                directory: "/tmp/docs".to_string(),
                total: 0,
                updated: 0,
                created: 0,
                deleted: 0,
                unchanged: 1,
                skipped: 0,
                conflicts: 0,
                dry_run: true,
            },
            ResolvedOutputFormat::Pretty,
        );
        assert!(pretty.starts_with("Dry run: Pushed"));
    }

    #[test]
    fn sync_summary_jsonl_emits_action_tag() {
        let line = render_to_string(
            SyncSummary::Synced {
                directory: "/tmp/docs".to_string(),
                total: 2,
                written: 2,
                unchanged: 0,
                removed: 0,
            },
            ResolvedOutputFormat::Jsonl,
        );
        let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("json");
        assert_eq!(parsed["action"], "synced");
        assert_eq!(parsed["total"], 2);
        assert_eq!(parsed["written"], 2);

        let line = render_to_string(
            SyncSummary::Pushed {
                directory: "/tmp/docs".to_string(),
                total: 1,
                updated: 1,
                created: 0,
                deleted: 0,
                unchanged: 0,
                skipped: 0,
                conflicts: 0,
                dry_run: true,
            },
            ResolvedOutputFormat::Jsonl,
        );
        let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("json");
        assert_eq!(parsed["action"], "pushed");
        assert_eq!(parsed["dry_run"], true);
    }
}
