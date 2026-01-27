use crate::config::BuildCacheConfig;
use crate::error::BuildCacheError;
use std::env;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use tar::{Builder, Header};
use walkdir::WalkDir;

/// Build cache archives are written as deterministic `tar.zst` files.

#[derive(Debug, Clone)]
pub struct BuildCacheClient {
    config: BuildCacheConfig,
}

impl BuildCacheClient {
    pub fn new(config: BuildCacheConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &BuildCacheConfig {
        &self.config
    }

    pub fn build_cache_archive(
        &self,
        output_path: impl AsRef<Path>,
    ) -> Result<(), BuildCacheError> {
        let root =
            env::current_dir().map_err(|err| BuildCacheError::io("reading current dir", err))?;
        self.build_cache_archive_in_dir(&root, output_path.as_ref())
    }

    pub async fn build_cache_archive_async(
        &self,
        output_path: PathBuf,
    ) -> Result<(), BuildCacheError> {
        let client = self.clone();
        tokio::task::spawn_blocking(move || client.build_cache_archive(output_path))
            .await
            .map_err(|err| {
                BuildCacheError::io("joining cache archive task", io::Error::other(err))
            })?
    }

    pub fn apply_cache_archive(
        &self,
        archive_path: impl AsRef<Path>,
    ) -> Result<(), BuildCacheError> {
        let root =
            env::current_dir().map_err(|err| BuildCacheError::io("reading current dir", err))?;
        self.apply_cache_archive_in_dir(&root, archive_path.as_ref())
    }

    pub async fn apply_cache_archive_async(
        &self,
        archive_path: PathBuf,
    ) -> Result<(), BuildCacheError> {
        let client = self.clone();
        tokio::task::spawn_blocking(move || client.apply_cache_archive(archive_path))
            .await
            .map_err(|err| BuildCacheError::io("joining cache apply task", io::Error::other(err)))?
    }

    fn build_cache_archive_in_dir(
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

    fn apply_cache_archive_in_dir(
        &self,
        root: &Path,
        archive_path: &Path,
    ) -> Result<(), BuildCacheError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    struct DirGuard {
        previous: PathBuf,
    }

    impl DirGuard {
        fn change_to(path: &Path) -> Self {
            let previous = env::current_dir().expect("current dir");
            env::set_current_dir(path).expect("set current dir");
            Self { previous }
        }
    }

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.previous);
        }
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create dirs");
        }
        let mut file = File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write file");
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

        {
            let _guard = DirGuard::change_to(source_dir.path());
            let client = BuildCacheClient::new(BuildCacheConfig::default());
            client
                .build_cache_archive(&archive_path)
                .expect("build archive");
        }

        {
            let _guard = DirGuard::change_to(destination_dir.path());
            let client = BuildCacheClient::new(BuildCacheConfig::default());
            client
                .apply_cache_archive(&archive_path)
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
}
