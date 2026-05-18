//! [`DocumentsMount`] — pre-agent `sync_documents` + post-agent
//! `push_documents` against `<dest>/documents`.
//!
//! The mount owns the documents directory: it runs `fs::create_dir_all`
//! at `setup` time. `worker_run::run` is responsible for setting
//! `HYDRA_DOCUMENTS_DIR` in the agent's `execution_env` up front, before
//! the mount runs, so the agent sees the same path the mount targets
//! regardless of whether `setup` succeeds.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;

use crate::client::HydraClientInterface;
use crate::command::documents::{push_documents, sync_documents, PushArgs, SyncArgs};

use super::{Mount, MountError, MountResult, Phase};

/// Per-phase timeout for the pre-agent document sync.
pub const SYNC_DOCUMENTS_TIMEOUT: Duration = Duration::from_secs(60);
/// Per-phase timeout for pushing documents back to hydra-server.
pub const PUSH_DOCUMENTS_TIMEOUT: Duration = Duration::from_secs(120);

pub struct DocumentsMount {
    documents_path: PathBuf,
    client: Arc<dyn HydraClientInterface>,
    synced: bool,
}

impl DocumentsMount {
    pub fn new(documents_path: PathBuf, client: Arc<dyn HydraClientInterface>) -> Self {
        Self {
            documents_path,
            client,
            synced: false,
        }
    }
}

#[async_trait]
impl Mount for DocumentsMount {
    fn setup_phase(&self) -> Phase {
        Phase {
            label: "document sync",
            timeout: Some(SYNC_DOCUMENTS_TIMEOUT),
        }
    }

    fn save_phase(&self) -> Option<Phase> {
        if !self.synced {
            return None;
        }
        Some(Phase {
            label: "document push",
            timeout: Some(PUSH_DOCUMENTS_TIMEOUT),
        })
    }

    async fn setup(&mut self) -> MountResult {
        std::fs::create_dir_all(&self.documents_path)
            .with_context(|| {
                format!(
                    "failed to create documents directory at {}",
                    self.documents_path.display()
                )
            })
            .map_err(MountError::tracked)?;

        let args = SyncArgs {
            directory: Some(self.documents_path.clone()),
            path_prefix: None,
            clean: false,
        };
        match sync_documents(self.client.as_ref(), args).await {
            Ok(()) => {
                self.synced = true;
                Ok(())
            }
            Err(err) => {
                tracing::warn!(
                    target: "hydra::mounts::documents",
                    "document sync failed, continuing without server-synced documents: {err}"
                );
                Ok(())
            }
        }
    }

