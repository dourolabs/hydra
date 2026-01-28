use crate::config::{FileSystemStorageConfig, S3StorageConfig};
use crate::error::BuildCacheError;
use async_trait::async_trait;
use aws_credential_types::Credentials;
use aws_credential_types::provider::SharedCredentialsProvider;
use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream;
use aws_types::region::Region;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct StorageObject {
    pub key: String,
    pub last_modified: Option<SystemTime>,
}

#[async_trait]
pub trait StorageClient: Send + Sync {
    async fn put_object(&self, key: &str, path: &Path) -> Result<(), BuildCacheError>;
    async fn get_object(&self, key: &str, destination: &Path) -> Result<(), BuildCacheError>;
    async fn list_objects(&self, prefix: &str) -> Result<Vec<StorageObject>, BuildCacheError>;
    async fn delete_object(&self, key: &str) -> Result<(), BuildCacheError>;
}

#[derive(Debug, Clone)]
pub struct S3StorageClient {
    client: Client,
    bucket: String,
}

impl S3StorageClient {
    pub fn new(config: &S3StorageConfig) -> Result<Self, BuildCacheError> {
        config.validate()?;
        let region = Region::new(config.region.clone());
        let credentials_provider = match (
            config.access_key_id.as_ref(),
            config.secret_access_key.as_ref(),
            config.session_token.as_ref(),
        ) {
            (Some(access_key_id), Some(secret_access_key), session_token) => {
                let credentials = Credentials::new(
                    access_key_id,
                    secret_access_key,
                    session_token.cloned(),
                    None,
                    "metis-build-cache",
                );
                Some(SharedCredentialsProvider::new(credentials))
            }
            _ => None,
        };

        let mut builder = aws_sdk_s3::config::Builder::new()
            .region(region)
            .endpoint_url(&config.endpoint_url);
        if let Some(provider) = credentials_provider {
            builder = builder.credentials_provider(provider);
        }

        let sdk_config = builder.build();

        Ok(Self {
            client: Client::from_conf(sdk_config),
            bucket: config.bucket.clone(),
        })
    }

    fn map_storage_error(context: &'static str, err: impl std::fmt::Display) -> BuildCacheError {
        BuildCacheError::storage(context, err.to_string())
    }

    fn to_system_time(value: &aws_sdk_s3::primitives::DateTime) -> Option<SystemTime> {
        SystemTime::try_from(*value).ok()
    }
}

#[async_trait]
impl StorageClient for S3StorageClient {
    async fn put_object(&self, key: &str, path: &Path) -> Result<(), BuildCacheError> {
        let body = ByteStream::from_path(path)
            .await
            .map_err(|err| Self::map_storage_error("reading upload body", err))?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .send()
            .await
            .map_err(|err| Self::map_storage_error("uploading object", err))?;
        Ok(())
    }

    async fn get_object(&self, key: &str, destination: &Path) -> Result<(), BuildCacheError> {
        let response = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|err| Self::map_storage_error("downloading object", err))?;

        let mut body = response.body.into_async_read();
        let mut file = tokio::fs::File::create(destination)
            .await
            .map_err(|err| BuildCacheError::io("creating download file", err))?;
        tokio::io::copy(&mut body, &mut file)
            .await
            .map_err(|err| BuildCacheError::io("writing download file", err))?;
        Ok(())
    }

    async fn list_objects(&self, prefix: &str) -> Result<Vec<StorageObject>, BuildCacheError> {
        let mut objects = Vec::new();
        let mut continuation = None::<String>;

        loop {
            let mut request = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix);
            if let Some(token) = continuation.as_ref() {
                request = request.continuation_token(token);
            }

            let response = request
                .send()
                .await
                .map_err(|err| Self::map_storage_error("listing objects", err))?;

            for item in response.contents() {
                if let Some(key) = item.key() {
                    let last_modified = item.last_modified().and_then(Self::to_system_time);
                    objects.push(StorageObject {
                        key: key.to_string(),
                        last_modified,
                    });
                }
            }

            continuation = response
                .next_continuation_token()
                .map(|value| value.to_string());
            if continuation.is_none() {
                break;
            }
        }

        Ok(objects)
    }

    async fn delete_object(&self, key: &str) -> Result<(), BuildCacheError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|err| Self::map_storage_error("deleting object", err))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FileSystemStorageClient {
    root_dir: PathBuf,
}

impl FileSystemStorageClient {
    pub fn new(config: &FileSystemStorageConfig) -> Result<Self, BuildCacheError> {
        config.validate()?;
        let root_dir = PathBuf::from(&config.root_dir);
        std::fs::create_dir_all(&root_dir)
            .map_err(|err| BuildCacheError::io("creating storage root", err))?;
        Ok(Self { root_dir })
    }

