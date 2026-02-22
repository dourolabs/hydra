use crate::{
    domain::{
        issues::{Issue, IssueStatus, IssueType},
        users::Username,
    },
    test_utils::{spawn_test_server, test_client},
};
use metis_common::api::v1::{
    activity::ActivityFeedResponse,
    events::SseEventType,
    issues::{UpsertIssueRequest, UpsertIssueResponse},
};

fn default_user() -> Username {
    Username::from("creator")
}

fn make_issue(description: &str, status: IssueStatus) -> Issue {
    Issue::new(
        IssueType::Task,
        description.to_string(),
        default_user(),
        String::new(),
        status,
        None,
        None,
        Vec::new(),
        vec![],
        Vec::new(),
    )
}

async fn create_issue(
    client: &reqwest::Client,
    base_url: &str,
    description: &str,
) -> anyhow::Result<UpsertIssueResponse> {
    let resp: UpsertIssueResponse = client
        .post(format!("{base_url}/v1/issues"))
        .json(&UpsertIssueRequest::new(
            make_issue(description, IssueStatus::Open).into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(resp)
}

async fn update_issue(
    client: &reqwest::Client,
    base_url: &str,
    issue_id: &str,
    description: &str,
    status: IssueStatus,
) -> anyhow::Result<UpsertIssueResponse> {
    let resp: UpsertIssueResponse = client
        .put(format!("{base_url}/v1/issues/{issue_id}"))
        .json(&UpsertIssueRequest::new(
            make_issue(description, status).into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(resp)
}

async fn fetch_activity(
    client: &reqwest::Client,
    base_url: &str,
    query: &str,
) -> anyhow::Result<ActivityFeedResponse> {
    let url = if query.is_empty() {
        format!("{base_url}/v1/activity")
    } else {
        format!("{base_url}/v1/activity?{query}")
    };
    let resp: ActivityFeedResponse = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(resp)
}

#[tokio::test]
async fn activity_feed_returns_events_in_reverse_chronological_order() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create two issues sequentially
    let first = create_issue(&client, &base, "first issue").await?;
    let second = create_issue(&client, &base, "second issue").await?;

    let feed = fetch_activity(&client, &base, "").await?;

    // Should have at least 2 events (the two issue creations)
    assert!(
        feed.events.len() >= 2,
        "expected at least 2 events, got {}",
        feed.events.len()
    );

    // Most recent event should be for the second issue
    let latest = &feed.events[0];
    assert_eq!(latest.data.entity_id, second.issue_id.to_string());
    assert_eq!(latest.event_type, SseEventType::IssueCreated);

    // Find the first issue creation event
    let first_event = feed
        .events
        .iter()
        .find(|e| e.data.entity_id == first.issue_id.to_string());
    assert!(first_event.is_some(), "first issue should appear in feed");
    assert_eq!(first_event.unwrap().event_type, SseEventType::IssueCreated);

    Ok(())
}

#[tokio::test]
async fn activity_feed_entity_type_filtering() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create an issue
    create_issue(&client, &base, "test issue for filtering").await?;

    // Filter for only patches — the issue creation should NOT appear
    let feed = fetch_activity(&client, &base, "entity_types=patches").await?;
    let has_issue_event = feed.events.iter().any(|e| e.data.entity_type == "issue");
    assert!(
        !has_issue_event,
        "issue events should be excluded when filtering for patches only"
    );

    // Filter for issues — the issue creation SHOULD appear
    let feed = fetch_activity(&client, &base, "entity_types=issues").await?;
    let has_issue_event = feed.events.iter().any(|e| e.data.entity_type == "issue");
    assert!(
        has_issue_event,
        "issue events should appear when filtering for issues"
    );

    Ok(())
}

#[tokio::test]
async fn activity_feed_cursor_based_pagination() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create 3 issues to generate enough events for pagination with limit=2
    let issue1 = create_issue(&client, &base, "page issue 1").await?;
    let issue2 = create_issue(&client, &base, "page issue 2").await?;
    let _issue3 = create_issue(&client, &base, "page issue 3").await?;

    // Fetch first page with limit=2, filtering only issues
    let page1 = fetch_activity(&client, &base, "limit=2&entity_types=issues").await?;
    assert_eq!(page1.events.len(), 2, "first page should have 2 events");
    assert!(
        page1.next_cursor.is_some(),
        "first page should have a next_cursor"
    );

    // Fetch second page using the cursor
    let cursor = page1.next_cursor.unwrap();
    let page2 = fetch_activity(
        &client,
        &base,
        &format!("limit=2&entity_types=issues&cursor={cursor}"),
    )
    .await?;

    // Page 2 should have the remaining event(s)
    assert!(!page2.events.is_empty(), "second page should have events");

    // Ensure no event appears on both pages
    let page1_ids: Vec<_> = page1
        .events
        .iter()
        .map(|e| (&e.data.entity_id, e.data.version))
        .collect();
    for event in &page2.events {
        assert!(
            !page1_ids.contains(&(&event.data.entity_id, event.data.version)),
            "events should not repeat across pages"
        );
    }

    // The first issue should appear somewhere across both pages
    let all_ids: Vec<_> = page1
        .events
        .iter()
        .chain(page2.events.iter())
        .map(|e| e.data.entity_id.as_str())
        .collect();
    assert!(
        all_ids.contains(&issue1.issue_id.as_ref()),
        "first issue should be in one of the pages"
    );
    assert!(
        all_ids.contains(&issue2.issue_id.as_ref()),
        "second issue should be in one of the pages"
    );

    Ok(())
}

#[tokio::test]
async fn activity_feed_event_type_classification() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create an issue (version 1 → IssueCreated)
    let created = create_issue(&client, &base, "classification issue").await?;
    let issue_id = created.issue_id.to_string();

    // Update the issue (version 2 → IssueUpdated)
    update_issue(
        &client,
        &base,
        &issue_id,
        "updated classification issue",
        IssueStatus::InProgress,
    )
    .await?;

    let feed = fetch_activity(&client, &base, "entity_types=issues").await?;

    // Find events for our issue, sorted by version
    let mut issue_events: Vec<_> = feed
        .events
        .iter()
        .filter(|e| e.data.entity_id == issue_id)
        .collect();
    issue_events.sort_by_key(|e| std::cmp::Reverse(e.data.version));

    assert!(
        issue_events.len() >= 2,
        "should have at least 2 events for the issue"
    );

    // Version 2 should be IssueUpdated (appears first in reverse chronological)
    assert_eq!(issue_events[0].event_type, SseEventType::IssueUpdated);
    assert_eq!(issue_events[0].data.version, 2);

    // Version 1 should be IssueCreated
    assert_eq!(issue_events[1].event_type, SseEventType::IssueCreated);
    assert_eq!(issue_events[1].data.version, 1);

    Ok(())
}

