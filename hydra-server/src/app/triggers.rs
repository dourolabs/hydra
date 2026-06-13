//! `AppState` methods that own the request validation + write path for
//! `/v1/triggers`. The route handlers in `routes/triggers.rs` are thin
//! wrappers; everything that combines request body with store state
//! (template / cron / schedule self-consistency, repo-existence lookups,
//! and the actual store write) lives here.

use super::{AppState, StoreWithEvents};
use crate::domain::actors::ActorRef;
use crate::domain::triggers::{
    TriggerValidation, ValidationError, ValidationWarning, referenced_repos,
};
use crate::store::{ReadOnlyStore, StoreError};
use hydra_common::triggers::Trigger;
use hydra_common::{RepoName, TriggerId, Versioned};
use hydra_common::{VersionNumber, api::v1::triggers::UpsertTriggerRequest};
use thiserror::Error;

/// Failure modes for [`AppState::create_trigger`] and
/// [`AppState::update_trigger`].
#[derive(Debug, Error)]
pub enum UpsertTriggerError {
    #[error("trigger validation failed: {0}")]
    Validation(#[from] ValidationError),
    #[error("repository '{0}' is not registered")]
    UnknownRepo(RepoName),
    #[error("store error: {source}")]
    Store {
        #[source]
        source: StoreError,
    },
}

impl From<StoreError> for UpsertTriggerError {
    fn from(source: StoreError) -> Self {
        UpsertTriggerError::Store { source }
    }
}

impl AppState {
    /// Convenience for callers (e.g. route handlers) that need a
    /// reference to the inner `StoreWithEvents` for mutation calls that
    /// take `&StoreWithEvents` directly (`Action::run` is the canonical
    /// example; trigger CRUD goes through the methods on this impl).
    pub(crate) fn store_with_events(&self) -> &StoreWithEvents {
        &self.store
    }

    /// Validate a `CreateTriggerRequest`, verify every referenced
    /// `repo_name` exists, then `Store::add_trigger`. Returns the new
    /// id, version, and any non-fatal warnings.
    pub async fn create_trigger(
        &self,
        request: UpsertTriggerRequest,
        actor: &ActorRef,
    ) -> Result<(TriggerId, VersionNumber, Vec<ValidationWarning>), UpsertTriggerError> {
        let trigger = trigger_from_request(request);
        let warnings = self.validate_trigger(&trigger).await?;
        let (id, version) = self.store.add_trigger(trigger, actor).await?;
        Ok((id, version, warnings))
    }

    /// Validate a `PUT` payload, verify every referenced `repo_name`
    /// exists, then `Store::update_trigger`. The store's
    /// `update_trigger` is responsible for carrying `last_fired_at`
    /// forward from the latest row, so the caller's payload need not
    /// (and cannot) set it.
    pub async fn update_trigger(
        &self,
        id: &TriggerId,
        request: UpsertTriggerRequest,
        actor: &ActorRef,
    ) -> Result<(VersionNumber, Vec<ValidationWarning>), UpsertTriggerError> {
        let trigger = trigger_from_request(request);
        let warnings = self.validate_trigger(&trigger).await?;
        let version = self.store.update_trigger(id, trigger, actor).await?;
        Ok((version, warnings))
    }

    /// Soft-delete a trigger via the store. Returns the post-deletion
    /// `Versioned<Trigger>` (fetched with `include_archived = true`) so the
    /// caller can populate a response that reflects the new tombstone row.
    pub async fn archive_trigger(
        &self,
        id: &TriggerId,
        actor: &ActorRef,
    ) -> Result<Versioned<Trigger>, UpsertTriggerError> {
        self.store.archive_trigger(id, actor).await?;
        let versioned = self.store.get_trigger(id, true).await?;
        Ok(versioned)
    }

    /// Self-consistency check on the trigger payload, plus a targeted
    /// `Store::get_repository` lookup for each repo name referenced by
    /// the trigger's actions. Returns the non-fatal warnings produced
    /// by validation on success.
    async fn validate_trigger(
        &self,
        trigger: &Trigger,
    ) -> Result<Vec<ValidationWarning>, UpsertTriggerError> {
        let warnings = trigger.validate()?;
        for repo in referenced_repos(trigger) {
            match self.store.get_repository(&repo, false).await {
                Ok(_) => {}
                Err(StoreError::RepositoryNotFound(name)) => {
                    return Err(UpsertTriggerError::UnknownRepo(name));
                }
                Err(source) => return Err(UpsertTriggerError::Store { source }),
            }
        }
        Ok(warnings)
    }
}

fn trigger_from_request(request: UpsertTriggerRequest) -> Trigger {
    Trigger::new(
        request.enabled,
        request.schedule,
        request.actions,
        request.creator,
        // `last_fired_at` is never set from the request body — the store
        // carries it forward from the latest row inside the write tx.
        None,
        false,
    )
}