    fn resolve_path(&self, key: &str) -> Result<PathBuf, BuildCacheError> {
        if key.trim().is_empty() {
            return Err(BuildCacheError::storage(
                "resolving object path",
                "object key must not be empty",
            ));
        }

        let path = Path::new(key);
        if path.is_absolute() {
            return Err(BuildCacheError::storage(
                "resolving object path",
                "object key must be relative",
            ));
        }

        for component in path.components() {
            match component {
                Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                    return Err(BuildCacheError::storage(
                        "resolving object path",
                        "object key contains invalid path segments",
                    ));
                }
                Component::CurDir | Component::Normal(_) => {}
            }
        }

        Ok(self.root_dir.join(path))
    }
}

#[async_trait]
impl StorageClient for FileSystemStorageClient {
    async fn put_object(&self, key: &str, path: &Path) -> Result<(), BuildCacheError> {
        let destination = self.resolve_path(key)?;
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| BuildCacheError::io("creating storage directory", err))?;
        }
        tokio::fs::copy(path, &destination)
            .await
            .map_err(|err| BuildCacheError::io("writing storage object", err))?;
        Ok(())
    }

    async fn get_object(&self, key: &str, destination: &Path) -> Result<(), BuildCacheError> {
        let source = self.resolve_path(key)?;
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| BuildCacheError::io("creating download directory", err))?;
        }
        tokio::fs::copy(&source, destination)
            .await
            .map_err(|err| BuildCacheError::io("reading storage object", err))?;
        Ok(())
    }

    async fn list_objects(&self, prefix: &str) -> Result<Vec<StorageObject>, BuildCacheError> {
        let root_dir = self.root_dir.clone();
        let prefix = prefix.to_string();
        tokio::task::spawn_blocking(move || {
            let base = root_dir.join(&prefix);
            if !base.exists() {
                return Ok(Vec::new());
            }

            let mut objects = Vec::new();
            for entry in WalkDir::new(&base).follow_links(false) {
                let entry = entry.map_err(|err| {
                    BuildCacheError::io("walking storage directories", std::io::Error::other(err))
                })?;
                if !entry.file_type().is_file() {
                    continue;
                }

                let metadata = entry.metadata().map_err(|err| {
                    BuildCacheError::io("reading storage metadata", std::io::Error::other(err))
                })?;
                let last_modified = metadata.modified().ok();
                let key = entry
                    .path()
                    .strip_prefix(&root_dir)
                    .map_err(|err| {
                        BuildCacheError::io("computing storage key", std::io::Error::other(err))
                    })?
                    .to_string_lossy()
                    .replace('\\', "/");

                objects.push(StorageObject { key, last_modified });
            }

            Ok(objects)
        })
        .await
        .map_err(|err| BuildCacheError::io("listing storage objects", std::io::Error::other(err)))?
    }

    async fn delete_object(&self, key: &str) -> Result<(), BuildCacheError> {
        let path = self.resolve_path(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(BuildCacheError::io("deleting storage object", err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create dirs");
        }
        std::fs::write(path, contents.as_bytes()).expect("write file");
    }

    #[tokio::test]
    async fn filesystem_storage_roundtrip() {
        let root = tempdir().expect("root dir");
        let config = FileSystemStorageConfig {
            root_dir: root.path().to_string_lossy().to_string(),
        };
        let client = FileSystemStorageClient::new(&config).expect("client");

        let source_dir = tempdir().expect("source");
        let source_path = source_dir.path().join("cache.tar.zst");
        write_file(&source_path, "payload");

        let key = "repo/acme/anvils/deadbeef/cache.tar.zst";
        client.put_object(key, &source_path).await.expect("put");

        let listed = client
            .list_objects("repo/acme/anvils/")
            .await
            .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key, key);
        assert!(listed[0].last_modified.is_some());

        let destination_dir = tempdir().expect("destination");
        let destination_path = destination_dir.path().join("downloaded.tar.zst");
        client
            .get_object(key, &destination_path)
            .await
            .expect("get");

        let contents = std::fs::read_to_string(&destination_path).expect("read download");
        assert_eq!(contents, "payload");

        client.delete_object(key).await.expect("delete");
        let remaining = client
            .list_objects("repo/acme/anvils/")
            .await
            .expect("list after delete");
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn filesystem_storage_list_missing_prefix_returns_empty() {
        let root = tempdir().expect("root dir");
        let config = FileSystemStorageConfig {
            root_dir: root.path().to_string_lossy().to_string(),
        };
        let client = FileSystemStorageClient::new(&config).expect("client");

        let listed = client
            .list_objects("repo/acme/anvils/")
            .await
            .expect("list");
        assert!(listed.is_empty());
    }
}
