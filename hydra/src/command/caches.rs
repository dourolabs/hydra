use crate::command::output::{CommandContext, ResolvedOutputFormat};
use crate::config;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use git2::Repository;
use hydra_build_cache::{
    find_nearest_cache_entry, BuildCacheClient, BuildCacheConfig, BuildCacheEntry, BuildCacheKey,
    FileSystemStorageClient, FileSystemStorageConfig, S3StorageClient, S3StorageConfig,
    StorageClient,
};
use hydra_common::RepoName;
use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Subcommand)]
pub enum CachesCommand {
    /// Build and upload a cache archive.
    Build(BuildCacheArgs),
    /// List available cache entries.
    List(ListCacheArgs),
    /// Apply a cache archive.
    Apply(ApplyCacheArgs),
}

#[derive(Debug, Clone, Args)]
pub struct CacheStorageArgs {
    /// Root directory for filesystem cache storage.
    #[arg(long = "storage-root", value_name = "DIR")]
    pub storage_root: Option<PathBuf>,

    /// S3-compatible endpoint URL.
    #[arg(long = "s3-endpoint-url", value_name = "URL")]
    pub s3_endpoint_url: Option<String>,

    /// S3 bucket name.
    #[arg(long = "s3-bucket", value_name = "BUCKET")]
    pub s3_bucket: Option<String>,

    /// S3 region name.
    #[arg(long = "s3-region", value_name = "REGION")]
    pub s3_region: Option<String>,

    /// S3 access key ID.
    #[arg(long = "s3-access-key-id", value_name = "ACCESS_KEY_ID")]
    pub s3_access_key_id: Option<String>,

    /// S3 secret access key.
    #[arg(long = "s3-secret-access-key", value_name = "SECRET_ACCESS_KEY")]
    pub s3_secret_access_key: Option<String>,

    /// S3 session token.
    #[arg(long = "s3-session-token", value_name = "SESSION_TOKEN")]
    pub s3_session_token: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct BuildCacheArgs {
    /// Repository name in the form org/repo.
    #[arg(long = "repo-name", value_name = "REPO")]
    pub repo_name: RepoName,

    /// Repository root directory.
    #[arg(long = "root", value_name = "DIR", default_value = ".")]
    pub root: PathBuf,

    /// Git SHA to use for the cache key (defaults to HEAD).
    #[arg(long = "git-sha", value_name = "SHA")]
    pub git_sha: Option<String>,

    /// Show which paths would be included, without uploading.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Home directory to include shared caches from.
    #[arg(long = "home-dir", value_name = "DIR")]
    pub home_dir: Option<PathBuf>,

    #[command(flatten)]
    pub storage: CacheStorageArgs,
}

#[derive(Debug, Clone, Args)]
pub struct ListCacheArgs {
    /// Repository name in the form org/repo.
    #[arg(long = "repo-name", value_name = "REPO")]
    pub repo_name: RepoName,

    #[command(flatten)]
    pub storage: CacheStorageArgs,
}

#[derive(Debug, Clone, Args)]
pub struct ApplyCacheArgs {
    /// Repository name in the form org/repo.
    #[arg(long = "repo-name", value_name = "REPO")]
    pub repo_name: RepoName,

    /// Repository root directory.
    #[arg(long = "root", value_name = "DIR", default_value = ".")]
    pub root: PathBuf,

    /// Git SHA to apply when not using --nearest.
    #[arg(
        long = "git-sha",
        value_name = "SHA",
        required_unless_present = "nearest",
        conflicts_with = "nearest"
    )]
    pub git_sha: Option<String>,

    /// Choose the cache entry closest to the current HEAD.
    #[arg(long = "nearest")]
    pub nearest: bool,

    /// Home directory where shared cache entries should be restored.
    #[arg(long = "home-dir", value_name = "DIR")]
    pub home_dir: Option<PathBuf>,

    #[command(flatten)]
    pub storage: CacheStorageArgs,
}

pub async fn run(command: CachesCommand, context: &CommandContext) -> Result<()> {
    match command {
        CachesCommand::Build(args) => build_cache(args, context).await,
        CachesCommand::List(args) => list_caches(args, context).await,
        CachesCommand::Apply(args) => apply_cache(args, context).await,
    }
}

async fn build_cache(args: BuildCacheArgs, context: &CommandContext) -> Result<()> {
    let root = resolve_root(&args.root)?;
    let home_dir = resolve_home_dir(&args.home_dir);
    let client = build_cache_client(&args.storage)?;

    if args.dry_run {
        let entries = client
            .list_cache_entries(&root, home_dir.as_deref())
            .context("failed to collect cache entries")?;
        return render_cache_paths(context.output_format, &entries);
    }

    let git_sha = resolve_git_sha(&root, args.git_sha.as_deref())?;
    let key = BuildCacheKey::new(args.repo_name.clone(), git_sha.clone());
    let temp = tempfile::NamedTempFile::new().context("failed to create temp cache file")?;
    let archive_path = temp.path().to_path_buf();

    client
        .build_cache_archive_async(root.clone(), home_dir.clone(), archive_path.clone())
        .await
        .context("failed to build cache archive")?;
    client
        .upload_cache(&key, &archive_path)
        .await
        .context("failed to upload cache archive")?;

    render_cache_build(context.output_format, &key)
}

