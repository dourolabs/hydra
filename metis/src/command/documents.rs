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
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
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
    /// Delete a document.
    Delete {
        /// Document ID (e.g., d-abcdef) or path (e.g., docs/plan.md).
        #[arg(value_name = "ID_OR_PATH")]
        id_or_path: String,
    },
    /// Sync documents to a local directory.
    Sync(SyncArgs),
    /// Push local document changes back to the server.
    Push(PushArgs),
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
    #[arg(long = "created-by", value_name = "TASK_ID")]
    pub created_by: Option<TaskId>,

    /// Show complete document body instead of truncated preview.
    #[arg(long = "full")]
    pub full: bool,

    /// Include deleted documents in the listing.
    #[arg(long = "include-deleted")]
    pub include_deleted: bool,
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

#[derive(Debug, Clone, Args)]
pub struct SyncArgs {
    /// Local directory to sync documents into.
    #[arg(value_name = "DIRECTORY")]
    pub directory: PathBuf,

    /// Only sync documents whose path starts with this prefix.
    #[arg(long = "path-prefix", value_name = "PREFIX")]
    pub path_prefix: Option<String>,

    /// Remove local files not present on the server.
    #[arg(long = "clean")]
    pub clean: bool,
}

#[derive(Debug, Clone, Args)]
pub struct PushArgs {
    /// Local directory previously synced with `metis documents sync`.
    #[arg(value_name = "DIRECTORY")]
    pub directory: PathBuf,

