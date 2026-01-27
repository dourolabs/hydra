use crate::config::S3StorageConfig;
use crate::error::BuildCacheError;
use async_trait::async_trait;
use aws_credential_types::Credentials;
use aws_credential_types::provider::SharedCredentialsProvider;
use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream;
use aws_types::region::Region;
use std::path::Path;
use std::time::SystemTime;

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
