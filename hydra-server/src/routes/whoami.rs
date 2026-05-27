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

    let identity = match actor.actor_id {
        ActorId::Username(username) => ActorIdentity::User {
            username: username.into(),
        },
        ActorId::Session(session_id) => ActorIdentity::Session {
            session_id,
            creator: actor.creator.clone(),
        },
        ActorId::Issue(issue_id) => ActorIdentity::Issue {
            issue_id,
            creator: actor.creator.clone(),
        },
        ActorId::Service(service_name) => ActorIdentity::Service { service_name },
        ActorId::Agent(name) => ActorIdentity::Agent {
            name,
            creator: actor.creator.clone(),
        },
        ActorId::Adhoc(session_id) => ActorIdentity::Adhoc {
            session_id,
            creator: actor.creator.clone(),
        },
        // `User` / `External` are not produced on the authenticated
        // request path (they're login / GitHub-poller flows). `Legacy`
        // is the read-only deserialization catch-all and should never
        // reach an authenticated request path.
        other => {
            return Err(ApiError::internal(format!(
                "whoami invariant violated: authenticated actor has unsupported variant {other:?}"
            )));
        }
    };

    info!("whoami completed");
    let response: v1::whoami::WhoAmIResponse = WhoAmIResponse::new(identity).into();
    Ok(Json(response))
}
