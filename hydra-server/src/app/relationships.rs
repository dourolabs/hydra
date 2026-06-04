use crate::domain::actors::ActorRef;
use crate::store::{RelationshipType, StoreError};
use hydra_common::HydraId;

use super::AppState;

impl AppState {
    pub async fn add_relationship(
        &self,
        source_id: &HydraId,
        target_id: &HydraId,
        rel_type: RelationshipType,
        actor: ActorRef,
    ) -> Result<bool, StoreError> {
        self.store
            .add_relationship_with_actor(source_id, target_id, rel_type, actor)
            .await
    }

    pub async fn remove_relationship(
        &self,
        source_id: &HydraId,
        target_id: &HydraId,
        rel_type: RelationshipType,
        actor: ActorRef,
    ) -> Result<bool, StoreError> {
        self.store
            .remove_relationship_with_actor(source_id, target_id, rel_type, actor)
            .await
    }
}
