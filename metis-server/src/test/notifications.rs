use crate::{
    domain::{notifications::Notification, users::Username},
    store::Store,
    test_utils::{
        spawn_test_server, spawn_test_server_with_state, test_actor, test_client,
        test_state_handles,
    },
};
use metis_common::{
    ActorId, IssueId,
    api::v1::notifications::{ListNotificationsResponse, MarkReadResponse, UnreadCountResponse},
};
use reqwest::StatusCode;
use std::sync::Arc;

fn sample_notification(recipient: ActorId) -> Notification {
    Notification::new(
        recipient,
        None,
        "issue".to_string(),
        IssueId::new().into(),
        1,
        "updated".to_string(),
        "Issue status changed".to_string(),
        None,
        "walk_up".to_string(),
    )
}

async fn insert_notification_for_test_actor(
    store: &Arc<dyn Store>,
) -> metis_common::NotificationId {
    let actor = test_actor();
    let notif = sample_notification(actor.actor_id);
    store.insert_notification(notif).await.unwrap()
}

#[tokio::test]
async fn list_notifications_returns_empty_when_none_exist() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/notifications", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: ListNotificationsResponse = response.json().await?;
    assert!(body.notifications.is_empty());

    Ok(())
}

#[tokio::test]
async fn list_notifications_returns_notifications_for_actor() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    let id = insert_notification_for_test_actor(&store).await;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/notifications", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: ListNotificationsResponse = response.json().await?;
    assert_eq!(body.notifications.len(), 1);
    assert_eq!(body.notifications[0].notification_id, id);
    assert!(!body.notifications[0].notification.is_read);

    Ok(())
}

#[tokio::test]
async fn list_notifications_excludes_other_actors() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    // Insert notification for a different actor
    let other_recipient = ActorId::Username(Username::from("other-user").into());
    let notif = sample_notification(other_recipient);
    store.insert_notification(notif).await?;

    // Insert one for the test actor
    insert_notification_for_test_actor(&store).await;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/notifications", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: ListNotificationsResponse = response.json().await?;
    assert_eq!(body.notifications.len(), 1);

    Ok(())
}

#[tokio::test]
async fn list_notifications_filters_by_is_read() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    let id1 = insert_notification_for_test_actor(&store).await;
    let _id2 = insert_notification_for_test_actor(&store).await;

    // Mark the first as read
    store.mark_notification_read(&id1).await?;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();

    // Query unread only
    let response = client
        .get(format!(
            "{}/v1/notifications?is_read=false",
            server.base_url()
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: ListNotificationsResponse = response.json().await?;
    assert_eq!(body.notifications.len(), 1);
    assert!(!body.notifications[0].notification.is_read);

    // Query read only
    let response = client
        .get(format!(
            "{}/v1/notifications?is_read=true",
            server.base_url()
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: ListNotificationsResponse = response.json().await?;
    assert_eq!(body.notifications.len(), 1);
    assert!(body.notifications[0].notification.is_read);

    Ok(())
}

#[tokio::test]
async fn unread_count_returns_correct_count() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    insert_notification_for_test_actor(&store).await;
    insert_notification_for_test_actor(&store).await;
    let id3 = insert_notification_for_test_actor(&store).await;

    // Mark one as read
    store.mark_notification_read(&id3).await?;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();

    let response = client
        .get(format!(
            "{}/v1/notifications/unread-count",
            server.base_url()
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: UnreadCountResponse = response.json().await?;
    assert_eq!(body.count, 2);

    Ok(())
}

#[tokio::test]
async fn mark_read_marks_single_notification() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    let id = insert_notification_for_test_actor(&store).await;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();

    let response = client
        .post(format!(
            "{}/v1/notifications/{}/read",
            server.base_url(),
            id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: MarkReadResponse = response.json().await?;
    assert_eq!(body.marked, 1);

    // Verify it's now read
    let response = client
        .get(format!(
            "{}/v1/notifications?is_read=true",
            server.base_url()
        ))
        .send()
        .await?;

    let body: ListNotificationsResponse = response.json().await?;
    assert_eq!(body.notifications.len(), 1);
    assert!(body.notifications[0].notification.is_read);

    Ok(())
}

#[tokio::test]
async fn mark_read_returns_404_for_other_actors_notification() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    // Insert notification for a different actor
    let other_recipient = ActorId::Username(Username::from("other-user").into());
    let notif = sample_notification(other_recipient);
    let id = store.insert_notification(notif).await?;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();

    let response = client
        .post(format!(
            "{}/v1/notifications/{}/read",
            server.base_url(),
            id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn mark_read_returns_404_for_nonexistent_notification() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .post(format!(
            "{}/v1/notifications/nf-doesnotexist/read",
            server.base_url()
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn mark_all_read_marks_all_notifications() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();

    insert_notification_for_test_actor(&store).await;
    insert_notification_for_test_actor(&store).await;
    insert_notification_for_test_actor(&store).await;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();

    let response = client
        .post(format!("{}/v1/notifications/read-all", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: MarkReadResponse = response.json().await?;
    assert_eq!(body.marked, 3);

    // Verify all are now read
    let response = client
        .get(format!(
            "{}/v1/notifications/unread-count",
            server.base_url()
        ))
        .send()
        .await?;

    let body: UnreadCountResponse = response.json().await?;
    assert_eq!(body.count, 0);

    Ok(())
}

#[tokio::test]
async fn mark_all_read_returns_zero_when_none_unread() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .post(format!("{}/v1/notifications/read-all", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: MarkReadResponse = response.json().await?;
    assert_eq!(body.marked, 0);

    Ok(())
}
