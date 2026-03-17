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
    };

    info!("whoami completed");
    let response: v1::whoami::WhoAmIResponse = WhoAmIResponse::new(identity).into();
    Ok(Json(response))
}
