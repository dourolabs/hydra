//! In-process aggregators for `/v1/analytics/token_usage/*`.
//!
//! The three compute functions are pure; they take pre-fetched
//! `(SessionId, Session)` rows whose `usage`/`end_time` have already
//! been narrowed to the requested window, and produce the wire
//! responses. The `fetch_sessions_with_usage` helper walks the
//! `list_sessions` primitive and applies the in-process narrowing —
//! `repo_name` resolves through each session's spawning issue's
//! `session_settings`, matching the throughput-handler precedent.

use super::buckets::{bucket_starts, step};
use super::pricing::cost_usd;
use crate::domain::sessions::Session;
use crate::store::{ReadOnlyStore, StoreError};
use chrono::{DateTime, Utc};
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::analytics::{
    AgentCost, AgentSessionCost, BucketGranularity, IssueCost, TokenUsageCostPerAgentResponse,
    TokenUsageOverTimeBucket, TokenUsageOverTimeResponse, TokenUsageTopIssuesByCostResponse,
};
use hydra_common::api::v1::sessions::{SearchSessionsQuery, TokenUsage};
use hydra_common::{IssueId, SessionId};
use std::collections::HashMap;
use tracing::warn;

const TOP_ISSUES_LIMIT: usize = 10;

/// Sessions filtered to (a) have a `TokenUsage`, (b) have an `end_time`,
/// and (c) pass the `repo_name` / `creator` filter. Aggregators take
/// these by reference so unit tests can construct fixtures without a
/// store.
#[derive(Debug, Clone)]
pub struct SessionWithUsage {
    pub session_id: SessionId,
    pub session: Session,
}

impl SessionWithUsage {
    /// Caller guarantees `Some(_)` (filtered in `fetch_sessions_with_usage`).
    fn usage(&self) -> &TokenUsage {
        self.session
            .usage
            .as_ref()
            .expect("SessionWithUsage requires Session.usage = Some(_)")
    }

    /// Caller guarantees `Some(_)` (filtered in `fetch_sessions_with_usage`).
    fn end_time(&self) -> DateTime<Utc> {
        self.session
            .end_time
            .expect("SessionWithUsage requires Session.end_time = Some(_)")
    }

    fn agent_key(&self) -> Option<&AgentName> {
        self.session.agent_config.agent_name.as_ref()
    }
}

/// Fetch sessions with `Some(TokenUsage)` whose `end_time` lands inside
/// `[from, to)`, narrowed by the optional `creator` (matched against
/// `Session.creator` — `creator` on `SearchSessionsQuery` does the same
/// thing store-side) and optional `repo_name` (which lives on the
/// spawning issue's `session_settings`, so we resolve it per session).
///
/// Sessions without `spawned_from` are dropped when a `repo_name`
/// filter is active — there's no issue to read the repo off of. With
/// no `repo_name` filter, those sessions stay in the result.
///
/// Deleted issues / lookup failures count as "doesn't match"; we log
/// and skip rather than 500.
pub async fn fetch_sessions_with_usage(
    store: &dyn ReadOnlyStore,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    repo_name: Option<&str>,
    creator: Option<&str>,
) -> Result<Vec<SessionWithUsage>, StoreError> {
    let mut search = SearchSessionsQuery::default();
    search.creator = creator.map(|s| s.to_string());

    let sessions = store.list_sessions(&search).await?;
    let mut out = Vec::with_capacity(sessions.len());
    let mut issue_repo_cache: HashMap<IssueId, Option<String>> = HashMap::new();

    for (session_id, versioned) in sessions {
        let session = versioned.item;
        if session.usage.is_none() {
            continue;
        }
        let Some(end_time) = session.end_time else {
            continue;
        };
        if end_time < from || end_time >= to {
            continue;
        }

        if let Some(expected_repo) = repo_name {
            let Some(issue_id) = session.spawned_from.as_ref() else {
                continue;
            };
            let cached = match issue_repo_cache.get(issue_id) {
                Some(v) => v.clone(),
                None => {
                    let repo = match store.get_issue(issue_id, false).await {
                        Ok(versioned) => versioned
                            .item
                            .session_settings
                            .repo_name
                            .as_ref()
                            .map(|r| r.to_string()),
                        Err(err) => {
                            warn!(
                                error = %err,
                                issue_id = %issue_id,
                                "token_usage: failed to resolve spawning issue for repo filter; skipping session"
                            );
                            None
                        }
                    };
                    issue_repo_cache.insert(issue_id.clone(), repo.clone());
                    repo
                }
            };
            match cached {
                Some(r) if r == expected_repo => {}
                _ => continue,
            }
        }

        out.push(SessionWithUsage {
            session_id,
            session,
        });
    }
    Ok(out)
}

