use crate::config::BuildCacheConfig;
use crate::error::BuildCacheError;
use crate::key::BuildCacheKey;
use crate::storage::{StorageClient, StorageObject};
use git2::{ErrorCode, Repository};
use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tar::{Builder, Header};
use walkdir::WalkDir;

/// Build cache archives are written as deterministic `tar.zst` files.

#[derive(Clone)]
pub struct BuildCacheClient {
    config: BuildCacheConfig,
    storage: Arc<dyn StorageClient>,
}

impl BuildCacheClient {
    pub fn new(config: BuildCacheConfig, storage: Arc<dyn StorageClient>) -> Self {
        Self { config, storage }
    }

    pub fn config(&self) -> &BuildCacheConfig {
        &self.config
    }

    pub fn storage(&self) -> &dyn StorageClient {
        self.storage.as_ref()
    }

    pub fn build_cache_archive(
        &self,
        root: impl AsRef<Path>,
        output_path: impl AsRef<Path>,
    ) -> Result<(), BuildCacheError> {
        self.build_cache_archive_impl(root.as_ref(), output_path.as_ref())
    }

    pub fn list_cache_entries(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<Vec<PathBuf>, BuildCacheError> {
        let matcher = self.config.matcher()?;
        let entries = collect_entries(root.as_ref(), &matcher)?;
        Ok(entries
            .into_iter()
            .map(|entry| entry.relative_path)
            .collect())
    }

    pub async fn build_cache_archive_async(
        &self,
        root: PathBuf,
        output_path: PathBuf,
    ) -> Result<(), BuildCacheError> {
        let client = self.clone();
        tokio::task::spawn_blocking(move || client.build_cache_archive(&root, &output_path))
            .await
            .map_err(|err| {
                BuildCacheError::io("joining cache archive task", io::Error::other(err))
            })?
    }

    pub fn apply_cache_archive(
        &self,
        root: impl AsRef<Path>,
        archive_path: impl AsRef<Path>,
    ) -> Result<(), BuildCacheError> {
        self.apply_cache_archive_impl(root.as_ref(), archive_path.as_ref())
    }

    pub async fn apply_cache_archive_async(
        &self,
        root: PathBuf,
        archive_path: PathBuf,
    ) -> Result<(), BuildCacheError> {
        let client = self.clone();
        tokio::task::spawn_blocking(move || client.apply_cache_archive(&root, &archive_path))
            .await
            .map_err(|err| BuildCacheError::io("joining cache apply task", io::Error::other(err)))?
    }

    pub async fn upload_cache(
        &self,
        key: &BuildCacheKey,
        archive_path: impl AsRef<Path>,
    ) -> Result<(), BuildCacheError> {
        self.storage
            .put_object(&key.object_key(), archive_path.as_ref())
            .await?;
        self.evict_if_needed(key.repo_name.clone()).await
    }

    pub async fn download_cache(
        &self,
        key: &BuildCacheKey,
        destination_path: impl AsRef<Path>,
    ) -> Result<(), BuildCacheError> {
        self.storage
            .get_object(&key.object_key(), destination_path.as_ref())
            .await
    }

    pub async fn list_caches(
        &self,
        repo_name: metis_common::RepoName,
    ) -> Result<Vec<BuildCacheEntry>, BuildCacheError> {
        let prefix = BuildCacheKey::new(repo_name, "").repo_prefix();
        let objects = self.storage.list_objects(&prefix).await?;
        Ok(objects.into_iter().map(BuildCacheEntry::from).collect())
    }

    pub async fn evict_if_needed(
        &self,
        repo_name: metis_common::RepoName,
    ) -> Result<(), BuildCacheError> {
        let Some(max_entries) = self.config.max_entries_per_repo else {
            return Ok(());
        };
        let prefix = BuildCacheKey::new(repo_name, "").repo_prefix();
        let mut objects = self.storage.list_objects(&prefix).await?;

        if objects.len() <= max_entries {
            return Ok(());
        }

        objects.sort_by(|a, b| {
            let ordering = a.last_modified.cmp(&b.last_modified);
            if ordering == std::cmp::Ordering::Equal {
                a.key.cmp(&b.key)
            } else {
                ordering
            }
        });

        let evict_count = objects.len().saturating_sub(max_entries);
        for object in objects.into_iter().take(evict_count) {
            self.storage.delete_object(&object.key).await?;
        }
        Ok(())
    }

    pub async fn download_and_apply_cache(
        &self,
        root: impl AsRef<Path>,
        key: &BuildCacheKey,
    ) -> Result<(), BuildCacheError> {
        let temp = tempfile::NamedTempFile::new()
            .map_err(|err| BuildCacheError::io("creating temp cache file", err))?;
        let path = temp.path().to_path_buf();
        self.download_cache(key, &path).await?;
        self.apply_cache_archive_async(root.as_ref().to_path_buf(), path)
            .await?;
        Ok(())
    }

    fn build_cache_archive_impl(
        &self,
        root: &Path,
        output_path: &Path,
    ) -> Result<(), BuildCacheError> {
        let matcher = self.config.matcher()?;
        let entries = collect_entries(root, &matcher)?;

        let output = File::create(output_path)
            .map_err(|err| BuildCacheError::io("creating cache archive", err))?;
        let encoder = zstd::Encoder::new(output, 0)
            .map_err(|err| BuildCacheError::io("initializing zstd encoder", err))?;
        let mut builder = Builder::new(encoder);

        for entry in entries {
            if entry.is_dir {
                append_directory(&mut builder, &entry.relative_path)?;
            } else {
                append_file(&mut builder, &entry.relative_path, &entry.full_path)?;
            }
        }

        let encoder = builder
            .into_inner()
            .map_err(|err| BuildCacheError::io("finalizing tar archive", err))?;
        encoder
            .finish()
            .map_err(|err| BuildCacheError::io("finalizing zstd encoder", err))?;
        Ok(())
    }

    fn apply_cache_archive_impl(
        &self,
        root: &Path,
        archive_path: &Path,
    ) -> Result<(), BuildCacheError> {
        let tracked_paths = collect_tracked_paths(root)?;
        if !tracked_paths.is_empty() {
            assert_archive_safe(archive_path, &tracked_paths)?;
        }

        let input = File::open(archive_path)
            .map_err(|err| BuildCacheError::io("opening cache archive", err))?;
        let decoder = zstd::Decoder::new(input)
            .map_err(|err| BuildCacheError::io("initializing zstd decoder", err))?;
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(root)
            .map_err(|err| BuildCacheError::io("unpacking cache archive", err))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BuildCacheEntry {
    pub key: String,
    pub last_modified: Option<SystemTime>,
}

impl From<StorageObject> for BuildCacheEntry {
    fn from(value: StorageObject) -> Self {
        Self {
            key: value.key,
            last_modified: value.last_modified,
        }
    }
}

#[derive(Debug)]
struct CacheEntry {
    relative_path: PathBuf,
    full_path: PathBuf,
    is_dir: bool,
}

fn collect_entries(
    root: &Path,
    matcher: &crate::config::BuildCacheMatcher,
) -> Result<Vec<CacheEntry>, BuildCacheError> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry
            .map_err(|err| BuildCacheError::io("walking cache paths", io::Error::other(err)))?;
        let path = entry.path();

        if path == root {
            continue;
        }

        let metadata = entry.metadata().map_err(|err| {
            BuildCacheError::io("reading cache entry metadata", io::Error::other(err))
        })?;
        if metadata.file_type().is_symlink() {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .map_err(|err| BuildCacheError::io("computing relative path", io::Error::other(err)))?;

        if !matcher.is_included(relative) {
            continue;
        }

        let is_dir = metadata.is_dir();
        entries.push(CacheEntry {
            relative_path: relative.to_path_buf(),
            full_path: path.to_path_buf(),
            is_dir,
        });
    }

    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(entries)
}

fn append_directory(
    builder: &mut Builder<zstd::Encoder<'_, File>>,
    path: &Path,
) -> Result<(), BuildCacheError> {
    let mut header = Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_mode(0o755);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(0);
    header.set_cksum();

    builder
        .append_data(&mut header, path, io::empty())
        .map_err(|err| BuildCacheError::io("adding directory to archive", err))
}

fn append_file(
    builder: &mut Builder<zstd::Encoder<'_, File>>,
    path: &Path,
    full_path: &Path,
) -> Result<(), BuildCacheError> {
    let mut file = File::open(full_path)
        .map_err(|err| BuildCacheError::io("opening file for archive", err))?;
    let metadata = file
        .metadata()
        .map_err(|err| BuildCacheError::io("reading file metadata", err))?;

    let mut header = Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(metadata.len());
    header.set_mode(default_file_mode(&metadata));
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();

    builder
        .append_data(&mut header, path, &mut file)
        .map_err(|err| BuildCacheError::io("adding file to archive", err))
}

#[cfg(unix)]
fn default_file_mode(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn default_file_mode(_metadata: &std::fs::Metadata) -> u32 {
    0o644
}

fn collect_tracked_paths(root: &Path) -> Result<HashSet<PathBuf>, BuildCacheError> {
    let repo = match Repository::discover(root) {
        Ok(repo) => repo,
        Err(err) if err.code() == ErrorCode::NotFound => {
            return Ok(HashSet::new());
        }
        Err(err) => {
            return Err(BuildCacheError::git("discovering git repository", err));
        }
    };

    let index = repo
        .index()
        .map_err(|err| BuildCacheError::git("reading git index", err))?;
    let mut tracked = HashSet::new();
    for entry in index.iter() {
        let path = PathBuf::from(String::from_utf8_lossy(&entry.path).to_string());
        tracked.insert(path);
    }
    Ok(tracked)
}

fn assert_archive_safe(
    archive_path: &Path,
    tracked_paths: &HashSet<PathBuf>,
) -> Result<(), BuildCacheError> {
    let input = File::open(archive_path)
        .map_err(|err| BuildCacheError::io("opening cache archive for inspection", err))?;
    let decoder = zstd::Decoder::new(input)
        .map_err(|err| BuildCacheError::io("initializing zstd decoder for inspection", err))?;
    let mut archive = tar::Archive::new(decoder);
    let mut conflicts = Vec::new();

    let entries = archive
        .entries()
        .map_err(|err| BuildCacheError::io("reading cache archive entries", err))?;
    for entry in entries {
        let entry = entry.map_err(|err| BuildCacheError::io("reading cache archive entry", err))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|err| BuildCacheError::io("reading cache archive entry path", err))?;
        let normalized = normalize_archive_path(&path)?;
        if tracked_paths.contains(&normalized) {
            conflicts.push(normalized);
        }
    }

    if !conflicts.is_empty() {
        return Err(BuildCacheError::tracked_files(&conflicts));
    }

    Ok(())
}

fn normalize_archive_path(path: &Path) -> Result<PathBuf, BuildCacheError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err(BuildCacheError::io(
                    "normalizing cache archive entry path",
                    io::Error::other("invalid cache archive entry path"),
                ));
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use git2::Repository;
    use std::collections::HashMap;
    use std::io::Write;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    type StoredObject = (Vec<u8>, Option<SystemTime>);

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create dirs");
        }
        let mut file = File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write file");
    }

    fn commit_file(repo: &Repository, path: &Path) {
        let signature = git2::Signature::now("metis", "metis@example.com").expect("signature");
        let workdir = repo.workdir().expect("workdir");
        let relative = path
            .strip_prefix(workdir)
            .expect("path relative to workdir");
        let mut index = repo.index().expect("index");
        index.add_path(relative).expect("add path");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let parent = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok());
        let parents = parent.iter().collect::<Vec<_>>();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "commit",
            &tree,
            &parents,
        )
        .expect("commit");
    }

    #[derive(Debug, Default)]
    struct MockStorageClient {
        objects: Mutex<HashMap<String, StoredObject>>,
    }

    impl MockStorageClient {
        fn new() -> Self {
            Self::default()
        }

        async fn insert_object_with_time(&self, key: &str, last_modified: Option<SystemTime>) {
            let mut objects = self.objects.lock().await;
            objects.insert(key.to_string(), (Vec::new(), last_modified));
        }
    }

    #[async_trait]
    impl StorageClient for MockStorageClient {
        async fn put_object(&self, key: &str, path: &Path) -> Result<(), BuildCacheError> {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|err| BuildCacheError::io("reading mock upload", err))?;
            let mut objects = self.objects.lock().await;
            objects.insert(key.to_string(), (bytes, Some(SystemTime::now())));
            Ok(())
        }

        async fn get_object(&self, key: &str, destination: &Path) -> Result<(), BuildCacheError> {
            let objects = self.objects.lock().await;
            let (bytes, _) = objects.get(key).ok_or_else(|| {
                BuildCacheError::storage("mock download", format!("missing key {key}"))
            })?;
            tokio::fs::write(destination, bytes)
                .await
                .map_err(|err| BuildCacheError::io("writing mock download", err))?;
            Ok(())
        }

        async fn list_objects(&self, prefix: &str) -> Result<Vec<StorageObject>, BuildCacheError> {
            let objects = self.objects.lock().await;
            Ok(objects
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(key, (_, last_modified))| StorageObject {
                    key: key.clone(),
                    last_modified: *last_modified,
                })
                .collect())
        }

        async fn delete_object(&self, key: &str) -> Result<(), BuildCacheError> {
            let mut objects = self.objects.lock().await;
            objects.remove(key);
            Ok(())
        }
    }

    #[test]
    fn roundtrip_build_and_apply() {
        let source_dir = tempdir().expect("source tempdir");
        let cache_dir = tempdir().expect("cache tempdir");
        let destination_dir = tempdir().expect("destination tempdir");

        write_file(&source_dir.path().join("target/debug/lib.a"), "artifact");
        write_file(&source_dir.path().join("target/debug/build.log"), "log");
        write_file(&source_dir.path().join("src/main.rs"), "source");

        let archive_path = cache_dir.path().join("cache.tar.zst");
        let storage = Arc::new(MockStorageClient::new());

        {
            let client = BuildCacheClient::new(BuildCacheConfig::default(), storage.clone());
            client
                .build_cache_archive(source_dir.path(), &archive_path)
                .expect("build archive");
        }

        {
            let client = BuildCacheClient::new(BuildCacheConfig::default(), storage);
            client
                .apply_cache_archive(destination_dir.path(), &archive_path)
                .expect("apply archive");
        }

        let artifact_path = destination_dir.path().join("target/debug/lib.a");
        let log_path = destination_dir.path().join("target/debug/build.log");
        let source_path = destination_dir.path().join("src/main.rs");

        assert!(artifact_path.exists());
        assert!(!log_path.exists());
        assert!(!source_path.exists());

        let contents = std::fs::read_to_string(&artifact_path).expect("read artifact");
        assert_eq!(contents, "artifact");
    }

    #[tokio::test]
    async fn upload_and_download_cache_roundtrip() {
        let temp_dir = tempdir().expect("temp dir");
        let archive_path = temp_dir.path().join("cache.tar.zst");
        let destination_path = temp_dir.path().join("downloaded.tar.zst");
        write_file(&archive_path, "payload");

        let storage = Arc::new(MockStorageClient::new());
        let client = BuildCacheClient::new(BuildCacheConfig::default(), storage);
        let repo = metis_common::RepoName::new("acme", "anvils").expect("repo");
        let key = BuildCacheKey::new(repo, "deadbeef");

        client
            .upload_cache(&key, &archive_path)
            .await
            .expect("upload");
        client
            .download_cache(&key, &destination_path)
            .await
            .expect("download");

        let contents = std::fs::read_to_string(&destination_path).expect("read download");
        assert_eq!(contents, "payload");
    }

    #[tokio::test]
    async fn list_caches_returns_objects_for_repo_prefix() {
        let temp_dir = tempdir().expect("temp dir");
        let archive_path = temp_dir.path().join("cache.tar.zst");
        write_file(&archive_path, "payload");

        let storage = Arc::new(MockStorageClient::new());
        let client = BuildCacheClient::new(BuildCacheConfig::default(), storage.clone());

        let repo = metis_common::RepoName::new("acme", "anvils").expect("repo");
        let other_repo = metis_common::RepoName::new("acme", "balloons").expect("repo");
        let key = BuildCacheKey::new(repo.clone(), "deadbeef");
        let other_key = BuildCacheKey::new(other_repo, "cafebabe");

        client
            .upload_cache(&key, &archive_path)
            .await
            .expect("upload");
        client
            .upload_cache(&other_key, &archive_path)
            .await
            .expect("upload other");

        let listed = client.list_caches(repo).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, key.object_key());
    }

    #[tokio::test]
    async fn download_and_apply_cache_applies_archive() {
        let source_dir = tempdir().expect("source tempdir");
        let cache_dir = tempdir().expect("cache tempdir");
        let destination_dir = tempdir().expect("destination tempdir");

        write_file(&source_dir.path().join("target/debug/lib.a"), "artifact");

        let archive_path = cache_dir.path().join("cache.tar.zst");
        let storage = Arc::new(MockStorageClient::new());
        let repo = metis_common::RepoName::new("acme", "anvils").expect("repo");
        let key = BuildCacheKey::new(repo, "deadbeef");

        {
            let client = BuildCacheClient::new(BuildCacheConfig::default(), storage.clone());
            client
                .build_cache_archive(source_dir.path(), &archive_path)
                .expect("build archive");
            client
                .upload_cache(&key, &archive_path)
                .await
                .expect("upload");
        }

        {
            let client = BuildCacheClient::new(BuildCacheConfig::default(), storage);
            client
                .download_and_apply_cache(destination_dir.path(), &key)
                .await
                .expect("download apply");
        }

        let artifact_path = destination_dir.path().join("target/debug/lib.a");
        assert!(artifact_path.exists());
    }

    #[tokio::test]
    async fn evict_if_needed_removes_oldest_entries() {
        let storage = Arc::new(MockStorageClient::new());
        let repo = metis_common::RepoName::new("acme", "anvils").expect("repo");
        let key1 = BuildCacheKey::new(repo.clone(), "oldest");
        let key2 = BuildCacheKey::new(repo.clone(), "middle");
        let key3 = BuildCacheKey::new(repo.clone(), "newest");

        let time1 = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        let time2 = SystemTime::UNIX_EPOCH + Duration::from_secs(2);
        let time3 = SystemTime::UNIX_EPOCH + Duration::from_secs(3);

        storage
            .insert_object_with_time(&key1.object_key(), Some(time1))
            .await;
        storage
            .insert_object_with_time(&key2.object_key(), Some(time2))
            .await;
        storage
            .insert_object_with_time(&key3.object_key(), Some(time3))
            .await;

        let config = BuildCacheConfig {
            max_entries_per_repo: Some(2),
            ..BuildCacheConfig::default()
        };
        let client = BuildCacheClient::new(config, storage);

        client.evict_if_needed(repo.clone()).await.expect("evict");

        let remaining = client.list_caches(repo).await.expect("list");
        let keys: Vec<String> = remaining.into_iter().map(|entry| entry.key).collect();
        assert_eq!(keys.len(), 2);
        assert!(!keys.contains(&key1.object_key()));
        assert!(keys.contains(&key2.object_key()));
        assert!(keys.contains(&key3.object_key()));
    }

    #[tokio::test]
    async fn upload_cache_triggers_eviction() {
        let temp_dir = tempdir().expect("temp dir");
        let archive_path = temp_dir.path().join("cache.tar.zst");
        write_file(&archive_path, "payload");

        let storage = Arc::new(MockStorageClient::new());
        let repo = metis_common::RepoName::new("acme", "anvils").expect("repo");
        let key1 = BuildCacheKey::new(repo.clone(), "older");
        let key2 = BuildCacheKey::new(repo.clone(), "old");
        let key3 = BuildCacheKey::new(repo.clone(), "new");

        let time1 = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        let time2 = SystemTime::UNIX_EPOCH + Duration::from_secs(2);
        storage
            .insert_object_with_time(&key1.object_key(), Some(time1))
            .await;
        storage
            .insert_object_with_time(&key2.object_key(), Some(time2))
            .await;

        let config = BuildCacheConfig {
            max_entries_per_repo: Some(2),
            ..BuildCacheConfig::default()
        };
        let client = BuildCacheClient::new(config, storage);

        client
            .upload_cache(&key3, &archive_path)
            .await
            .expect("upload");

        let remaining = client.list_caches(repo).await.expect("list");
        let keys: Vec<String> = remaining.into_iter().map(|entry| entry.key).collect();
        assert_eq!(keys.len(), 2);
        assert!(!keys.contains(&key1.object_key()));
        assert!(keys.contains(&key2.object_key()));
        assert!(keys.contains(&key3.object_key()));
    }

    #[test]
    fn apply_cache_archive_rejects_tracked_files() {
        let repo_dir = tempdir().expect("repo dir");
        let repo = Repository::init(repo_dir.path()).expect("init repo");
        let tracked_path = repo_dir.path().join("src/lib.rs");
        write_file(&tracked_path, "tracked");
        commit_file(&repo, &tracked_path);

        let cache_dir = tempdir().expect("cache dir");
        let archive_path = cache_dir.path().join("cache.tar.zst");

        let storage = Arc::new(MockStorageClient::new());
        let config = BuildCacheConfig {
            include: vec!["src/".to_string()],
            exclude: Vec::new(),
            max_entries_per_repo: None,
        };
        let client = BuildCacheClient::new(config, storage);
        client
            .build_cache_archive(repo_dir.path(), &archive_path)
            .expect("build archive");

        let error = client
            .apply_cache_archive(repo_dir.path(), &archive_path)
            .expect_err("expected conflict error");
        match error {
            BuildCacheError::TrackedFiles { .. } => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
