use crate::{
    client::MetisClientInterface,
    command::output::{render_document_records, CommandContext, ResolvedOutputFormat},
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use metis_common::{
    constants::ENV_METIS_ID,
    documents::{
        Document as DocumentPayload, DocumentRecord, SearchDocumentsQuery, UpsertDocumentRequest,
    },
    DocumentId, TaskId,
};
use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    path::PathBuf,
};

#[derive(Debug, Subcommand)]
pub enum DocumentsCommand {
    /// List stored documents.
    List(DocumentsListArgs),
    /// Fetch a document by ID or path.
    Get {
        /// Document ID (e.g., d-abcdef) or path (e.g., docs/plan.md).
        #[arg(value_name = "ID_OR_PATH")]
        id_or_path: String,
    },
    /// Create a new document.
    Create(CreateDocumentArgs),
    /// Update an existing document.
    Update(UpdateDocumentArgs),
}

#[derive(Debug, Clone, Args)]
pub struct DocumentsListArgs {
    /// Query string used to match document titles or bodies.
    #[arg(long = "query", value_name = "QUERY")]
    pub query: Option<String>,

    /// Filter by path prefix (e.g. docs/runbooks/).
    #[arg(long = "path-prefix", value_name = "PREFIX")]
    pub path_prefix: Option<String>,

    /// Filter by job id that created the document.
    #[arg(long = "created-by", value_name = "TASK_ID", env = ENV_METIS_ID)]
    pub created_by: Option<TaskId>,

    /// Show complete document body instead of truncated preview.
    #[arg(long = "full")]
    pub full: bool,
}

#[derive(Debug, Clone, Args)]
pub struct CreateDocumentArgs {
    /// Title for the document.
    #[arg(long = "title", value_name = "TITLE")]
    pub title: String,

    /// Optional path (e.g. docs/designs/agent.md).
    #[arg(long = "path", value_name = "PATH")]
    pub path: Option<String>,

    /// Job id responsible for creating the document (defaults to $METIS_ID).
    #[arg(long = "created-by", value_name = "TASK_ID", env = ENV_METIS_ID)]
    pub created_by: Option<TaskId>,

    #[command(flatten)]
    pub body: DocumentBodyInput,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateDocumentArgs {
    /// Document id to update.
    #[arg(value_name = "DOCUMENT_ID")]
    pub id: DocumentId,

    /// Updated title for the document.
    #[arg(long = "title", value_name = "TITLE")]
    pub title: Option<String>,

    /// Updated path for the document.
    #[arg(long = "path", value_name = "PATH", conflicts_with = "clear_path")]
    pub path: Option<String>,

    /// Remove the existing path value.
    #[arg(long = "clear-path")]
    pub clear_path: bool,

    #[command(flatten)]
    pub body: DocumentBodyInput,
}

#[derive(Debug, Clone, Default, Args)]
pub struct DocumentBodyInput {
    /// Inline markdown body text.
    #[arg(long = "body", value_name = "MARKDOWN", conflicts_with_all = ["body_file", "body_stdin"])]
    pub body: Option<String>,

    /// Path to a file containing the markdown body.
    #[arg(long = "body-file", value_name = "FILE", conflicts_with_all = ["body", "body_stdin"])]
    pub body_file: Option<PathBuf>,

    /// Read the document body from stdin even if running in a terminal.
    #[arg(long = "body-stdin", conflicts_with_all = ["body", "body_file"])]
    pub body_stdin: bool,
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: DocumentsCommand,
    context: &CommandContext,
) -> Result<()> {
    match command {
        DocumentsCommand::List(args) => {
            let full_output = args.full;
            let documents = list_documents(client, args).await?;
            write_documents_output(context.output_format, &documents, full_output)?;
        }
        DocumentsCommand::Get { id_or_path } => {
            let document = get_document_by_id_or_path(client, &id_or_path).await?;
            write_documents_output(context.output_format, &[document], true)?;
        }
        DocumentsCommand::Create(args) => {
            let document = create_document(client, args).await?;
            write_documents_output(context.output_format, &[document], true)?;
        }
        DocumentsCommand::Update(args) => {
            let document = update_document(client, args).await?;
            write_documents_output(context.output_format, &[document], true)?;
        }
    }

    Ok(())
}

fn write_documents_output(
    format: ResolvedOutputFormat,
    documents: &[DocumentRecord],
    full_output: bool,
) -> Result<()> {
    let mut stdout = io::stdout();
    write_documents_output_with_writer(format, documents, full_output, &mut stdout)
}

fn write_documents_output_with_writer(
    format: ResolvedOutputFormat,
    documents: &[DocumentRecord],
    full_output: bool,
    writer: &mut impl Write,
) -> Result<()> {
    let buffer = render_documents_to_buffer(format, documents, full_output)?;
    writer.write_all(&buffer)?;
    writer.flush()?;
    Ok(())
}

fn render_documents_to_buffer(
    format: ResolvedOutputFormat,
    documents: &[DocumentRecord],
    full_output: bool,
) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    render_document_records(format, documents, full_output, &mut buffer)?;
    Ok(buffer)
}

