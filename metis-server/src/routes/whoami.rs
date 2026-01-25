use crate::{
    domain::{
        actors::{Actor, UserOrWorker},
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

    let identity = match actor.user_or_worker {
        UserOrWorker::Username(username) => ActorIdentity::User { username },
        UserOrWorker::Task(task_id) => ActorIdentity::Job { job_id: task_id },
    };

    info!("whoami completed");
    let response: v1::whoami::WhoAmIResponse = WhoAmIResponse::new(identity).into();
    Ok(Json(response))
}