/// Compute the `token_usage/over_time` series: per bucket, the sum of
/// every `TokenUsage` field across sessions whose `end_time` lands in
/// the bucket. Zero buckets are kept so the frontend gets a dense
/// series.
pub fn compute_token_usage_over_time(
    sessions: &[SessionWithUsage],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: BucketGranularity,
) -> TokenUsageOverTimeResponse {
    let starts = bucket_starts(from, to, bucket);
    if starts.is_empty() {
        return TokenUsageOverTimeResponse::new(Vec::new());
    }
    let step = step(bucket);

    let mut buckets: Vec<TokenUsageOverTimeBucket> = starts
        .iter()
        .map(|s| TokenUsageOverTimeBucket::new(*s, 0, 0, 0, 0))
        .collect();

    let first_start = starts[0];
    let bucket_len = buckets.len();
    let bucket_for = |t: DateTime<Utc>| -> Option<usize> {
        if t < from || t >= to {
            return None;
        }
        let delta = t - first_start;
        let idx = (delta.num_seconds() / step.num_seconds()) as usize;
        if idx >= bucket_len { None } else { Some(idx) }
    };

    for entry in sessions {
        let Some(idx) = bucket_for(entry.end_time()) else {
            continue;
        };
        let usage = entry.usage();
        let b = &mut buckets[idx];
        b.input_tokens = b.input_tokens.saturating_add(usage.input_tokens);
        b.output_tokens = b.output_tokens.saturating_add(usage.output_tokens);
        b.cache_read_input_tokens = b
            .cache_read_input_tokens
            .saturating_add(usage.cache_read_input_tokens);
        b.cache_creation_input_tokens = b
            .cache_creation_input_tokens
            .saturating_add(usage.cache_creation_input_tokens);
    }

    TokenUsageOverTimeResponse::new(buckets)
}