async fn get_document_by_id_or_path(
    client: &dyn MetisClientInterface,
    id_or_path: &str,
) -> Result<DocumentRecord> {
    match DocumentId::try_from(id_or_path.to_string()) {
        Ok(id) => client
            .get_document(&id)
            .await
            .context("failed to fetch document"),
        Err(_) => client
            .get_document_by_path(id_or_path)
            .await
            .with_context(|| format!("failed to fetch document with path '{id_or_path}'")),
    }
}

async fn list_documents(
    client: &dyn MetisClientInterface,
    args: DocumentsListArgs,
) -> Result<Vec<DocumentRecord>> {
    let query = SearchDocumentsQuery::new(args.query, args.path_prefix, None, args.created_by);
    let response = client
        .list_documents(&query)
        .await
        .context("failed to list documents")?;
    Ok(response.documents)
}

async fn create_document(
    client: &dyn MetisClientInterface,
    args: CreateDocumentArgs,
) -> Result<DocumentRecord> {
    if args.title.trim().is_empty() {
        bail!("document title must not be empty");
    }

    if let Some(path) = args.path.as_deref() {
        if path.trim().is_empty() {
            bail!("document path must not be empty when provided");
        }
    }

    let body = args.body.read_required(true)?;
    let mut document = DocumentPayload::new(args.title.clone(), body);
    if let Some(path) = &args.path {
        document.path = Some(path.clone());
    }
    if let Some(created_by) = &args.created_by {
        document.created_by = Some(created_by.clone());
    }

    let response = client
        .create_document(&UpsertDocumentRequest::new(document))
        .await
        .context("failed to create document")?;

    client
        .get_document(&response.document_id)
        .await
        .context("failed to fetch created document")
}

async fn update_document(
    client: &dyn MetisClientInterface,
    args: UpdateDocumentArgs,
) -> Result<DocumentRecord> {
    let mut record = client
        .get_document(&args.id)
        .await
        .context("failed to fetch document")?;
    let mut document = record.document.clone();

    let mut changed = false;
    if let Some(title) = &args.title {
        if title.trim().is_empty() {
            bail!("document title must not be empty");
        }
        if document.title != *title {
            document.title = title.clone();
            changed = true;
        }
    }

    let body_override = args.body.read_optional(false)?;
    if let Some(body) = body_override {
        if document.body_markdown != body {
            document.body_markdown = body;
            changed = true;
        }
    }

    if args.clear_path {
        if document.path.take().is_some() {
            changed = true;
        }
    } else if let Some(path) = &args.path {
        if path.trim().is_empty() {
            bail!("document path must not be empty when provided");
        }
        if document.path.as_deref() != Some(path.as_str()) {
            document.path = Some(path.clone());
            changed = true;
        }
    }

    if !changed {
        bail!("no updates specified; use --title, --body/--body-file, --path, or --clear-path");
    }

    client
        .update_document(&args.id, &UpsertDocumentRequest::new(document.clone()))
        .await
        .context("failed to update document")?;

    record.document = document;
    Ok(record)
}

impl DocumentBodyInput {
    fn read_required(&self, allow_implicit_stdin: bool) -> Result<String> {
        match self.read_internal(allow_implicit_stdin)? {
            Some(body) => Ok(body),
            None => bail!(
                "document body is required; pass --body, --body-file, or pipe markdown via stdin"
            ),
        }
    }

    fn read_optional(&self, allow_implicit_stdin: bool) -> Result<Option<String>> {
        self.read_internal(allow_implicit_stdin)
    }