    /// Show what would be uploaded without making changes.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Only push documents whose path starts with this prefix.
    #[arg(long = "path-prefix", value_name = "PREFIX")]
    pub path_prefix: Option<String>,
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
        DocumentsCommand::Delete { id_or_path } => {
            let document = get_document_by_id_or_path(client, &id_or_path).await?;
            let deleted = client
                .delete_document(&document.id)
                .await
                .with_context(|| format!("failed to delete document '{}'", document.id))?;
            println!("Deleted document '{}'", deleted.id);
        }
        DocumentsCommand::Sync(args) => {
            sync_documents(client, args).await?;
        }
        DocumentsCommand::Push(args) => {
            push_documents(client, args).await?;
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
    let include_deleted = if args.include_deleted {
        Some(true)
    } else {
        None
    };
    let query = SearchDocumentsQuery::new(
        args.query,
        args.path_prefix,
        None,
        args.created_by,
        include_deleted,
    );
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
    let mut document = DocumentPayload::new(args.title.clone(), body, false);
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

const MANIFEST_FILENAME: &str = ".metis-documents.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SyncManifest {
    synced_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_prefix: Option<String>,
    documents: BTreeMap<String, SyncManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SyncManifestEntry {
    document_id: DocumentId,
    content_hash: String,
}

fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{result:x}")
}

fn load_manifest(directory: &Path) -> Result<Option<SyncManifest>> {
    let manifest_path = directory.join(MANIFEST_FILENAME);
    if !manifest_path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read manifest at '{}'", manifest_path.display()))?;
    let manifest: SyncManifest = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse manifest at '{}'", manifest_path.display()))?;
    Ok(Some(manifest))
}

fn save_manifest(directory: &Path, manifest: &SyncManifest) -> Result<()> {
    let manifest_path = directory.join(MANIFEST_FILENAME);
    let contents =
        serde_json::to_string_pretty(manifest).context("failed to serialize manifest")?;
    fs::write(&manifest_path, contents)
        .with_context(|| format!("failed to write manifest to '{}'", manifest_path.display()))?;
    Ok(())
}

pub async fn sync_documents(client: &dyn MetisClientInterface, args: SyncArgs) -> Result<()> {
    let directory = &args.directory;

    // Create directory if it doesn't exist
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create directory '{}'", directory.display()))?;

    // Load existing manifest for incremental sync
    let existing_manifest = load_manifest(directory)?;

    // List documents from server
    let query = SearchDocumentsQuery::new(None, args.path_prefix.clone(), None, None, None);
    let response = client
        .list_documents(&query)
        .await
        .context("failed to list documents")?;

    // Filter to only documents with a path
    let pathed_documents: Vec<&DocumentRecord> = response
        .documents
        .iter()
        .filter(|d| d.document.path.is_some())
        .collect();

    let mut new_entries = BTreeMap::new();
    let mut server_paths = HashSet::new();
    let mut synced_count = 0u64;
    let mut skipped_count = 0u64;

    for record in &pathed_documents {
        let doc_path = record.document.path.as_deref().unwrap();
        // Strip leading slash if present for filesystem path
        let relative_path = doc_path.strip_prefix('/').unwrap_or(doc_path);
        server_paths.insert(relative_path.to_string());

        let content_hash = compute_content_hash(&record.document.body_markdown);

        // Check if we can skip this document (incremental sync)
        if let Some(ref manifest) = existing_manifest {
            if let Some(existing_entry) = manifest.documents.get(relative_path) {
                if existing_entry.content_hash == content_hash
                    && existing_entry.document_id == record.id
                {
                    // Content unchanged, skip download
                    new_entries.insert(
                        relative_path.to_string(),
                        SyncManifestEntry {
                            document_id: record.id.clone(),
                            content_hash,
                        },
                    );
                    skipped_count += 1;
                    continue;
                }
            }
        }

        // Write file to disk
        let file_path = directory.join(relative_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory '{}'", parent.display()))?;
        }
        fs::write(&file_path, &record.document.body_markdown)
            .with_context(|| format!("failed to write file '{}'", file_path.display()))?;

        new_entries.insert(
            relative_path.to_string(),
            SyncManifestEntry {
                document_id: record.id.clone(),
                content_hash,
            },
        );
        synced_count += 1;
    }

    // Clean up local files not on server if --clean is set
    let mut removed_count = 0u64;
    if args.clean {
        if let Some(ref manifest) = existing_manifest {
            for local_path in manifest.documents.keys() {
                if !server_paths.contains(local_path.as_str()) {
                    let file_path = directory.join(local_path);
                    if file_path.exists() {
                        fs::remove_file(&file_path).with_context(|| {
                            format!("failed to remove file '{}'", file_path.display())
                        })?;
                        removed_count += 1;
                    }
                }
            }
        }
    }

    // Write manifest
    let manifest = SyncManifest {
        synced_at: chrono::Utc::now().to_rfc3339(),
        path_prefix: args.path_prefix,
        documents: new_entries,
    };
    save_manifest(directory, &manifest)?;

    println!(
        "Synced {} document(s) to '{}' ({} written, {} unchanged, {} removed)",
        pathed_documents.len(),
        directory.display(),
        synced_count,
        skipped_count,
        removed_count,
    );

    Ok(())
}

/// Derive a document title from a filename by removing the extension
/// and replacing hyphens/underscores with spaces, then capitalizing the first letter.
fn title_from_filename(filename: &str) -> String {
    let stem = filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(filename);
    let title: String = stem
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    let mut chars = title.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Collect local files in a directory, returning relative paths (excluding the manifest).
fn collect_local_files(directory: &Path, path_prefix: Option<&str>) -> Result<Vec<String>> {
    let mut files = Vec::new();
    collect_local_files_recursive(directory, directory, path_prefix, &mut files)?;
    Ok(files)
}

fn collect_local_files_recursive(
    base: &Path,
    current: &Path,
    path_prefix: Option<&str>,
    files: &mut Vec<String>,
) -> Result<()> {
    let entries = fs::read_dir(current)
        .with_context(|| format!("failed to read directory '{}'", current.display()))?;

    for entry in entries {
        let entry = entry.with_context(|| {
            format!("failed to read directory entry in '{}'", current.display())
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_local_files_recursive(base, &path, path_prefix, files)?;
        } else {
            let relative = path
                .strip_prefix(base)
                .with_context(|| {
                    format!("failed to compute relative path for '{}'", path.display())
                })?
                .to_string_lossy()
                .to_string();

            // Skip the manifest file
            if relative == MANIFEST_FILENAME {
                continue;
            }

            // Apply path prefix filter if specified
            if let Some(prefix) = path_prefix {
                let prefix_stripped = prefix.strip_prefix('/').unwrap_or(prefix);
                if !relative.starts_with(prefix_stripped) {
                    continue;
                }
            }

            files.push(relative);
        }
    }
    Ok(())
}

pub async fn push_documents(client: &dyn MetisClientInterface, args: PushArgs) -> Result<()> {
    let directory = &args.directory;

    // Safety guard: refuse to operate without a manifest
    let manifest = load_manifest(directory)?.with_context(|| {
        format!(
            "no manifest file found at '{}'. Run 'metis documents sync' first.",
            directory.join(MANIFEST_FILENAME).display()
        )
    })?;

    // Collect all local files
    let local_files = collect_local_files(directory, args.path_prefix.as_deref())?;

    let mut updated_count = 0u64;
    let mut created_count = 0u64;
    let mut unchanged_count = 0u64;
    let mut conflict_count = 0u64;

    let mut new_entries = manifest.documents.clone();

    for relative_path in &local_files {
        let file_path = directory.join(relative_path);
        let content = fs::read_to_string(&file_path)
            .with_context(|| format!("failed to read file '{}'", file_path.display()))?;
        let local_hash = compute_content_hash(&content);

        if let Some(entry) = manifest.documents.get(relative_path.as_str()) {
            // Existing document — check if changed locally
            if entry.content_hash == local_hash {
                unchanged_count += 1;
                continue;
            }

            // Check for server-side conflict by fetching current server content
            let server_record =
                client
                    .get_document(&entry.document_id)
                    .await
                    .with_context(|| {
                        format!(
                            "failed to fetch document '{}' from server",
                            entry.document_id
                        )
                    })?;
            let server_hash = compute_content_hash(&server_record.document.body_markdown);
            if server_hash != entry.content_hash {
                let doc_id = &entry.document_id;
                eprintln!(
                    "Warning: server document '{relative_path}' ({doc_id}) has changed since last sync; pushing local version anyway"
                );
                conflict_count += 1;
            }

            if args.dry_run {
                let doc_id = &entry.document_id;
                println!("Would update: {relative_path} ({doc_id})");
            } else {
                let mut document = server_record.document.clone();
                document.body_markdown = content.clone();
                client
                    .update_document(&entry.document_id, &UpsertDocumentRequest::new(document))
                    .await
                    .with_context(|| {
                        format!("failed to update document '{}'", entry.document_id)
                    })?;

                new_entries.insert(
                    relative_path.to_string(),
                    SyncManifestEntry {
                        document_id: entry.document_id.clone(),
                        content_hash: local_hash,
                    },
                );
                let doc_id = &entry.document_id;
                println!("Updated: {relative_path} ({doc_id})");
            }
            updated_count += 1;
        } else {
            // New file — create a new document on the server
            let doc_path = format!("/{relative_path}");
            let filename = Path::new(relative_path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| relative_path.clone());
            let title = title_from_filename(&filename);

            if args.dry_run {
                println!("Would create: {relative_path} (title: \"{title}\")");
            } else {
                let document =
                    DocumentPayload::new(title.clone(), content.clone(), false).with_path(doc_path);
                let response = client
                    .create_document(&UpsertDocumentRequest::new(document))
                    .await
                    .with_context(|| format!("failed to create document for '{relative_path}'"))?;

                new_entries.insert(
                    relative_path.to_string(),
                    SyncManifestEntry {
                        document_id: response.document_id.clone(),
                        content_hash: local_hash,
                    },
                );
                let doc_id = &response.document_id;
                println!("Created: {relative_path} ({doc_id}, title: \"{title}\")");
            }
            created_count += 1;
        }
    }

    // Update manifest (only if not dry-run)
    if !args.dry_run {
        let updated_manifest = SyncManifest {
            synced_at: chrono::Utc::now().to_rfc3339(),
            path_prefix: manifest.path_prefix.clone(),
            documents: new_entries,
        };
        save_manifest(directory, &updated_manifest)?;
    }

    let prefix = if args.dry_run { "Dry run: " } else { "" };
    let total = updated_count + created_count;
    let dir_display = directory.display();
    println!(
        "{prefix}Pushed {total} document(s) from '{dir_display}' ({updated_count} updated, {created_count} created, {unchanged_count} unchanged, {conflict_count} conflicts)"
    );

    Ok(())
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
        let document = DocumentPayload::new("Runbook".to_string(), "# Steps".to_string(), false)
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
                include_deleted: false,
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

    #[test]
    fn manifest_serialization_roundtrip() {
        let doc_id = DocumentId::new();
        let mut documents = BTreeMap::new();
        documents.insert(
            "playbooks/deploy.md".to_string(),
            SyncManifestEntry {
                document_id: doc_id.clone(),
                content_hash: "sha256:abc123".to_string(),
            },
        );
        let manifest = SyncManifest {
            synced_at: "2026-02-11T00:00:00Z".to_string(),
            path_prefix: Some("/playbooks".to_string()),
            documents,
        };

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let deserialized: SyncManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(manifest, deserialized);
        assert_eq!(
            deserialized.documents["playbooks/deploy.md"].document_id,
            doc_id
        );
        assert_eq!(deserialized.path_prefix, Some("/playbooks".to_string()));
    }

    #[test]
    fn manifest_serialization_omits_null_path_prefix() {
        let manifest = SyncManifest {
            synced_at: "2026-02-11T00:00:00Z".to_string(),
            path_prefix: None,
            documents: BTreeMap::new(),
        };

        let json = serde_json::to_string(&manifest).unwrap();
        assert!(!json.contains("path_prefix"));
    }

    #[test]
    fn content_hash_is_deterministic() {
        let hash1 = compute_content_hash("# Hello World");
        let hash2 = compute_content_hash("# Hello World");
        assert_eq!(hash1, hash2);
        assert!(hash1.starts_with("sha256:"));
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        let hash1 = compute_content_hash("# Hello");
        let hash2 = compute_content_hash("# World");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn load_manifest_returns_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_manifest(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn save_and_load_manifest_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let mut documents = BTreeMap::new();
        documents.insert(
            "docs/readme.md".to_string(),
            SyncManifestEntry {
                document_id: doc_id.clone(),
                content_hash: "sha256:def456".to_string(),
            },
        );
        let manifest = SyncManifest {
            synced_at: "2026-02-11T12:00:00Z".to_string(),
            path_prefix: None,
            documents,
        };

        save_manifest(dir.path(), &manifest).unwrap();
        let loaded = load_manifest(dir.path()).unwrap().unwrap();

        assert_eq!(loaded, manifest);
    }

    #[tokio::test]
    async fn sync_documents_downloads_pathed_documents() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let document = DocumentPayload::new(
            "Deploy Guide".to_string(),
            "# Deploy\nStep 1: Run deploy".to_string(),
            false,
        )
        .with_path("guides/deploy.md");
        let record = DocumentRecord::new(doc_id.clone(), document);
        let response = ListDocumentsResponse::new(vec![record]);

        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);

        sync_documents(
            &client,
            SyncArgs {
                directory: dir.path().to_path_buf(),
                path_prefix: None,
                clean: false,
            },
        )
        .await
        .unwrap();

        list_mock.assert();

        // Verify file was written
        let file_path = dir.path().join("guides/deploy.md");
        assert!(file_path.exists());
        let contents = fs::read_to_string(&file_path).unwrap();
        assert_eq!(contents, "# Deploy\nStep 1: Run deploy");

        // Verify manifest was written
        let manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(manifest.documents.len(), 1);
        assert!(manifest.documents.contains_key("guides/deploy.md"));
        assert_eq!(manifest.documents["guides/deploy.md"].document_id, doc_id);
    }

    #[tokio::test]
    async fn sync_documents_skips_unpathed_documents() {
        let dir = tempfile::tempdir().unwrap();
        let pathed_id = DocumentId::new();
        let unpathed_id = DocumentId::new();

        let pathed = DocumentRecord::new(
            pathed_id.clone(),
            DocumentPayload::new("Pathed".to_string(), "body".to_string(), false)
                .with_path("docs/pathed.md"),
        );
        let unpathed = DocumentRecord::new(
            unpathed_id,
            DocumentPayload::new("Unpathed".to_string(), "body".to_string(), false),
        );
        let response = ListDocumentsResponse::new(vec![pathed, unpathed]);

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);

        sync_documents(
            &client,
            SyncArgs {
                directory: dir.path().to_path_buf(),
                path_prefix: None,
                clean: false,
            },
        )
        .await
        .unwrap();

        let manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(manifest.documents.len(), 1);
        assert!(manifest.documents.contains_key("docs/pathed.md"));
    }

    #[tokio::test]
    async fn sync_documents_incremental_skips_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let body = "# Steps\nDo the thing";
        let document = DocumentPayload::new("Guide".to_string(), body.to_string(), false)
            .with_path("guides/steps.md");
        let record = DocumentRecord::new(doc_id.clone(), document);
        let response = ListDocumentsResponse::new(vec![record]);

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);

