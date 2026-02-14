use std::sync::Arc;

use anyhow::{Context, Result};
use metis_build_cache::{
    BuildCacheClient, BuildCacheConfig, FileSystemStorageClient, S3StorageClient,
};
use metis_common::{BuildCacheContext, BuildCacheStorageConfig};

pub(crate) fn build_cache_client(context: &BuildCacheContext) -> Result<BuildCacheClient> {
    let storage = build_storage_client(&context.storage)?;
    let settings = &context.settings;
    let config = BuildCacheConfig {
        include: settings.include.clone(),
        exclude: settings.exclude.clone(),
        home_include: settings.home_include.clone(),
        home_exclude: settings.home_exclude.clone(),
        max_entries_per_repo: settings.max_entries_per_repo,
    };
    Ok(BuildCacheClient::new(config, storage))
}

fn build_storage_client(
    storage: &BuildCacheStorageConfig,
) -> Result<Arc<dyn metis_build_cache::StorageClient>> {
    match storage {
        BuildCacheStorageConfig::FileSystem { root_dir } => {
            let config = metis_build_cache::FileSystemStorageConfig {
                root_dir: root_dir.clone(),
            };
            let client = FileSystemStorageClient::new(&config)
                .context("initializing filesystem build cache storage")?;
            Ok(Arc::new(client))
        }
        BuildCacheStorageConfig::S3 {
            endpoint_url,
            bucket,
            region,
            access_key_id,
            secret_access_key,
            session_token,
        } => {
            let config = metis_build_cache::S3StorageConfig {
                endpoint_url: endpoint_url.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
                access_key_id: access_key_id.clone(),
                secret_access_key: secret_access_key.clone(),
                session_token: session_token.clone(),
            };
            let client =
                S3StorageClient::new(&config).context("initializing s3 build cache storage")?;
            Ok(Arc::new(client))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Repository;
    use metis_common::{BuildCacheContext, BuildCacheSettings, BuildCacheStorageConfig, RepoName};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn init_repo(path: &Path) -> Repository {
        let repo = Repository::init(path).expect("init repo");
        let signature = repo
            .signature()
            .or_else(|_| git2::Signature::now("metis", "metis@example.com"))
            .expect("signature");
        fs::write(path.join("README.md"), "hello").expect("write file");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("README.md")).expect("add path");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("tree");
        repo.commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])
            .expect("commit");
        drop(tree);
        repo
    }

    #[tokio::test]
    async fn build_cache_roundtrip_applies_nearest_cache() {
        let repo_dir = tempdir().expect("repo dir");
        let repo = init_repo(repo_dir.path());
        let repo_root = repo_dir.path();
        let target_dir = repo_root.join("target");
        fs::create_dir_all(&target_dir).expect("create target");
        fs::write(target_dir.join("artifact.txt"), "cached").expect("write cache file");

        let storage_dir = tempdir().expect("storage dir");
        let context = BuildCacheContext {
            storage: BuildCacheStorageConfig::FileSystem {
                root_dir: storage_dir.path().to_string_lossy().to_string(),
            },
            settings: BuildCacheSettings::default(),
        };

        let repo_name = RepoName::new("acme", "widgets").expect("repo name");
        let git_sha = repo
            .head()
            .expect("head")
            .target()
            .expect("oid")
            .to_string();
        let client = build_cache_client(&context).expect("cache client");
        let (_key, _timings) = client
            .build_and_upload_cache(repo_root, None, repo_name.clone(), &git_sha)
            .await
            .expect("upload cache");

        fs::remove_dir_all(&target_dir).expect("remove target");
        let (applied, _timings) = client
            .apply_nearest_cache(repo_root, None, repo_name.clone())
            .await
            .expect("apply cache");
        assert!(applied.is_some());
        let restored =
            fs::read_to_string(target_dir.join("artifact.txt")).expect("read restored file");
        assert_eq!(restored, "cached");
    }
}