    async fn save(&mut self) -> MountResult {
        let args = PushArgs {
            directory: Some(self.documents_path.clone()),
            dry_run: false,
            path_prefix: None,
        };
        push_documents(self.client.as_ref(), args)
            .await
            .map_err(MountError::tracked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HydraClient;
    use httpmock::prelude::*;
    use hydra_common::documents::ListDocumentsResponse;
    use reqwest::Client as HttpClient;
    use serde_json::json;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    fn mock_client(server: &MockServer) -> Arc<dyn HydraClientInterface> {
        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())
                .expect("build mock client");
        Arc::new(client)
    }

    fn empty_documents_response() -> ListDocumentsResponse {
        ListDocumentsResponse::new(Vec::new())
    }

    #[tokio::test]
    async fn setup_happy_path_creates_directory_and_sets_synced() {
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200).json_body_obj(&empty_documents_response());
        });
        let client = mock_client(&server);

        let tempdir = tempfile::tempdir().expect("create tempdir");
        let documents_path = tempdir.path().join("documents");
        assert!(
            !documents_path.exists(),
            "precondition: documents_path should not yet exist"
        );

        let mut mount = DocumentsMount::new(documents_path.clone(), client);
        let result = mount.setup().await;

        assert!(result.is_ok(), "setup should succeed: {result:?}");
        assert!(
            documents_path.is_dir(),
            "setup should have created the documents directory"
        );
        list_mock.assert();
        assert!(
            mount.save_phase().is_some(),
            "save_phase should be Some after a successful sync"
        );
    }

    #[tokio::test]
    async fn setup_sync_error_is_warned_and_leaves_synced_false() {
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(500).json_body(json!({ "message": "boom" }));
        });
        let client = mock_client(&server);

        let tempdir = tempfile::tempdir().expect("create tempdir");
        let documents_path = tempdir.path().join("documents");
        let mut mount = DocumentsMount::new(documents_path.clone(), client);

        let result = mount.setup().await;

        assert!(
            result.is_ok(),
            "sync errors are warn-only and must not abort setup: {result:?}"
        );
        assert!(
            documents_path.is_dir(),
            "directory creation should have happened before the sync error"
        );
        list_mock.assert();
        assert!(
            mount.save_phase().is_none(),
            "save_phase must be None when sync failed"
        );
    }

    #[tokio::test]
    async fn setup_mkdir_error_returns_tracked_mount_error() {
        // A file at the parent path makes `create_dir_all` fail.
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let blocker = tempdir.path().join("blocker");
        std::fs::write(&blocker, b"not a directory").expect("write blocker file");
        let documents_path = blocker.join("documents");

        // No mock server is hit because we fail before sync_documents.
        let server = MockServer::start();
        let client = mock_client(&server);

        let mut mount = DocumentsMount::new(documents_path, client);
        let err = mount
            .setup()
            .await
            .expect_err("setup must fail when create_dir_all fails");

        assert!(
            !err.fatal,
            "mkdir failures should be MountError::tracked, not fatal"
        );
        assert!(
            mount.save_phase().is_none(),
            "save_phase must be None when setup failed before sync"
        );
    }

    #[tokio::test]
    async fn save_happy_path_calls_push_documents() {
        let server = MockServer::start();
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200).json_body_obj(&empty_documents_response());
        });
        let client = mock_client(&server);

        let tempdir = tempfile::tempdir().expect("create tempdir");
        let documents_path = tempdir.path().join("documents");
        let mut mount = DocumentsMount::new(documents_path.clone(), client);

        // Drive a successful setup so a manifest is written and `synced` flips
        // to true (push_documents refuses to run without a manifest).
        mount.setup().await.expect("setup");
        assert!(
            mount.save_phase().is_some(),
            "precondition: setup must mark the mount as synced"
        );

        let result = mount.save().await;
        assert!(result.is_ok(), "save should succeed: {result:?}");

        // Both setup and save call list_documents (one each, no docs to fetch).
        list_mock.assert_hits(2);
    }

    #[tokio::test]
    async fn save_error_returns_tracked_mount_error() {
        let server = MockServer::start();
        // First call (setup) returns an empty list; the second call (save)
        // returns 500 so push_documents bubbles an error up. The manifest
        // written by setup still lives on disk, so push_documents proceeds
        // past its manifest check and reaches list_documents — which is the
        // call this test is actually probing.
        let mut list_ok = server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(200).json_body_obj(&empty_documents_response());
        });
        let client = mock_client(&server);

        let tempdir = tempfile::tempdir().expect("create tempdir");
        let documents_path = tempdir.path().join("documents");
        let mut mount = DocumentsMount::new(documents_path.clone(), client);

        mount.setup().await.expect("setup");

        // Replace the mock with a 500 response for the save call.
        list_ok.delete();
        let list_fail = server.mock(|when, then| {
            when.method(GET).path("/v1/documents");
            then.status(500).json_body(json!({ "message": "boom" }));
        });

        let err = mount
            .save()
            .await
            .expect_err("save must propagate push errors as tracked MountError");

        assert!(
            !err.fatal,
            "save errors should be MountError::tracked, not fatal"
        );
        list_fail.assert();
    }
}
