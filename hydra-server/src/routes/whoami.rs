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
        // Phase-1 ActorId additions (User/Agent/Adhoc/External/Legacy)
        // are not constructed by any hydra-server call site yet — Phases
        // 2–6 of `/designs/actor-system-overhaul.md` migrate handlers
        // one variant at a time. Until then, an authenticated token
        // backed by one of these new variants would indicate a
        // protocol-level bug.
        other => {
            return Err(ApiError::internal(format!(
                "phase-1 invariant violated: authenticated actor has unsupported variant {other:?}"
            )));
        }
    };

    info!("whoami completed");
    let response: v1::whoami::WhoAmIResponse = WhoAmIResponse::new(identity).into();
    Ok(Json(response))
}
