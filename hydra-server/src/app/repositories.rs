use crate::{
    domain::actors::ActorRef,
    store::{ReadOnlyStore, StoreError},
};
use hydra_common::{RepoName, api::v1::repositories::SearchRepositoriesQuery};

use super::app_state::AppState;
use super::{Repository, RepositoryError, RepositoryRecord};

impl AppState {
    pub async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<RepositoryRecord>, RepositoryError> {
        let store = self.store.as_ref();
        let repositories = store
            .list_repositories(query)
            .await
            .map_err(|source| RepositoryError::Store { source })?;

        Ok(repositories
            .into_iter()
            .map(|(name, repository)| RepositoryRecord::from((name, repository.item)))
            .collect())
    }

    pub async fn delete_repository(
        &self,
        name: &RepoName,
        actor: ActorRef,
    ) -> Result<RepositoryRecord, RepositoryError> {
        // Get the repository before deleting to return it
        // Use include_deleted: true since we need to access the repository to mark it as deleted
        let current =
            self.store
                .get_repository(name, true)
                .await
                .map_err(|source| match source {
                    StoreError::RepositoryNotFound(_) => RepositoryError::NotFound(name.clone()),
                    other => RepositoryError::Store { source: other },
                })?;

        self.store
            .delete_repository(name, actor)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => RepositoryError::NotFound(name.clone()),
                other => RepositoryError::Store { source: other },
            })?;

        self.service_state.clear_cache(name).await;

        let mut deleted_repo = current.item;
        deleted_repo.deleted = true;
        Ok(RepositoryRecord::from((name.clone(), deleted_repo)))
    }

    pub async fn create_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: ActorRef,
    ) -> Result<RepositoryRecord, RepositoryError> {
        self.store
            .add_repository(name.clone(), config.clone(), actor)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryAlreadyExists(name) => RepositoryError::AlreadyExists(name),
                other => RepositoryError::Store { source: other },
            })?;

        Ok(RepositoryRecord::from((name, config)))
    }

    pub async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: ActorRef,
    ) -> Result<RepositoryRecord, RepositoryError> {
        self.store
            .update_repository(name.clone(), config.clone(), actor)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => RepositoryError::NotFound(name.clone()),
                StoreError::RepositoryAlreadyExists(_) => {
                    RepositoryError::AlreadyExists(name.clone())
                }
                other => RepositoryError::Store { source: other },
            })?;

        self.service_state.clear_cache(&name).await;

        Ok(RepositoryRecord::from((name, config)))
    }

    pub async fn repository_from_store(&self, name: &RepoName) -> Result<Repository, StoreError> {
        let store = self.store.as_ref();
        // Use include_deleted: false since API callers should not see deleted repositories
        store
            .get_repository(name, false)
            .await
            .map(|repo| repo.item)
    }
}