/// Compute the `token_usage/cost_per_agent` response: per agent, the
/// blended-dollar total cost across the window plus the per-session
/// breakdown. Sorted by `total_cost_usd` descending with `agent_name`
/// ascending (and `None` last) as a tie-break so output is deterministic
/// across runs. The per-session list inside each agent is sorted by
/// `cost_usd` descending; that sort is stable over an input-ordered Vec,
/// so it's already deterministic. Ad-hoc sessions
/// (`agent_config.agent_name == None`) are aggregated under a single
/// `None` bucket.
pub fn compute_cost_per_agent(sessions: &[SessionWithUsage]) -> TokenUsageCostPerAgentResponse {
    let mut by_agent: HashMap<Option<AgentName>, Vec<AgentSessionCost>> = HashMap::new();
    for entry in sessions {
        let cost = cost_usd(entry.usage());
        let key = entry.agent_key().cloned();
        by_agent
            .entry(key)
            .or_default()
            .push(AgentSessionCost::new(entry.session_id.clone(), cost));
    }

    let mut agents: Vec<AgentCost> = by_agent
        .into_iter()
        .map(|(name, mut session_costs)| {
            session_costs.sort_by(|a, b| {
                b.cost_usd
                    .partial_cmp(&a.cost_usd)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let total: f64 = session_costs.iter().map(|s| s.cost_usd).sum();
            AgentCost::new(name, total, session_costs)
        })
        .collect();
    agents.sort_by(|a, b| {
        b.total_cost_usd
            .partial_cmp(&a.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| match (&a.agent_name, &b.agent_name) {
                (Some(x), Some(y)) => x.cmp(y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
    });

    TokenUsageCostPerAgentResponse::new(agents)
}

/// Compute the (IssueId, cost, session_count) tuples ranked by cost,
/// truncated to the top [`TOP_ISSUES_LIMIT`]. Title lookups happen at
/// the route layer (the spec calls out direct, single-hop
/// `session.spawned_from == issue_id` attribution — sessions without
/// `spawned_from` are excluded).
pub fn rank_top_issues_by_cost(sessions: &[SessionWithUsage]) -> Vec<(IssueId, f64, u64)> {
    let mut by_issue: HashMap<IssueId, (f64, u64)> = HashMap::new();
    for entry in sessions {
        let Some(issue_id) = entry.session.spawned_from.as_ref() else {
            continue;
        };
        let cost = cost_usd(entry.usage());
        let slot = by_issue.entry(issue_id.clone()).or_insert((0.0, 0));
        slot.0 += cost;
        slot.1 += 1;
    }
    let mut ranked: Vec<(IssueId, f64, u64)> = by_issue
        .into_iter()
        .map(|(id, (c, n))| (id, c, n))
        .collect();
    // Secondary key on `issue_id` asc makes the truncate boundary stable
    // when costs tie — without it, the dropped issue depends on HashMap
    // iteration order.
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(TOP_ISSUES_LIMIT);
    ranked
}

/// Compose the `top_issues_by_cost` wire response from pre-ranked
/// tuples and a `(IssueId -> title)` resolver map. Pure; the route
/// layer is responsible for populating `titles` via per-id
/// `store.get_issue` calls. Issues missing from `titles` are dropped
/// (treats lookup failures or deletions as "no longer attributable").
pub fn compute_top_issues_by_cost(
    ranked: Vec<(IssueId, f64, u64)>,
    titles: &HashMap<IssueId, String>,
) -> TokenUsageTopIssuesByCostResponse {
    let issues = ranked
        .into_iter()
        .filter_map(|(id, cost, count)| {
            titles
                .get(&id)
                .map(|t| IssueCost::new(id, t.clone(), cost, count))
        })
        .collect();
    TokenUsageTopIssuesByCostResponse::new(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sessions::{AgentConfig, Session, SessionMode};
    use crate::domain::task_status::Status;
    use crate::domain::users::Username;
    use crate::routes::sessions::mount_spec_from_create_request;
    use chrono::DateTime;
    use hydra_common::api::v1::agents::AgentName;
    use hydra_common::api::v1::sessions::Bundle;
    use std::collections::HashMap;

    fn dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("rfc3339 timestamp")
            .with_timezone(&Utc)
    }

    fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> TokenUsage {
        TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_input_tokens: cache_read,
            cache_creation_input_tokens: cache_write,
        }
    }

    fn session_with(
        agent: Option<&str>,
        spawned_from: Option<IssueId>,
        end_time: Option<DateTime<Utc>>,
        usage: Option<TokenUsage>,
    ) -> Session {
        let agent_config = AgentConfig {
            agent_name: agent.map(|n| AgentName::try_new(n).expect("valid agent name")),
            model: None,
            system_prompt: None,
            mcp_config: None,
        };
        let mut s = Session::new(
            Username::from("test"),
            spawned_from,
            None,
            agent_config,
            mount_spec_from_create_request(Bundle::None, None),
            None,
            std::collections::HashMap::new(),
            None,
            None,
            None,
            SessionMode::Headless,
            Status::Complete,
            None,
            None,
        );
        s.end_time = end_time;
        s.usage = usage;
        s
    }

    fn entry(
        agent: Option<&str>,
        spawned_from: Option<IssueId>,
        end_time: DateTime<Utc>,
        usage: TokenUsage,
    ) -> SessionWithUsage {
        SessionWithUsage {
            session_id: SessionId::new(),
            session: session_with(agent, spawned_from, Some(end_time), Some(usage)),
        }
    }

    // ----- over_time -----

    #[test]
    fn over_time_empty_window_returns_empty_buckets() {
        let resp = compute_token_usage_over_time(
            &[],
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-10T00:00:00Z"),
            BucketGranularity::Day,
        );
        assert!(resp.buckets.is_empty());
    }

    #[test]
    fn over_time_empty_window_with_nonempty_sessions_returns_dense_zero_series() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let resp = compute_token_usage_over_time(&[], from, to, BucketGranularity::Day);
        assert_eq!(resp.buckets.len(), 3);
        for b in &resp.buckets {
            assert_eq!(b.input_tokens, 0);
            assert_eq!(b.output_tokens, 0);
            assert_eq!(b.cache_read_input_tokens, 0);
            assert_eq!(b.cache_creation_input_tokens, 0);
        }
    }

    #[test]
    fn over_time_sums_per_bucket_by_end_time() {
        let from = dt("2026-05-10T00:00:00Z");
        let to = dt("2026-05-13T00:00:00Z");
        let sessions = vec![
            entry(
                Some("swe"),
                None,
                dt("2026-05-10T01:00:00Z"),
                usage(100, 50, 10, 5),
            ),
            entry(
                Some("swe"),
                None,
                dt("2026-05-10T22:00:00Z"),
                usage(200, 100, 0, 0),
            ),
            entry(
                Some("reviewer"),
                None,
                dt("2026-05-12T05:00:00Z"),
                usage(7, 9, 1, 0),
            ),
            // Out of window — must not count.
            entry(
                Some("swe"),
                None,
                dt("2026-05-09T22:00:00Z"),
                usage(999, 999, 999, 999),
            ),
        ];
        let resp = compute_token_usage_over_time(&sessions, from, to, BucketGranularity::Day);
        assert_eq!(resp.buckets.len(), 3);
        assert_eq!(resp.buckets[0].input_tokens, 300);
        assert_eq!(resp.buckets[0].output_tokens, 150);
        assert_eq!(resp.buckets[0].cache_read_input_tokens, 10);
        assert_eq!(resp.buckets[0].cache_creation_input_tokens, 5);
        assert_eq!(resp.buckets[1].input_tokens, 0);
        assert_eq!(resp.buckets[2].input_tokens, 7);
        assert_eq!(resp.buckets[2].output_tokens, 9);
    }

    // ----- cost_per_agent -----

    #[test]
    fn cost_per_agent_empty_input_returns_empty_response() {
        let resp = compute_cost_per_agent(&[]);
        assert!(resp.agents.is_empty());
    }

    #[test]
    fn cost_per_agent_sorts_by_total_desc_and_aggregates_adhoc_under_none() {
        // swe has two sessions, reviewer one, plus one adhoc.
        let sessions = vec![
            entry(
                Some("swe"),
                None,
                dt("2026-05-10T01:00:00Z"),
                usage(1_000_000, 0, 0, 0),
            ),
            entry(
                Some("swe"),
                None,
                dt("2026-05-10T02:00:00Z"),
                usage(2_000_000, 0, 0, 0),
            ),
            entry(
                Some("reviewer"),
                None,
                dt("2026-05-10T01:00:00Z"),
                usage(500_000, 0, 0, 0),
            ),
            // Ad-hoc: agent_name = None.
            entry(
                None,
                None,
                dt("2026-05-10T03:00:00Z"),
                usage(10_000, 0, 0, 0),
            ),
        ];

        let resp = compute_cost_per_agent(&sessions);
        // Three buckets: swe, reviewer, ad-hoc.
        assert_eq!(resp.agents.len(), 3);

        // Order is sorted desc by total_cost_usd.
        // swe: 3M input * $5/MTok = $15.00
        // reviewer: 500k * $5/MTok = $2.50
        // adhoc: 10k * $5/MTok = $0.05
        assert_eq!(
            resp.agents[0].agent_name.as_ref().map(|n| n.as_str()),
            Some("swe")
        );
        assert!((resp.agents[0].total_cost_usd - 15.0).abs() < 1e-9);
        assert_eq!(resp.agents[0].sessions.len(), 2);
        // The two sessions inside swe are sorted by cost desc.
        assert!(resp.agents[0].sessions[0].cost_usd >= resp.agents[0].sessions[1].cost_usd);

        assert_eq!(
            resp.agents[1].agent_name.as_ref().map(|n| n.as_str()),
            Some("reviewer")
        );
        assert!((resp.agents[1].total_cost_usd - 2.5).abs() < 1e-9);

        assert!(resp.agents[2].agent_name.is_none());
        assert!((resp.agents[2].total_cost_usd - 0.05).abs() < 1e-9);
        assert_eq!(resp.agents[2].sessions.len(), 1);
    }

    #[test]
    fn cost_per_agent_tie_break_uses_agent_name_asc_with_none_last() {
        // Three agents with identical totals; one is ad-hoc (`None`).
        // Expect agent_name ascending, with `None` placed last.
        let sessions = vec![
            entry(
                Some("swe"),
                None,
                dt("2026-05-10T01:00:00Z"),
                usage(1_000_000, 0, 0, 0),
            ),
            entry(
                Some("pm"),
                None,
                dt("2026-05-10T01:00:00Z"),
                usage(1_000_000, 0, 0, 0),
            ),
            entry(
                None,
                None,
                dt("2026-05-10T01:00:00Z"),
                usage(1_000_000, 0, 0, 0),
            ),
        ];
        let resp = compute_cost_per_agent(&sessions);
        assert_eq!(resp.agents.len(), 3);
        assert_eq!(
            resp.agents[0].agent_name.as_ref().map(|n| n.as_str()),
            Some("pm")
        );
        assert_eq!(
            resp.agents[1].agent_name.as_ref().map(|n| n.as_str()),
            Some("swe")
        );
        assert!(resp.agents[2].agent_name.is_none());
    }

    // ----- top_issues_by_cost -----

    #[test]
    fn top_issues_truncates_at_ten_when_more_qualify() {
        // 11 issues, each with a single session whose cost varies by
        // input-token count. The cheapest should be dropped.
        let mut sessions = Vec::new();
        let mut all_ids = Vec::new();
        for i in 0..11u64 {
            let id = IssueId::new();
            all_ids.push(id.clone());
            sessions.push(entry(
                Some("swe"),
                Some(id),
                dt("2026-05-10T01:00:00Z"),
                usage((i + 1) * 1_000_000, 0, 0, 0),
            ));
        }
        let ranked = rank_top_issues_by_cost(&sessions);
        assert_eq!(ranked.len(), 10);
        // Highest input count was issue 11 (last id); it should be #1.
        assert_eq!(&ranked[0].0, all_ids.last().unwrap());
        // Cheapest issue (i=0, cost = 1M * $5/MTok = $5) should be the
        // one dropped — it isn't present in the ranked output.
        assert!(!ranked.iter().any(|(id, _, _)| id == &all_ids[0]));

        // Resolver map gives titles to the present ids.
        let titles: HashMap<IssueId, String> = ranked
            .iter()
            .map(|(id, _, _)| (id.clone(), format!("issue-{id}")))
            .collect();
        let resp = compute_top_issues_by_cost(ranked, &titles);
        assert_eq!(resp.issues.len(), 10);
        // First entry's cost is the largest.
        for window in resp.issues.windows(2) {
            assert!(window[0].cost_usd >= window[1].cost_usd);
        }
    }

    #[test]
    fn top_issues_tie_break_uses_issue_id_asc() {
        use std::str::FromStr;
        // Three issues at identical costs; expect issue_id ascending order.
        let id_c = IssueId::from_str("i-cccccc").expect("valid id");
        let id_a = IssueId::from_str("i-aaaaaa").expect("valid id");
        let id_b = IssueId::from_str("i-bbbbbb").expect("valid id");
        let sessions = vec![
            entry(
                Some("swe"),
                Some(id_c.clone()),
                dt("2026-05-10T01:00:00Z"),
                usage(1_000_000, 0, 0, 0),
            ),
            entry(
                Some("swe"),
                Some(id_a.clone()),
                dt("2026-05-10T01:00:00Z"),
                usage(1_000_000, 0, 0, 0),
            ),
            entry(
                Some("swe"),
                Some(id_b.clone()),
                dt("2026-05-10T01:00:00Z"),
                usage(1_000_000, 0, 0, 0),
            ),
        ];
        let ranked = rank_top_issues_by_cost(&sessions);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].0, id_a);
        assert_eq!(ranked[1].0, id_b);
        assert_eq!(ranked[2].0, id_c);
    }

    #[test]
    fn top_issues_drops_sessions_without_spawned_from() {
        let id = IssueId::new();
        let sessions = vec![
            entry(
                Some("swe"),
                Some(id.clone()),
                dt("2026-05-10T01:00:00Z"),
                usage(1_000_000, 0, 0, 0),
            ),
            entry(
                Some("swe"),
                None,
                dt("2026-05-10T01:00:00Z"),
                usage(9_000_000, 0, 0, 0),
            ),
        ];
        let ranked = rank_top_issues_by_cost(&sessions);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].0, id);
        assert_eq!(ranked[0].2, 1);
    }

    #[test]
    fn top_issues_filters_to_known_titles_only() {
        let id_a = IssueId::new();
        let id_b = IssueId::new();
        let sessions = vec![
            entry(
                Some("swe"),
                Some(id_a.clone()),
                dt("2026-05-10T01:00:00Z"),
                usage(1_000_000, 0, 0, 0),
            ),
            entry(
                Some("swe"),
                Some(id_b.clone()),
                dt("2026-05-10T01:00:00Z"),
                usage(2_000_000, 0, 0, 0),
            ),
        ];
        let ranked = rank_top_issues_by_cost(&sessions);
        let mut titles = HashMap::new();
        titles.insert(id_a.clone(), "A".to_string());
        // id_b has no title (simulating a deleted issue) — dropped.
        let resp = compute_top_issues_by_cost(ranked, &titles);
        assert_eq!(resp.issues.len(), 1);
        assert_eq!(resp.issues[0].issue_id, id_a);
    }

    // ----- fetch helper -----

    #[tokio::test]
    async fn fetch_filters_to_sessions_with_usage_and_end_time_in_window() {
        use crate::test_utils::test_state_handles;
        use hydra_common::ActorRef as CommonActorRef;

        let handles = test_state_handles();
        let store = handles.store.clone();
        let actor = CommonActorRef::test();

        // A session with usage and end_time in window — kept.
        let mut keep = session_with(
            Some("swe"),
            None,
            Some(dt("2026-05-11T00:00:00Z")),
            Some(usage(100, 50, 0, 0)),
        );
        keep.creation_time = Some(dt("2026-05-10T00:00:00Z"));
        let (keep_id, _) = store
            .add_session(keep, dt("2026-05-10T00:00:00Z"), &actor)
            .await
            .expect("add keep");

        // A session with usage but no end_time — dropped.
        let no_end = session_with(Some("swe"), None, None, Some(usage(100, 50, 0, 0)));
        store
            .add_session(no_end, dt("2026-05-10T00:00:00Z"), &actor)
            .await
            .expect("add no_end");

        // A session with end_time inside window but no usage — dropped.
        let no_usage = session_with(Some("swe"), None, Some(dt("2026-05-11T00:00:00Z")), None);
        store
            .add_session(no_usage, dt("2026-05-10T00:00:00Z"), &actor)
            .await
            .expect("add no_usage");

        // A session with end_time outside window — dropped.
        let out_of_window = session_with(
            Some("swe"),
            None,
            Some(dt("2026-06-01T00:00:00Z")),
            Some(usage(100, 50, 0, 0)),
        );
        store
            .add_session(out_of_window, dt("2026-05-10T00:00:00Z"), &actor)
            .await
            .expect("add out");

        let got = fetch_sessions_with_usage(
            store.as_ref(),
            dt("2026-05-10T00:00:00Z"),
            dt("2026-05-13T00:00:00Z"),
            None,
            None,
        )
        .await
        .expect("fetch");
        let kept: Vec<_> = got.into_iter().map(|s| s.session_id).collect();
        assert_eq!(kept, vec![keep_id]);
    }
}
