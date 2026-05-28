use crate::{
    domain::{
        actors::{Actor, ActorId},
        whoami::{ActorIdentity, WhoAmIResponse},
    },
    routes::sessions::ApiError,
};
use axum::{Extension, Json};
use hydra_common::api::v1;
use tracing::info;

pub async fn whoami(
    Extension(actor): Extension<Actor>,
) -> Result<Json<v1::whoami::WhoAmIResponse>, ApiError> {
    info!(actor = %actor.name(), "whoami invoked");

    let identity = match actor.actor_id.clone() {
        ActorId::User(username) => ActorIdentity::User {
            username: username.into(),
        },
        ActorId::Agent(name) => ActorIdentity::Agent {
            name,
            creator: actor.creator.clone(),
        },
        ActorId::Adhoc(session_id) => ActorIdentity::Adhoc {
            session_id,
            creator: actor.creator.clone(),
        },
        // `External` is not produced on the authenticated request path
        // (it's a GitHub-poller / external-source flow).
        other @ ActorId::External { .. } => {
            return Err(ApiError::internal(format!(
                "whoami invariant violated: authenticated actor has unsupported variant {other:?}"
            )));
        }
    };

    info!("whoami completed");
    let response: v1::whoami::WhoAmIResponse = WhoAmIResponse::new(identity).into();
    Ok(Json(response))
}