    fn read_internal(&self, allow_implicit_stdin: bool) -> Result<Option<String>> {
        if let Some(body) = &self.body {
            return Ok(Some(body.clone()));
        }

        if let Some(path) = &self.body_file {
            let contents = fs::read_to_string(path).with_context(|| {
                format!("failed to read document body from '{}'", path.display())
            })?;
            return Ok(Some(contents));
        }

        let mut stdin = io::stdin();
        let stdin_is_terminal = stdin.is_terminal();
        let should_read_stdin = self.body_stdin || (allow_implicit_stdin && !stdin_is_terminal);
        if should_read_stdin {
            let mut buffer = String::new();
            stdin
                .read_to_string(&mut buffer)
                .context("failed to read document body from stdin")?;
            return Ok(Some(buffer));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use httpmock::prelude::*;
    use metis_common::documents::{
        Document as DocumentPayload, ListDocumentsResponse, UpsertDocumentResponse,
    };
    use reqwest::Client as HttpClient;
    use serde_json::{json, Value};
    use std::io::{self, Write};
    use tempfile::NamedTempFile;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    fn mock_client(server: &MockServer) -> MetisClient {
        MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
            .expect("client")
    }

    fn sample_document_record(id: &DocumentId) -> DocumentRecord {
        let document = DocumentPayload::new("Runbook".to_string(), "# Steps".to_string())
            .with_path("docs/runbook.md")
            .with_created_by(TaskId::new());
        DocumentRecord::new(id.clone(), document)
    }

    #[tokio::test]
    async fn list_documents_supports_filters() {
        let document_id = DocumentId::new();
        let response = ListDocumentsResponse::new(vec![sample_document_record(&document_id)]);
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/documents")
                .query_param("q", "runbook")
                .query_param("path_prefix", "docs/")
                .query_param_exists("created_by");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);

        let created_by = TaskId::new();
        let records = list_documents(
            &client,
            DocumentsListArgs {
                query: Some("runbook".to_string()),
                path_prefix: Some("docs/".to_string()),
                created_by: Some(created_by),
                full: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, document_id);
        list_mock.assert();
    }

    #[tokio::test]
    async fn create_document_reads_body_from_file() {
        let document_id = DocumentId::new();
        let created_by = TaskId::new();
        let created_by_for_mock = created_by.clone();
        let document_id_for_mock = document_id.clone();
        let file = NamedTempFile::new().expect("temp file");
        fs::write(file.path(), "contents").expect("write body");
        let server = MockServer::start();
        let create_mock = server.mock(move |when, then| {
            when.method(POST).path("/v1/documents").json_body(json!({
                "document": {
                    "title": "Release notes",
                    "body_markdown": "contents",
                    "path": "docs/release.md",
                    "created_by": created_by_for_mock
                }
            }));
            then.status(200)
                .json_body_obj(&UpsertDocumentResponse::new(document_id_for_mock.clone()));
        });
        let document_record = sample_document_record(&document_id);
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/documents/{document_id}").as_str());
            then.status(200).json_body_obj(&document_record);
        });
        let client = mock_client(&server);

        let record = create_document(
            &client,
            CreateDocumentArgs {
                title: "Release notes".to_string(),
                path: Some("docs/release.md".to_string()),
                created_by: Some(created_by),
                body: DocumentBodyInput {
                    body: None,
                    body_file: Some(file.path().to_path_buf()),
                    body_stdin: false,
                },
            },
        )
        .await
        .unwrap();

        assert_eq!(record.id, document_id);
        create_mock.assert();
        get_mock.assert();
    }

    #[tokio::test]
    async fn update_document_applies_changes_and_allows_clearing_path() {
        let document_id = DocumentId::new();
        let server = MockServer::start();
        let existing = sample_document_record(&document_id);
        let update_path = format!("/v1/documents/{document_id}");
        let get_existing_path = update_path.clone();
        let existing_for_mock = existing.clone();
        let get_existing = server.mock(move |when, then| {
            when.method(GET).path(get_existing_path.as_str());
            then.status(200).json_body_obj(&existing_for_mock);
        });
        let document_id_for_update = document_id.clone();
        let update_mock = server.mock(move |when, then| {
            when.method(PUT);
            then.status(200)
                .json_body_obj(&UpsertDocumentResponse::new(document_id_for_update.clone()));
        });
        let client = mock_client(&server);

        let record = update_document(
            &client,
            UpdateDocumentArgs {
                id: document_id.clone(),
                title: Some("Updated".to_string()),
                path: None,
                clear_path: true,
                body: DocumentBodyInput {
                    body: Some("new body".to_string()),
                    body_file: None,
                    body_stdin: false,
                },
            },
        )
        .await
        .unwrap();

        assert!(record.document.path.is_none());
        assert_eq!(record.document.title, "Updated");
        assert_eq!(record.document.created_by, existing.document.created_by);
        get_existing.assert();
        update_mock.assert();
    }

