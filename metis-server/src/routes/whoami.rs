use crate::{
    domain::{
        actors::{Actor, ActorId},
        whoami::{ActorIdentity, WhoAmIResponse},
    },
    routes::jobs::ApiError,
};
use axum::{Extension, Json};
use metis_common::api::v1;
use tracing::info;

pub async fn whoami(
    Extension(actor): Extension<Actor>,
) -> Result<Json<v1::whoami::WhoAmIResponse>, ApiError> {
    info!(actor = %actor.name(), "whoami invoked");

    let identity = match actor.actor_id {
        ActorId::Username(username) => ActorIdentity::User {
            username: username.into(),
        },
        ActorId::Task(task_id) => ActorIdentity::Task {
            task_id,
            creator: actor.creator.clone(),
        },
    };

    info!("whoami completed");
    let response: v1::whoami::WhoAmIResponse = WhoAmIResponse::new(identity).into();
    Ok(Json(response))
}
