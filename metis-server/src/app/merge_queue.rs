use crate::{
    domain::patches::Patch,
    store::{ReadOnlyStore, StoreError},
};
use metis_common::{PatchId, RepoName, merge_queues::MergeQueue};

use super::MergeQueueError;
use super::app_state::AppState;

impl AppState {
    pub async fn merge_queue(
        &self,
        service_repo_name: &RepoName,
        branch_name: &str,
    ) -> Result<MergeQueue, MergeQueueError> {
        let config = self
            .repository_from_store(service_repo_name)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => {
                    MergeQueueError::UnknownRepository(service_repo_name.clone())
                }
                other => MergeQueueError::RepositoryLookup {
                    repo_name: service_repo_name.clone(),
                    source: other,
                },
            })?;

        self.service_state
            .ensure_cached(service_repo_name, &config)
            .await?;
        self.service_state
            .get_merge_queue(service_repo_name, &config, branch_name)
            .await
    }

    pub async fn enqueue_merge_queue_patch(
        &self,
        service_repo_name: &RepoName,
        branch_name: &str,
        patch_id: PatchId,
    ) -> Result<MergeQueue, MergeQueueError> {
        let config = self
            .repository_from_store(service_repo_name)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => {
                    MergeQueueError::UnknownRepository(service_repo_name.clone())
                }
                other => MergeQueueError::RepositoryLookup {
                    repo_name: service_repo_name.clone(),
                    source: other,
                },
            })?;

        let patch = self.load_patch(patch_id.clone()).await?;

        self.service_state
            .ensure_cached(service_repo_name, &config)
            .await?;
        self.service_state
            .add_patch_to_merge_queue(service_repo_name, &config, branch_name, patch_id, &patch)
            .await
    }

    async fn load_patch(&self, patch_id: PatchId) -> Result<Patch, MergeQueueError> {
        let store = self.store.as_ref();
        match store.get_patch(&patch_id, false).await {
            Ok(patch) => Ok(patch.item),
            Err(StoreError::PatchNotFound(_)) => Err(MergeQueueError::PatchNotFound { patch_id }),
            Err(source) => Err(MergeQueueError::PatchLookup { patch_id, source }),
        }
    }
}