    #[tokio::test]
    async fn update_document_requires_changes() {
        let document_id = DocumentId::new();
        let server = MockServer::start();
        let existing = sample_document_record(&document_id);
        let path = format!("/v1/documents/{document_id}");
        let existing_for_mock = existing.clone();
        let get_mock = server.mock(move |when, then| {
            when.method(GET).path(path.as_str());
            then.status(200).json_body_obj(&existing_for_mock);
        });
        let client = mock_client(&server);

        let error = update_document(
            &client,
            UpdateDocumentArgs {
                id: document_id,
                title: None,
                path: None,
                clear_path: false,
                body: DocumentBodyInput::default(),
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("no updates specified"));
        get_mock.assert();
    }

    #[test]
    fn write_documents_output_with_writer_buffers_pretty_output() {
        let document_id = DocumentId::new();
        let record = sample_document_record(&document_id);
        let mut writer = RecordingWriter::default();

        write_documents_output_with_writer(
            ResolvedOutputFormat::Pretty,
            &[record.clone()],
            false,
            &mut writer,
        )
        .unwrap();

        assert_eq!(writer.write_calls, 1);
        assert_eq!(writer.flush_calls, 1);
        let output = String::from_utf8(writer.buffer.clone()).unwrap();
        assert!(output.contains("Document"));
        assert!(output.contains(record.document.title.as_str()));
    }

    #[test]
    fn render_documents_to_buffer_supports_jsonl_output() {
        let document_id = DocumentId::new();
        let record = sample_document_record(&document_id);

        let buffer = render_documents_to_buffer(ResolvedOutputFormat::Jsonl, &[record], false)
            .expect("buffer");
        let output = String::from_utf8(buffer).expect("utf8");
        let lines: Vec<&str> = output.lines().collect();

        assert_eq!(lines.len(), 1);
        let parsed: Value = serde_json::from_str(lines[0]).expect("json");
        assert_eq!(parsed["id"], document_id.as_ref());
    }

    #[derive(Default)]
    struct RecordingWriter {
        buffer: Vec<u8>,
        write_calls: usize,
        flush_calls: usize,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.write_calls += 1;
            self.buffer.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flush_calls += 1;
            Ok(())
        }
    }

    #[tokio::test]
    async fn get_document_by_id_or_path_uses_id_endpoint_for_valid_id() {
        let document_id = DocumentId::new();
        let document_id_str = document_id.as_ref().to_string();
        let record = sample_document_record(&document_id);
        let server = MockServer::start();
        let get_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/documents/{document_id}").as_str());
            then.status(200).json_body_obj(&record);
        });
        let client = mock_client(&server);

        let result = get_document_by_id_or_path(&client, &document_id_str)
            .await
            .unwrap();

        assert_eq!(result.id, document_id);
        get_mock.assert();
    }

    #[tokio::test]
    async fn get_document_by_id_or_path_uses_list_endpoint_for_path() {
        let document_id = DocumentId::new();
        let path = "docs/runbook.md";
        let record = sample_document_record(&document_id);
        let response = ListDocumentsResponse::new(vec![record.clone()]);
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/documents")
                .query_param("path_prefix", path)
                .query_param("path_is_exact", "true");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);

        let result = get_document_by_id_or_path(&client, path).await.unwrap();

        assert_eq!(result.id, document_id);
        list_mock.assert();
    }

    #[tokio::test]
    async fn get_document_by_id_or_path_returns_error_for_missing_path() {
        let path = "docs/nonexistent.md";
        let response = ListDocumentsResponse::new(vec![]);
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/documents")
                .query_param("path_prefix", path)
                .query_param("path_is_exact", "true");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);

        let error = get_document_by_id_or_path(&client, path).await.unwrap_err();

        assert!(error.to_string().contains("docs/nonexistent.md"));
        list_mock.assert();
    }
}