async fn list_caches(args: ListCacheArgs, context: &CommandContext) -> Result<()> {
    let client = build_cache_client(&args.storage)?;
    let mut entries = client
        .list_caches(args.repo_name.clone())
        .await
        .context("failed to list cache entries")?;
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    render_cache_entries(context.output_format, &entries)
}

async fn apply_cache(args: ApplyCacheArgs, context: &CommandContext) -> Result<()> {
    let root = resolve_root(&args.root)?;
    let home_dir = resolve_home_dir(&args.home_dir);
    let client = build_cache_client(&args.storage)?;

    let (key, distance) = if args.nearest {
        let entries = client
            .list_caches(args.repo_name.clone())
            .await
            .context("failed to list cache entries")?;
        match find_nearest_cache_entry(&root, args.repo_name.clone(), entries)
            .context("failed to find nearest cache entry")?
        {
            Some(nearest) => (nearest.key, Some(nearest.distance)),
            None => {
                return render_cache_apply(context.output_format, CacheApplyOutput::not_found());
            }
        }
    } else {
        let git_sha = resolve_git_sha(&root, args.git_sha.as_deref())?;
        (BuildCacheKey::new(args.repo_name.clone(), git_sha), None)
    };

    client
        .download_and_apply_cache(&root, home_dir.as_deref(), &key)
        .await
        .context("failed to apply cache entry")?;

    render_cache_apply(
        context.output_format,
        CacheApplyOutput::applied(&key, distance),
    )
}

fn resolve_root(root: &Path) -> Result<PathBuf> {
    Ok(config::expand_path(root))
}

fn resolve_home_dir(home: &Option<PathBuf>) -> Option<PathBuf> {
    home.as_ref().map(config::expand_path)
}

fn resolve_git_sha(root: &Path, override_sha: Option<&str>) -> Result<String> {
    if let Some(raw) = override_sha {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("git SHA must not be empty");
        }
        return Ok(trimmed.to_string());
    }

    let repo = Repository::discover(root)
        .map_err(|err| anyhow!("failed to discover git repository: {err}"))?;
    let head = repo
        .head()
        .map_err(|err| anyhow!("failed to resolve HEAD: {err}"))?;
    let commit = head
        .peel_to_commit()
        .map_err(|err| anyhow!("failed to resolve HEAD commit: {err}"))?;
    Ok(commit.id().to_string())
}

fn build_cache_client(args: &CacheStorageArgs) -> Result<BuildCacheClient> {
    let storage = build_storage_client(args)?;
    Ok(BuildCacheClient::new(BuildCacheConfig::default(), storage))
}

fn build_storage_client(args: &CacheStorageArgs) -> Result<Arc<dyn StorageClient>> {
    let has_s3_args = args.s3_endpoint_url.is_some()
        || args.s3_bucket.is_some()
        || args.s3_region.is_some()
        || args.s3_access_key_id.is_some()
        || args.s3_secret_access_key.is_some()
        || args.s3_session_token.is_some();

    if args.storage_root.is_some() && has_s3_args {
        bail!("choose either filesystem storage or S3 storage, not both");
    }

    if let Some(root) = args.storage_root.as_ref() {
        let root = config::expand_path(root);
        let client = FileSystemStorageClient::new(&FileSystemStorageConfig {
            root_dir: root.to_string_lossy().to_string(),
        })
        .context("failed to configure filesystem storage")?;
        return Ok(Arc::new(client));
    }

    if has_s3_args {
        let endpoint_url = args
            .s3_endpoint_url
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("--s3-endpoint-url is required for S3 storage"))?;
        let bucket = args
            .s3_bucket
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("--s3-bucket is required for S3 storage"))?;
        let region = args
            .s3_region
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("--s3-region is required for S3 storage"))?;

        let config = S3StorageConfig {
            endpoint_url,
            bucket,
            region,
            access_key_id: args.s3_access_key_id.clone(),
            secret_access_key: args.s3_secret_access_key.clone(),
            session_token: args.s3_session_token.clone(),
        };
        let client = S3StorageClient::new(&config).context("failed to configure S3 storage")?;
        return Ok(Arc::new(client));
    }

    bail!("storage configuration is required (use --storage-root or S3 options)")
}