#[tokio::test]
async fn activity_feed_includes_base_objects_for_updates() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Create and then update an issue so we get a version > 1 event
    let created = create_issue(&client, &base, "base object issue").await?;
    let issue_id = created.issue_id.to_string();

    update_issue(
        &client,
        &base,
        &issue_id,
        "updated base object issue",
        IssueStatus::InProgress,
    )
    .await?;

    let feed = fetch_activity(&client, &base, "entity_types=issues").await?;

    // The update event (version 2) should have a corresponding base object (version 1)
    let update_event = feed
        .events
        .iter()
        .find(|e| e.data.entity_id == issue_id && e.data.version == 2);
    assert!(
        update_event.is_some(),
        "should have the update event for the issue"
    );

    let base_key = format!("issue:{issue_id}:1");
    assert!(
        feed.base_objects.contains_key(&base_key),
        "should include base object for version 1, keys: {:?}",
        feed.base_objects.keys().collect::<Vec<_>>()
    );

    // The base object should contain the original issue data
    let base_obj = &feed.base_objects[&base_key];
    assert!(
        base_obj.get("issue").is_some(),
        "base object should have an 'issue' key"
    );

    Ok(())
}

#[tokio::test]
async fn activity_feed_empty_when_no_matching_entities() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let base = server.base_url();

    // Query for documents when none exist — should still return a valid (empty or near-empty) response
    let feed = fetch_activity(&client, &base, "entity_types=documents").await?;
    // Just verify the response is well-formed
    assert!(feed.next_cursor.is_none());

    Ok(())
}