        // First sync
        sync_documents(
            &client,
            SyncArgs {
                directory: dir.path().to_path_buf(),
                path_prefix: None,
                clean: false,
            },
        )
        .await
        .unwrap();

        // Overwrite the file with different content to verify it doesn't get re-written
        let file_path = dir.path().join("guides/steps.md");
        fs::write(&file_path, "local changes").unwrap();

        // Second sync — should skip because manifest hash matches server content
        sync_documents(
            &client,
            SyncArgs {
                directory: dir.path().to_path_buf(),
                path_prefix: None,
                clean: false,
            },
        )
        .await
        .unwrap();

        // File should still have local changes because sync skipped it
        let contents = fs::read_to_string(&file_path).unwrap();
        assert_eq!(contents, "local changes");
    }

    #[tokio::test]
    async fn sync_documents_clean_removes_stale_files() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let document = DocumentPayload::new("Keep".to_string(), "keep body".to_string(), false)
            .with_path("docs/keep.md");
        let record = DocumentRecord::new(doc_id.clone(), document);

        let removed_id = DocumentId::new();
        let removed_doc =
            DocumentPayload::new("Remove".to_string(), "remove body".to_string(), false)
                .with_path("docs/remove.md");
        let removed_record = DocumentRecord::new(removed_id.clone(), removed_doc);

        // First sync with both documents
        let response = ListDocumentsResponse::new(vec![record.clone(), removed_record]);
        let server = MockServer::start();
        let mut mock1 = server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);

        sync_documents(
            &client,
            SyncArgs {
                directory: dir.path().to_path_buf(),
                path_prefix: None,
                clean: false,
            },
        )
        .await
        .unwrap();
        mock1.assert();

        assert!(dir.path().join("docs/keep.md").exists());
        assert!(dir.path().join("docs/remove.md").exists());

        // Second sync with only one document and --clean
        let response2 = ListDocumentsResponse::new(vec![record]);
        mock1.delete();
        let mock2 = server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200).json_body_obj(&response2);
        });

        sync_documents(
            &client,
            SyncArgs {
                directory: dir.path().to_path_buf(),
                path_prefix: None,
                clean: true,
            },
        )
        .await
        .unwrap();
        mock2.assert();

        assert!(dir.path().join("docs/keep.md").exists());
        assert!(!dir.path().join("docs/remove.md").exists());
    }

    #[tokio::test]
    async fn sync_documents_with_path_prefix_filter() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let document = DocumentPayload::new("Guide".to_string(), "body".to_string(), false)
            .with_path("playbooks/guide.md");
        let record = DocumentRecord::new(doc_id, document);
        let response = ListDocumentsResponse::new(vec![record]);

        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/documents")
                .query_param("path_prefix", "/playbooks");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);

        sync_documents(
            &client,
            SyncArgs {
                directory: dir.path().to_path_buf(),
                path_prefix: Some("/playbooks".to_string()),
                clean: false,
            },
        )
        .await
        .unwrap();

        list_mock.assert();

        let manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(manifest.path_prefix, Some("/playbooks".to_string()));
    }

    #[tokio::test]
    async fn sync_documents_handles_leading_slash_in_path() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let document = DocumentPayload::new("Guide".to_string(), "body".to_string(), false)
            .with_path("/playbooks/guide.md");
        let record = DocumentRecord::new(doc_id, document);
        let response = ListDocumentsResponse::new(vec![record]);

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200).json_body_obj(&response);
        });
        let client = mock_client(&server);

        sync_documents(
            &client,
            SyncArgs {
                directory: dir.path().to_path_buf(),
                path_prefix: None,
                clean: false,
            },
        )
        .await
        .unwrap();

        // Should strip leading slash
        assert!(dir.path().join("playbooks/guide.md").exists());
        let manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert!(manifest.documents.contains_key("playbooks/guide.md"));
    }

    #[test]
    fn title_from_filename_strips_extension_and_capitalizes() {
        assert_eq!(title_from_filename("deploy-guide.md"), "Deploy guide");
        assert_eq!(title_from_filename("my_notes.md"), "My notes");
        assert_eq!(title_from_filename("README"), "README");
        assert_eq!(title_from_filename("hello-world.txt"), "Hello world");
    }

    #[test]
    fn title_from_filename_handles_empty_and_no_extension() {
        assert_eq!(title_from_filename(""), "");
        assert_eq!(title_from_filename("notes"), "Notes");
    }

    #[tokio::test]
    async fn push_documents_refuses_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start();
        let client = mock_client(&server);

        let error = push_documents(
            &client,
            PushArgs {
                directory: dir.path().to_path_buf(),
                dry_run: false,
                path_prefix: None,
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("no manifest file found"));
    }

    #[tokio::test]
    async fn push_documents_uploads_modified_files() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let original_body = "# Original content";

        // Create manifest and file
        let mut documents = BTreeMap::new();
        documents.insert(
            "docs/guide.md".to_string(),
            SyncManifestEntry {
                document_id: doc_id.clone(),
                content_hash: compute_content_hash(original_body),
            },
        );
        let manifest = SyncManifest {
            synced_at: "2026-02-11T00:00:00Z".to_string(),
            path_prefix: None,
            documents,
        };
        save_manifest(dir.path(), &manifest).unwrap();

        // Write modified local file
        fs::create_dir_all(dir.path().join("docs")).unwrap();
        let modified_body = "# Updated content";
        fs::write(dir.path().join("docs/guide.md"), modified_body).unwrap();

        // Mock server: GET document (for conflict check), PUT update
        let server = MockServer::start();
        let server_record = DocumentRecord::new(
            doc_id.clone(),
            DocumentPayload::new("Guide".to_string(), original_body.to_string(), false)
                .with_path("/docs/guide.md"),
        );
        let doc_id_for_get = doc_id.clone();
        let get_mock = server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/documents/{doc_id_for_get}").as_str());
            then.status(200).json_body_obj(&server_record);
        });
        let doc_id_for_update = doc_id.clone();
        let update_mock = server.mock(move |when, then| {
            when.method(PUT)
                .path(format!("/v1/documents/{doc_id_for_update}").as_str());
            then.status(200)
                .json_body_obj(&UpsertDocumentResponse::new(doc_id_for_update.clone()));
        });
        let client = mock_client(&server);

        push_documents(
            &client,
            PushArgs {
                directory: dir.path().to_path_buf(),
                dry_run: false,
                path_prefix: None,
            },
        )
        .await
        .unwrap();

        get_mock.assert();
        update_mock.assert();

        // Verify manifest was updated with new hash
        let updated_manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(
            updated_manifest.documents["docs/guide.md"].content_hash,
            compute_content_hash(modified_body)
        );
    }

    #[tokio::test]
    async fn push_documents_skips_unchanged_files() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let body = "# Unchanged content";

        // Create manifest and file with same content
        let mut documents = BTreeMap::new();
        documents.insert(
            "docs/stable.md".to_string(),
            SyncManifestEntry {
                document_id: doc_id.clone(),
                content_hash: compute_content_hash(body),
            },
        );
        let manifest = SyncManifest {
            synced_at: "2026-02-11T00:00:00Z".to_string(),
            path_prefix: None,
            documents,
        };
        save_manifest(dir.path(), &manifest).unwrap();

        fs::create_dir_all(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("docs/stable.md"), body).unwrap();

        let server = MockServer::start();
        // No mocks needed — server should not be called
        let client = mock_client(&server);

        push_documents(
            &client,
            PushArgs {
                directory: dir.path().to_path_buf(),
                dry_run: false,
                path_prefix: None,
            },
        )
        .await
        .unwrap();

        // Manifest should be updated (synced_at changes) but content unchanged
        let updated_manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(
            updated_manifest.documents["docs/stable.md"].content_hash,
            compute_content_hash(body)
        );
    }

    #[tokio::test]
    async fn push_documents_creates_new_files() {
        let dir = tempfile::tempdir().unwrap();

        // Create manifest with no documents
        let manifest = SyncManifest {
            synced_at: "2026-02-11T00:00:00Z".to_string(),
            path_prefix: None,
            documents: BTreeMap::new(),
        };
        save_manifest(dir.path(), &manifest).unwrap();

        // Create a new local file
        fs::create_dir_all(dir.path().join("guides")).unwrap();
        let new_body = "# New Guide\nSome content";
        fs::write(dir.path().join("guides/new-guide.md"), new_body).unwrap();

        let new_doc_id = DocumentId::new();
        let server = MockServer::start();
        let new_doc_id_for_mock = new_doc_id.clone();
        let create_mock = server.mock(move |when, then| {
            when.method(POST).path("/v1/documents");
            then.status(200)
                .json_body_obj(&UpsertDocumentResponse::new(new_doc_id_for_mock.clone()));
        });
        let client = mock_client(&server);

        push_documents(
            &client,
            PushArgs {
                directory: dir.path().to_path_buf(),
                dry_run: false,
                path_prefix: None,
            },
        )
        .await
        .unwrap();

        create_mock.assert();

        // Verify manifest now contains the new document
        let updated_manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert!(updated_manifest
            .documents
            .contains_key("guides/new-guide.md"));
        assert_eq!(
            updated_manifest.documents["guides/new-guide.md"].document_id,
            new_doc_id
        );
    }

    #[tokio::test]
    async fn push_documents_dry_run_does_not_modify() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let original_body = "# Original";

        // Create manifest and modified file
        let mut documents = BTreeMap::new();
        documents.insert(
            "docs/guide.md".to_string(),
            SyncManifestEntry {
                document_id: doc_id.clone(),
                content_hash: compute_content_hash(original_body),
            },
        );
        let manifest = SyncManifest {
            synced_at: "2026-02-11T00:00:00Z".to_string(),
            path_prefix: None,
            documents,
        };
        save_manifest(dir.path(), &manifest).unwrap();

        fs::create_dir_all(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("docs/guide.md"), "# Modified").unwrap();

        // Also add a new file
        fs::create_dir_all(dir.path().join("guides")).unwrap();
        fs::write(dir.path().join("guides/new.md"), "# New").unwrap();

        let server = MockServer::start();
        // Mock GET for conflict check on the modified file
        let server_record = DocumentRecord::new(
            doc_id.clone(),
            DocumentPayload::new("Guide".to_string(), original_body.to_string(), false)
                .with_path("/docs/guide.md"),
        );
        let doc_id_for_get = doc_id.clone();
        server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/documents/{doc_id_for_get}").as_str());
            then.status(200).json_body_obj(&server_record);
        });
        // No PUT or POST mocks — should not be called in dry-run
        let client = mock_client(&server);

        push_documents(
            &client,
            PushArgs {
                directory: dir.path().to_path_buf(),
                dry_run: true,
                path_prefix: None,
            },
        )
        .await
        .unwrap();

        // Manifest should NOT have been updated
        let loaded_manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(loaded_manifest.synced_at, "2026-02-11T00:00:00Z");
        assert_eq!(loaded_manifest.documents.len(), 1);
        assert!(!loaded_manifest.documents.contains_key("guides/new.md"));
    }

    #[tokio::test]
    async fn push_documents_detects_server_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let doc_id = DocumentId::new();
        let original_body = "# Original synced content";
        let server_body = "# Server has changed this";

        // Create manifest with original hash
        let mut documents = BTreeMap::new();
        documents.insert(
            "docs/guide.md".to_string(),
            SyncManifestEntry {
                document_id: doc_id.clone(),
                content_hash: compute_content_hash(original_body),
            },
        );
        let manifest = SyncManifest {
            synced_at: "2026-02-11T00:00:00Z".to_string(),
            path_prefix: None,
            documents,
        };
        save_manifest(dir.path(), &manifest).unwrap();

        // Write locally modified file
        fs::create_dir_all(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("docs/guide.md"), "# Local changes").unwrap();

        // Mock server returns different content than what was synced
        let server = MockServer::start();
        let server_record = DocumentRecord::new(
            doc_id.clone(),
            DocumentPayload::new("Guide".to_string(), server_body.to_string(), false)
                .with_path("/docs/guide.md"),
        );
        let doc_id_for_get = doc_id.clone();
        server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/documents/{doc_id_for_get}").as_str());
            then.status(200).json_body_obj(&server_record);
        });
        let doc_id_for_update = doc_id.clone();
        server.mock(move |when, then| {
            when.method(PUT)
                .path(format!("/v1/documents/{doc_id_for_update}").as_str());
            then.status(200)
                .json_body_obj(&UpsertDocumentResponse::new(doc_id_for_update.clone()));
        });
        let client = mock_client(&server);

        // Push should succeed but print a warning (we can't easily capture stderr in test,
        // but we verify it doesn't error out)
        push_documents(
            &client,
            PushArgs {
                directory: dir.path().to_path_buf(),
                dry_run: false,
                path_prefix: None,
            },
        )
        .await
        .unwrap();

        // Manifest should be updated with local content hash
        let updated_manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(
            updated_manifest.documents["docs/guide.md"].content_hash,
            compute_content_hash("# Local changes")
        );
    }

    #[tokio::test]
    async fn push_documents_with_path_prefix_filter() {
        let dir = tempfile::tempdir().unwrap();

        // Create manifest with no docs
        let manifest = SyncManifest {
            synced_at: "2026-02-11T00:00:00Z".to_string(),
            path_prefix: None,
            documents: BTreeMap::new(),
        };
        save_manifest(dir.path(), &manifest).unwrap();

        // Create files in different directories
        fs::create_dir_all(dir.path().join("playbooks")).unwrap();
        fs::create_dir_all(dir.path().join("guides")).unwrap();
        fs::write(dir.path().join("playbooks/deploy.md"), "# Deploy").unwrap();
        fs::write(dir.path().join("guides/intro.md"), "# Intro").unwrap();

        let new_doc_id = DocumentId::new();
        let server = MockServer::start();
        let new_doc_id_for_mock = new_doc_id.clone();
        let create_mock = server.mock(move |when, then| {
            when.method(POST).path("/v1/documents");
            then.status(200)
                .json_body_obj(&UpsertDocumentResponse::new(new_doc_id_for_mock.clone()));
        });
        let client = mock_client(&server);

        push_documents(
            &client,
            PushArgs {
                directory: dir.path().to_path_buf(),
                dry_run: false,
                path_prefix: Some("/playbooks".to_string()),
            },
        )
        .await
        .unwrap();

        // Only playbooks/deploy.md should have been pushed
        create_mock.assert_hits(1);

        let updated_manifest = load_manifest(dir.path()).unwrap().unwrap();
        assert!(updated_manifest
            .documents
            .contains_key("playbooks/deploy.md"));
        assert!(!updated_manifest.documents.contains_key("guides/intro.md"));
    }

    #[test]
    fn collect_local_files_excludes_manifest() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(MANIFEST_FILENAME), "{}").unwrap();
        fs::write(dir.path().join("doc.md"), "content").unwrap();

        let files = collect_local_files(dir.path(), None).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.contains(&"doc.md".to_string()));
    }

    #[test]
    fn collect_local_files_with_path_prefix() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("playbooks")).unwrap();
        fs::create_dir_all(dir.path().join("guides")).unwrap();
        fs::write(dir.path().join("playbooks/deploy.md"), "content").unwrap();
        fs::write(dir.path().join("guides/intro.md"), "content").unwrap();

        let files = collect_local_files(dir.path(), Some("/playbooks")).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.contains(&"playbooks/deploy.md".to_string()));
    }
}