fn render_cache_paths(format: ResolvedOutputFormat, paths: &[PathBuf]) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match format {
        ResolvedOutputFormat::Jsonl => {
            for path in paths {
                let record = CachePathOutput {
                    path: path.to_string_lossy().to_string(),
                };
                write_json_line(&record, &mut stdout)?;
            }
        }
        ResolvedOutputFormat::Pretty => {
            if paths.is_empty() {
                writeln!(stdout, "No cache entries matched the configured patterns.")?;
            } else {
                for path in paths {
                    let path = path.to_string_lossy();
                    writeln!(stdout, "{path}")?;
                }
            }
        }
    }
    stdout.flush()?;
    Ok(())
}

fn render_cache_entries(format: ResolvedOutputFormat, entries: &[BuildCacheEntry]) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match format {
        ResolvedOutputFormat::Jsonl => {
            for entry in entries {
                let record = CacheListOutput::from_entry(entry);
                write_json_line(&record, &mut stdout)?;
            }
        }
        ResolvedOutputFormat::Pretty => {
            if entries.is_empty() {
                writeln!(stdout, "No cache entries found.")?;
            } else {
                for entry in entries {
                    let last_modified = entry
                        .last_modified
                        .as_ref()
                        .map(format_system_time)
                        .unwrap_or_else(|| "-".to_string());
                    let key = &entry.key;
                    writeln!(stdout, "{key}\t{last_modified}")?;
                }
            }
        }
    }
    stdout.flush()?;
    Ok(())
}

fn render_cache_build(format: ResolvedOutputFormat, key: &BuildCacheKey) -> Result<()> {
    let mut stdout = io::stdout().lock();
    let record = CacheBuildOutput::new(key);
    match format {
        ResolvedOutputFormat::Jsonl => write_json_line(&record, &mut stdout)?,
        ResolvedOutputFormat::Pretty => {
            let object_key = key.object_key();
            writeln!(stdout, "Uploaded cache {object_key}")?;
        }
    }
    stdout.flush()?;
    Ok(())
}

fn render_cache_apply(format: ResolvedOutputFormat, output: CacheApplyOutput) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match format {
        ResolvedOutputFormat::Jsonl => write_json_line(&output, &mut stdout)?,
        ResolvedOutputFormat::Pretty => {
            if output.applied {
                if let Some(key) = output.key.as_ref() {
                    writeln!(stdout, "Applied cache {key}")?;
                }
            } else {
                writeln!(stdout, "No cache entry found to apply.")?;
            }
        }
    }
    stdout.flush()?;
    Ok(())
}

fn write_json_line<T: Serialize>(record: &T, writer: &mut impl Write) -> Result<()> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")?;
    Ok(())
}

fn format_system_time(time: &std::time::SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Utc> = (*time).into();
    datetime.to_rfc3339()
}

#[derive(Debug, Serialize)]
struct CachePathOutput {
    path: String,
}

#[derive(Debug, Serialize)]
struct CacheListOutput {
    key: String,
    last_modified: Option<String>,
}

impl CacheListOutput {
    fn from_entry(entry: &BuildCacheEntry) -> Self {
        Self {
            key: entry.key.clone(),
            last_modified: entry.last_modified.as_ref().map(format_system_time),
        }
    }
}

#[derive(Debug, Serialize)]
struct CacheBuildOutput {
    key: String,
    repo_name: String,
    git_sha: String,
}

impl CacheBuildOutput {
    fn new(key: &BuildCacheKey) -> Self {
        Self {
            key: key.object_key(),
            repo_name: key.repo_name.as_str(),
            git_sha: key.git_sha.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct CacheApplyOutput {
    applied: bool,
    key: Option<String>,
    git_sha: Option<String>,
    distance: Option<usize>,
}

impl CacheApplyOutput {
    fn applied(key: &BuildCacheKey, distance: Option<usize>) -> Self {
        Self {
            applied: true,
            key: Some(key.object_key()),
            git_sha: Some(key.git_sha.clone()),
            distance,
        }
    }

    fn not_found() -> Self {
        Self {
            applied: false,
            key: None,
            git_sha: None,
            distance: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;
    use std::fs;
    use tempfile::tempdir;

    fn commit_file(repo: &Repository, path: &Path, contents: &str) -> String {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dir");
        }
        fs::write(path, contents).expect("write file");

        let mut index = repo.index().expect("index");
        let workdir = repo.workdir().expect("workdir");
        index
            .add_path(path.strip_prefix(workdir).expect("strip prefix"))
            .expect("add path");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("tree");
        let signature = Signature::now("metis", "metis@example.com").expect("signature");
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok());
        let parent_refs = parents.iter().collect::<Vec<_>>();
        let oid = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "commit",
                &tree,
                &parent_refs,
            )
            .expect("commit");
        oid.to_string()
    }

    #[test]
    fn resolve_git_sha_uses_head_when_missing() {
        let dir = tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("repo");
        let workdir = repo.workdir().expect("workdir");
        let head = commit_file(&repo, &workdir.join("README.md"), "test");

        let resolved = resolve_git_sha(workdir, None).expect("resolve");

        assert_eq!(resolved, head);
    }
}
