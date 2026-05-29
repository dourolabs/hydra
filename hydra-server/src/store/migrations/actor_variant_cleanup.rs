//! Rewrite stored `ActorId` / `ActorRef` JSON blobs that reference the
//! deleted pre-cleanup variants (`Username`, `Session`, `Issue`,
//! `Service`, plus bare-string `Legacy` payloads) into one of the four
//! variants the post-cleanup `ActorId` accepts (`User`, `Agent`,
//! `Adhoc`, `External`).
//!
//! ## Mapping rules (§11 row 7 of `/designs/actor-system-overhaul.md`)
//!
//! | Pre-migration shape                              | Post-migration shape                                                                                |
//! |--------------------------------------------------|-----------------------------------------------------------------------------------------------------|
//! | `{"Username":"alice"}`                           | `{"User":{"name":"alice"}}`                                                                         |
//! | `{"Session":"s-..."}`                            | `{"Adhoc":{"session_id":"s-..."}}`                                                                  |
//! | `{"Issue":"i-..."}`                              | Actor of the latest non-deleted `tasks_v2` row where `spawned_from = "i-..."`. Otherwise External-legacy(`"i-..."`). |
//! | `{"Service":"<n>"}`                              | `{"Agent":{"name":"<n>"}}` if `<n>` validates as `AgentName`. Otherwise External-legacy(`"<n>"`).   |
//! | `"<bare-string>"` (a `Legacy` payload)           | Parsed via this module's self-contained parser; on parse failure External-legacy(`"<bare-string>"`).|
//! | Multi-key map (Legacy catch-all)                 | External-legacy with the JSON-stringified map as the username.                                      |
//! | Anything else (unknown tag, non-string/non-map)  | External-legacy with the JSON-stringified payload as the username.                                  |
//!
//! ## External-legacy fallback
//!
//! Any input the per-variant rules above can't rewrite into a typed
//! post-cleanup variant lands in `{"External":{"system":"legacy","username":<preserved>}}`
//! rather than NULL. The original identifier is preserved verbatim so
//! the row stays forensically recoverable. `system = "legacy"` is a
//! domain-meaningful marker (not a synonym for "unknown") — see
//! `i-rxlmigrv` for the user's rationale and the AGENTS.md
//! placeholder-values carve-out.
//!
//! Two `Option`-shaped slots intentionally null-collapse instead:
//! `System.on_behalf_of` (`Option<ActorId>`) and
//! `Automation.triggered_by` (`Option<Box<ActorRef>>`). The row's outer
//! `actor` column already stays non-null via the attribution captured
//! in `worker_name` / `automation_name`, so propagating External-legacy
//! into the inner slot would add noise without satisfying any new
//! invariant.
//!
//! ## Self-contained
//!
//! Per the design's call-out the migration MUST NOT depend on the
//! post-cleanup `hydra_common::ActorId` deserialization, otherwise a
//! future tweak to that type could silently invalidate the rewrite.
//! This module defines a local `NewActorId` enum and uses raw
//! `serde_json::Value` construction for the output JSON shape.
//!
//! ## Idempotent
//!
//! Per-row strategy: read JSON, if it parses as an OLD shape, rewrite
//! to the new shape; otherwise no-op. After a successful run every
//! row's `actor_id` JSON matches one of the four post-cleanup tags
//! (`User`/`Agent`/`Adhoc`/`External`), none of which match the
//! pre-cleanup-shape detector — so a second run is a no-op by
//! construction.

use super::{Backend, RustMigration};
use anyhow::{Context, Result};
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::users::Username;
use hydra_common::principal::ExternalSystem;
use serde_json::{Value, json};
use std::collections::HashMap;

/// The sqlx migration version this Rust step must run *after*. Pin to
/// the next clean date after the latest SQL migration
/// (`20260602000000_require_creator_not_null.sql`). A no-op SQL anchor
/// at the same version sits under `sqlite-migrations/` /
/// `migrations/` so the integration test's
/// `MIGRATOR.iter().any(|m| m.version as u64 == b.version)` check
/// keeps passing if a baseline anchors here.
pub const ACTOR_VARIANT_CLEANUP_VERSION: u64 = 20_260_603_000_000;

pub struct ActorVariantCleanupMigration;

#[async_trait::async_trait]
impl RustMigration for ActorVariantCleanupMigration {
    fn version(&self) -> u64 {
        ACTOR_VARIANT_CLEANUP_VERSION
    }

    fn name(&self) -> &'static str {
        "actor-variant-cleanup"
    }

    async fn run(&self, backend: &Backend) -> Result<()> {
        match backend {
            Backend::Sqlite(pool) => sqlite::run(pool).await,
            #[cfg(feature = "postgres")]
            Backend::Postgres(pool) => postgres::run(pool).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Self-contained parser + emitter
// ---------------------------------------------------------------------------

/// Local mirror of the post-cleanup `hydra_common::ActorId` shape.
/// Defined here so the migration's logic doesn't shift when the
/// upstream type tweaks its variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NewActorId {
    User { name: String },
    Agent { name: String },
    Adhoc { session_id: String },
    External { system: String, username: String },
}

impl NewActorId {
    /// Emit the canonical wire-form `serde_json::Value` for this
    /// variant — matches `hydra_common::ActorId::Serialize`.
    fn to_value(&self) -> Value {
        match self {
            NewActorId::User { name } => json!({"User": { "name": name }}),
            NewActorId::Agent { name } => json!({"Agent": { "name": name }}),
            NewActorId::Adhoc { session_id } => json!({"Adhoc": { "session_id": session_id }}),
            NewActorId::External { system, username } => {
                json!({"External": { "system": system, "username": username }})
            }
        }
    }
}

/// Outcome of attempting to rewrite a single stored `ActorId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rewrite {
    /// Already in a post-cleanup shape — leave alone.
    NoOp,
    /// Successfully decoded a pre-cleanup shape; write `value`.
    Replace(Value),
    /// Pre-cleanup `{"Issue":"<id>"}` shape — needs a `tasks_v2`
    /// lookup in the backend layer to resolve into a final shape.
    /// If the lookup misses, the caller selects a context-specific
    /// fallback (External-legacy for non-null columns, null for
    /// `Option`-shaped sub-actor slots).
    NeedsIssueLookup(String),
    /// No typed rewrite possible (unparseable bare-string Legacy,
    /// invalid Service name, multi-key map, unknown tag, …). `username`
    /// preserves the original identifier verbatim so the caller can
    /// wrap it into an External-legacy fallback for non-null columns,
    /// or null-collapse it for `Option`-shaped sub-actor slots.
    Fallback { username: String },
}

/// Build the canonical External-legacy fallback JSON. Used by callers
/// that own a non-null column (the outer `actor` / `actor_id`) and
/// need to preserve `username` verbatim instead of NULLing the row.
fn external_legacy_fallback(username: impl Into<String>) -> Value {
    NewActorId::External {
        system: "legacy".to_string(),
        username: username.into(),
    }
    .to_value()
}

/// Resolve a non-`NoOp` `Rewrite` outcome for a slot that owns a
/// **non-null** column (outer `actor` / `actor_id`). Issue-lookup
/// misses and `Fallback` outcomes both resolve to External-legacy with
/// the original identifier preserved as the username.
fn resolve_for_outer_slot(
    rewrite: Rewrite,
    issue_to_actor_id: &HashMap<String, Value>,
) -> OuterResolution {
    match rewrite {
        Rewrite::NoOp => OuterResolution::NoOp,
        Rewrite::Replace(v) => OuterResolution::Value(v),
        Rewrite::NeedsIssueLookup(iid) => OuterResolution::Value(
            issue_to_actor_id
                .get(&iid)
                .cloned()
                .unwrap_or_else(|| external_legacy_fallback(iid)),
        ),
        Rewrite::Fallback { username } => {
            OuterResolution::Value(external_legacy_fallback(username))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OuterResolution {
    NoOp,
    Value(Value),
}

/// Resolve a non-`NoOp` `Rewrite` outcome for an `Option`-shaped
/// sub-actor slot (`System.on_behalf_of`, the inner `actor_id` of
/// `Automation.triggered_by.Authenticated`). Issue-lookup misses and
/// `Fallback` outcomes both collapse to `null` rather than propagating
/// the External-legacy noise into a field that's already attribution-
/// covered by `worker_name` / `automation_name`.
fn resolve_for_inner_slot(
    rewrite: Rewrite,
    issue_to_actor_id: &HashMap<String, Value>,
) -> InnerResolution {
    match rewrite {
        Rewrite::NoOp => InnerResolution::NoOp,
        Rewrite::Replace(v) => InnerResolution::Value(v),
        Rewrite::NeedsIssueLookup(iid) => match issue_to_actor_id.get(&iid).cloned() {
            Some(v) => InnerResolution::Value(v),
            None => InnerResolution::Null,
        },
        Rewrite::Fallback { .. } => InnerResolution::Null,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InnerResolution {
    NoOp,
    Value(Value),
    Null,
}

/// Parse a possibly-legacy `ActorId` JSON value.
///
/// Returns `Rewrite::NoOp` if `value` is already in a post-cleanup
/// shape (so we don't rewrite already-migrated rows).
pub(crate) fn classify_actor_id(value: &Value) -> Rewrite {
    if let Some(s) = value.as_str() {
        // Bare-string Legacy payload.
        return match parse_legacy_string(s) {
            Some(new) => Rewrite::Replace(new.to_value()),
            None => Rewrite::Fallback {
                username: s.to_string(),
            },
        };
    }

    let Some(map) = value.as_object() else {
        // Numbers, booleans, arrays, null — preserve via JSON form.
        return Rewrite::Fallback {
            username: value.to_string(),
        };
    };

    // Externally-tagged form: exactly one key.
    if map.len() == 1 {
        let (tag, payload) = map.iter().next().expect("len==1");
        return classify_tagged(tag, payload);
    }

    // Multi-key map (legacy catch-all): preserve the JSON form so
    // operators can recover whatever the row used to be.
    Rewrite::Fallback {
        username: value.to_string(),
    }
}

fn classify_tagged(tag: &str, payload: &Value) -> Rewrite {
    match tag {
        // ---- Post-cleanup variants: already migrated, no-op ----
        "User" | "Agent" | "Adhoc" | "External" => Rewrite::NoOp,

        // ---- Pre-cleanup variants ----
        "Username" => match payload.as_str() {
            Some(name) => Rewrite::Replace(
                NewActorId::User {
                    name: name.to_string(),
                }
                .to_value(),
            ),
            None => Rewrite::Fallback {
                username: payload.to_string(),
            },
        },
        "Session" => match payload.as_str() {
            Some(sid) => Rewrite::Replace(
                NewActorId::Adhoc {
                    session_id: sid.to_string(),
                }
                .to_value(),
            ),
            None => Rewrite::Fallback {
                username: payload.to_string(),
            },
        },
        "Issue" => match payload.as_str() {
            Some(iid) => Rewrite::NeedsIssueLookup(iid.to_string()),
            None => Rewrite::Fallback {
                username: payload.to_string(),
            },
        },
        "Service" => match payload.as_str() {
            Some(name) => match AgentName::try_new(name) {
                Ok(_) => Rewrite::Replace(
                    NewActorId::Agent {
                        name: name.to_string(),
                    }
                    .to_value(),
                ),
                Err(_) => Rewrite::Fallback {
                    username: name.to_string(),
                },
            },
            None => Rewrite::Fallback {
                username: payload.to_string(),
            },
        },
        _ => {
            // Unknown variant tag: preserve the whole `{tag: payload}`
            // JSON form so operators can grep for and recover the
            // original shape.
            Rewrite::Fallback {
                username: json!({ tag: payload }).to_string(),
            }
        }
    }
}

/// Parse a bare-string Legacy payload into a post-cleanup `NewActorId`.
///
/// Recognises:
/// 1. Path forms: `users/<x>`, `agents/<x>`, `adhoc/<x>`,
///    `external/<sys>/<x>`.
/// 2. Pre-cleanup shorthand: `u-<x>` → User, `s-<x>` → Adhoc,
///    `svc-<x>` → Agent (when `<x>` validates as `AgentName`).
///
/// Pre-cleanup `a-<issue_id>` shorthand is intentionally NOT
/// recognised here — Issue rewrites require a `tasks_v2` lookup that
/// the caller drives, and Legacy strings carrying `a-i-...` are far
/// rarer than the corresponding `{"Issue":"i-..."}` tagged shape.
/// They fall back to External-legacy via `Rewrite::Fallback` with the
/// bare string preserved as the username.
fn parse_legacy_string(s: &str) -> Option<NewActorId> {
    if let Some(rest) = s.strip_prefix("users/") {
        Username::try_new(rest).ok()?;
        return Some(NewActorId::User {
            name: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("agents/") {
        AgentName::try_new(rest).ok()?;
        return Some(NewActorId::Agent {
            name: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("adhoc/") {
        if rest.is_empty() {
            return None;
        }
        return Some(NewActorId::Adhoc {
            session_id: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("external/") {
        let (system, username) = rest.split_once('/')?;
        if username.is_empty() {
            return None;
        }
        ExternalSystem::try_new(system).ok()?;
        return Some(NewActorId::External {
            system: system.to_string(),
            username: username.to_string(),
        });
    }

    if let Some(rest) = s.strip_prefix("u-") {
        Username::try_new(rest).ok()?;
        return Some(NewActorId::User {
            name: rest.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("s-") {
        // `s-<id>` was an `ActorId::Session` shorthand. Note the prefix
        // is included in the SessionId itself (e.g. `s-abcdef`), so
        // pass the whole string as the session_id.
        if rest.is_empty() {
            return None;
        }
        return Some(NewActorId::Adhoc {
            session_id: s.to_string(),
        });
    }
    if let Some(rest) = s.strip_prefix("svc-") {
        AgentName::try_new(rest).ok()?;
        return Some(NewActorId::Agent {
            name: rest.to_string(),
        });
    }

    None
}

/// Walk the inner `actor_id` inside an `ActorRef` blob (any of the
/// three `ActorRef` variants: `Authenticated`, `System`,
/// `Automation`).
///
/// Returns `(rewrite_outcome, rewritten_actor_ref)` if the blob's
/// payload was changed by the rewrite, or `Rewrite::NoOp` otherwise.
///
/// The `Automation` variant has a nested `triggered_by: Option<Box<ActorRef>>`,
/// so this is recursive.
pub(crate) fn classify_actor_ref(
    value: &Value,
    issue_to_actor_id: &HashMap<String, Value>,
) -> ActorRefRewrite {
    let Some(map) = value.as_object() else {
        return ActorRefRewrite::NoOp;
    };
    if map.len() != 1 {
        return ActorRefRewrite::NoOp;
    }
    let (tag, payload) = map.iter().next().expect("len==1");

    match tag.as_str() {
        "Authenticated" => {
            let Some(inner) = payload.as_object() else {
                return ActorRefRewrite::NoOp;
            };
            let Some(actor_id) = inner.get("actor_id") else {
                return ActorRefRewrite::NoOp;
            };
            // Outer `actor` column must stay non-null per `i-rxlmigrv`,
            // so unresolved Issue lookups / Fallback outcomes both
            // surface as External-legacy with the original identifier
            // preserved as the username.
            let final_actor_id =
                match resolve_for_outer_slot(classify_actor_id(actor_id), issue_to_actor_id) {
                    OuterResolution::NoOp => return ActorRefRewrite::NoOp,
                    OuterResolution::Value(v) => v,
                };
            let mut new_inner = inner.clone();
            new_inner.insert("actor_id".to_string(), final_actor_id);
            ActorRefRewrite::Replace(json!({ "Authenticated": new_inner }))
        }
        "System" => {
            // `on_behalf_of: Option<ActorId>`. An unresolvable
            // sub-actor null-collapses (rather than wrapping in
            // External-legacy) because the row's outer `actor`
            // already attribution-covers via `worker_name`. See module
            // docs / `i-rxlmigrv` for the rationale.
            let Some(inner) = payload.as_object() else {
                return ActorRefRewrite::NoOp;
            };
            let Some(on_behalf_of) = inner.get("on_behalf_of") else {
                return ActorRefRewrite::NoOp;
            };
            if on_behalf_of.is_null() {
                return ActorRefRewrite::NoOp;
            }
            let final_actor_id =
                match resolve_for_inner_slot(classify_actor_id(on_behalf_of), issue_to_actor_id) {
                    InnerResolution::NoOp => return ActorRefRewrite::NoOp,
                    InnerResolution::Value(v) => v,
                    InnerResolution::Null => Value::Null,
                };
            let mut new_inner = inner.clone();
            new_inner.insert("on_behalf_of".to_string(), final_actor_id);
            ActorRefRewrite::Replace(json!({ "System": new_inner }))
        }
        "Automation" => {
            // `triggered_by: Option<Box<ActorRef>>`. Inner unresolved
            // Authenticated sub-actors null-collapse for the same
            // reason as `System.on_behalf_of` above (the outer
            // `automation_name` carries attribution).
            let Some(inner) = payload.as_object() else {
                return ActorRefRewrite::NoOp;
            };
            let Some(triggered_by) = inner.get("triggered_by") else {
                return ActorRefRewrite::NoOp;
            };
            if triggered_by.is_null() {
                return ActorRefRewrite::NoOp;
            }
            match classify_actor_ref_inner(triggered_by, issue_to_actor_id) {
                InnerActorRefRewrite::NoOp => ActorRefRewrite::NoOp,
                InnerActorRefRewrite::Value(v) => {
                    let mut new_inner = inner.clone();
                    new_inner.insert("triggered_by".to_string(), v);
                    ActorRefRewrite::Replace(json!({ "Automation": new_inner }))
                }
                InnerActorRefRewrite::Null => {
                    let mut new_inner = inner.clone();
                    new_inner.insert("triggered_by".to_string(), Value::Null);
                    ActorRefRewrite::Replace(json!({ "Automation": new_inner }))
                }
            }
        }
        _ => ActorRefRewrite::NoOp,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InnerActorRefRewrite {
    NoOp,
    Value(Value),
    Null,
}

/// Like `classify_actor_ref`, but for an `ActorRef` that lives inside
/// an `Option`-shaped sub-actor slot (`Automation.triggered_by`).
/// Inner Authenticated arms null-collapse on unresolved Issue lookups
/// / `Fallback`, rather than wrapping into External-legacy.
fn classify_actor_ref_inner(
    value: &Value,
    issue_to_actor_id: &HashMap<String, Value>,
) -> InnerActorRefRewrite {
    let Some(map) = value.as_object() else {
        return InnerActorRefRewrite::NoOp;
    };
    if map.len() != 1 {
        return InnerActorRefRewrite::NoOp;
    }
    let (tag, payload) = map.iter().next().expect("len==1");
    match tag.as_str() {
        "Authenticated" => {
            let Some(inner) = payload.as_object() else {
                return InnerActorRefRewrite::NoOp;
            };
            let Some(actor_id) = inner.get("actor_id") else {
                return InnerActorRefRewrite::NoOp;
            };
            match resolve_for_inner_slot(classify_actor_id(actor_id), issue_to_actor_id) {
                InnerResolution::NoOp => InnerActorRefRewrite::NoOp,
                InnerResolution::Value(v) => {
                    let mut new_inner = inner.clone();
                    new_inner.insert("actor_id".to_string(), v);
                    InnerActorRefRewrite::Value(json!({ "Authenticated": new_inner }))
                }
                InnerResolution::Null => InnerActorRefRewrite::Null,
            }
        }
        // Nested System / Automation inside triggered_by reuse the
        // outer walker — the rule "null-collapse on unresolved" only
        // applies to the immediately-Authenticated sub-actor.
        "System" | "Automation" => match classify_actor_ref(value, issue_to_actor_id) {
            ActorRefRewrite::NoOp => InnerActorRefRewrite::NoOp,
            ActorRefRewrite::Replace(v) => InnerActorRefRewrite::Value(v),
        },
        _ => InnerActorRefRewrite::NoOp,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActorRefRewrite {
    NoOp,
    Replace(Value),
}

// ---------------------------------------------------------------------------
// Per-rewrite counts (logged at end-of-run for operator spot-checks)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Counts {
    /// (table, kind) -> count. `kind` is `"rewritten"` — no driver
    /// path NULLs an actor / actor_id column anymore (the
    /// External-legacy fallback covers every previously-NULLable
    /// case).
    by_table: HashMap<(&'static str, &'static str), u64>,
}

impl Counts {
    fn rewrote(&mut self, table: &'static str) {
        *self.by_table.entry((table, "rewritten")).or_insert(0) += 1;
    }
}

fn log_counts(counts: &Counts) {
    let mut entries: Vec<_> = counts.by_table.iter().collect();
    entries.sort_by_key(|((t, k), _)| (*t, *k));
    for ((table, kind), count) in entries {
        tracing::info!(
            target: "actor_variant_cleanup",
            table = %table,
            kind = %kind,
            count = *count,
            "actor_variant_cleanup row count"
        );
    }
}

// ---------------------------------------------------------------------------
// Tables walked by both backends. Centralised so a new mutation-history
// table doesn't risk drifting between the two backend impls.
// ---------------------------------------------------------------------------

/// Tables with an `ActorRef` JSON column (the column carries a nested
/// `ActorId` inside an `Authenticated`/`System`/`Automation` envelope).
///
/// Both backends share these names except for `session_events` /
/// `session_events_v2` and `conversation_events` /
/// `conversation_events_v2`; the per-backend driver expands the schema
/// prefix as needed.
const ACTOR_REF_TABLES_COMMON: &[&str] = &[
    "repositories_v2",
    "actors_v2",
    "users_v2",
    "issues_v2",
    "patches_v2",
    "tasks_v2",
    "documents_v2",
];

// ---------------------------------------------------------------------------
// SQLite driver
// ---------------------------------------------------------------------------

mod sqlite {
    use super::*;
    use sqlx::{Row, SqlitePool};

    pub async fn run(pool: &SqlitePool) -> Result<()> {
        let mut counts = Counts::default();

        // Build issue_id -> actor JSON map up-front for the `Issue` arm.
        let issue_to_actor = load_issue_to_actor_id(pool).await?;

        for table in ACTOR_REF_TABLES_COMMON {
            rewrite_actor_ref_column(pool, table, "id", &issue_to_actor, &mut counts).await?;
        }
        // `session_events` / `conversation_events` have non-`id`
        // primary keys; expand explicitly.
        rewrite_actor_ref_column(
            pool,
            "session_events",
            "(session_id, version_number)",
            &issue_to_actor,
            &mut counts,
        )
        .await?;
        rewrite_actor_ref_column(
            pool,
            "conversation_events",
            "(id, version_number)",
            &issue_to_actor,
            &mut counts,
        )
        .await?;

        // Bare `ActorId` column: `actors_v2.actor_id`.
        rewrite_actor_id_column(
            pool,
            "actors_v2",
            "actor_id",
            "id",
            &issue_to_actor,
            &mut counts,
        )
        .await?;

        // `patches_v2.reviews[*].author` — already typed `Principal`
        // post p-ajkfmhax (only User/Agent/External), so we don't walk
        // it here; the legacy `Username` shape is unreachable for
        // Reviews.

        log_counts(&counts);
        Ok(())
    }

    async fn load_issue_to_actor_id(pool: &SqlitePool) -> Result<HashMap<String, Value>> {
        let rows = sqlx::query(
            "SELECT spawned_from, actor FROM tasks_v2 \
             WHERE spawned_from IS NOT NULL AND is_latest = 1 AND deleted = 0",
        )
        .fetch_all(pool)
        .await
        .context("load tasks_v2 for issue-spawned-from lookup")?;

        let mut out: HashMap<String, Vec<Value>> = HashMap::new();
        for row in rows {
            let issue_id: String = row.try_get("spawned_from")?;
            let actor: Option<String> = row.try_get("actor")?;
            let Some(actor_json) = actor.as_deref() else {
                continue;
            };
            let parsed: Value = match serde_json::from_str(actor_json) {
                Ok(v) => v,
                Err(_) => continue,
            };
            // Extract the inner actor_id when wrapped in
            // Authenticated/System; for Automation we can't pick a
            // single actor_id so we skip.
            if let Some(inner_actor_id) = extract_actor_id_from_actor_ref(&parsed) {
                out.entry(issue_id).or_default().push(inner_actor_id);
            }
        }

        let mut single: HashMap<String, Value> = HashMap::new();
        for (iid, mut actors) in out {
            if actors.len() == 1 {
                single.insert(iid, actors.pop().expect("len==1"));
            } else {
                tracing::warn!(
                    target: "actor_variant_cleanup",
                    issue_id = %iid,
                    matches = actors.len(),
                    "Issue actor lookup: 0 or >1 matching tasks_v2 rows; will fall back to External-legacy for the original Issue id"
                );
            }
        }
        Ok(single)
    }

    async fn rewrite_actor_ref_column(
        pool: &SqlitePool,
        table: &'static str,
        pk_sql: &str,
        issue_to_actor_id: &HashMap<String, Value>,
        counts: &mut Counts,
    ) -> Result<()> {
        let select_sql = format!(
            "SELECT {}, actor FROM {table} WHERE actor IS NOT NULL",
            pk_cols_for(pk_sql).join(", ")
        );
        let rows = sqlx::query(&select_sql)
            .fetch_all(pool)
            .await
            .with_context(|| format!("scan {table}.actor"))?;
        for row in rows {
            let actor: String = row.try_get("actor")?;
            let parsed: Value = match serde_json::from_str(&actor) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let rewrite = classify_actor_ref(&parsed, issue_to_actor_id);
            apply_actor_ref_rewrite(pool, table, pk_sql, &row, rewrite, counts).await?;
        }
        Ok(())
    }

    async fn apply_actor_ref_rewrite(
        pool: &SqlitePool,
        table: &'static str,
        pk_sql: &str,
        row: &sqlx::sqlite::SqliteRow,
        rewrite: ActorRefRewrite,
        counts: &mut Counts,
    ) -> Result<()> {
        match rewrite {
            ActorRefRewrite::NoOp => Ok(()),
            ActorRefRewrite::Replace(new_value) => {
                let new_json = new_value.to_string();
                exec_pk_update(pool, table, "actor", pk_sql, row, &new_json).await?;
                counts.rewrote(table);
                Ok(())
            }
        }
    }

    async fn rewrite_actor_id_column(
        pool: &SqlitePool,
        table: &'static str,
        column: &'static str,
        pk_sql: &str,
        issue_to_actor_id: &HashMap<String, Value>,
        counts: &mut Counts,
    ) -> Result<()> {
        let select_sql = format!(
            "SELECT {}, {column} AS payload FROM {table} \
             WHERE {column} IS NOT NULL",
            pk_cols_for(pk_sql).join(", ")
        );
        let rows = sqlx::query(&select_sql)
            .fetch_all(pool)
            .await
            .with_context(|| format!("scan {table}.{column}"))?;
        for row in rows {
            let actor_id: String = row.try_get("payload")?;
            let parsed: Value = match serde_json::from_str(&actor_id) {
                Ok(v) => v,
                Err(_) => Value::String(actor_id.clone()),
            };
            let final_value =
                match resolve_for_outer_slot(classify_actor_id(&parsed), issue_to_actor_id) {
                    OuterResolution::NoOp => continue,
                    OuterResolution::Value(v) => v,
                };
            let serialized = final_value.to_string();
            exec_pk_update(pool, table, column, pk_sql, &row, &serialized).await?;
            counts.rewrote(table);
        }
        Ok(())
    }

    fn pk_cols_for(pk_sql: &str) -> Vec<&'static str> {
        match pk_sql {
            "id" => vec!["id"],
            "(session_id, version_number)" => vec!["session_id", "version_number"],
            "(id, version_number)" => vec!["id", "version_number"],
            other => panic!("unsupported pk_sql expression for sqlite: {other}"),
        }
    }

    async fn exec_pk_update(
        pool: &SqlitePool,
        table: &'static str,
        column: &'static str,
        pk_sql: &str,
        row: &sqlx::sqlite::SqliteRow,
        new_value: &str,
    ) -> Result<()> {
        // sqlx::sqlite can't express tuple equality directly, so we
        // dispatch on the shape of pk_sql.
        if pk_sql == "id" {
            let id: String = row.try_get("id")?;
            let sql = format!("UPDATE {table} SET {column} = ?1 WHERE id = ?2");
            sqlx::query(&sql)
                .bind(new_value)
                .bind(&id)
                .execute(pool)
                .await
                .with_context(|| format!("update {table}.{column} for id={id}"))?;
        } else if pk_sql == "(session_id, version_number)" {
            let session_id: String = row.try_get("session_id")?;
            let version: i64 = row.try_get("version_number")?;
            let sql = format!(
                "UPDATE {table} SET {column} = ?1 WHERE session_id = ?2 AND version_number = ?3"
            );
            sqlx::query(&sql)
                .bind(new_value)
                .bind(&session_id)
                .bind(version)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("update {table}.{column} for ({session_id}, {version})")
                })?;
        } else if pk_sql == "(id, version_number)" {
            let id: String = row.try_get("id")?;
            let version: i64 = row.try_get("version_number")?;
            let sql =
                format!("UPDATE {table} SET {column} = ?1 WHERE id = ?2 AND version_number = ?3");
            sqlx::query(&sql)
                .bind(new_value)
                .bind(&id)
                .bind(version)
                .execute(pool)
                .await
                .with_context(|| format!("update {table}.{column} for ({id}, {version})"))?;
        } else {
            anyhow::bail!("unsupported pk_sql expression for sqlite: {pk_sql}");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Postgres driver
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use sqlx::{PgPool, Row};

    pub async fn run(pool: &PgPool) -> Result<()> {
        let mut counts = Counts::default();
        let issue_to_actor = load_issue_to_actor_id(pool).await?;

        for table in ACTOR_REF_TABLES_COMMON {
            rewrite_actor_ref_column(pool, table, "id", &issue_to_actor, &mut counts).await?;
        }
        rewrite_actor_ref_column(
            pool,
            "session_events_v2",
            "(session_id, version_number)",
            &issue_to_actor,
            &mut counts,
        )
        .await?;
        rewrite_actor_ref_column(
            pool,
            "conversation_events_v2",
            "(conversation_id, version_number)",
            &issue_to_actor,
            &mut counts,
        )
        .await?;

        rewrite_actor_id_column(
            pool,
            "actors_v2",
            "actor_id",
            "id",
            &issue_to_actor,
            &mut counts,
        )
        .await?;

        log_counts(&counts);
        Ok(())
    }

    async fn load_issue_to_actor_id(pool: &PgPool) -> Result<HashMap<String, Value>> {
        let rows = sqlx::query(
            "SELECT spawned_from, actor FROM metis.tasks_v2 \
             WHERE spawned_from IS NOT NULL AND is_latest = TRUE AND deleted = FALSE",
        )
        .fetch_all(pool)
        .await
        .context("load tasks_v2 for issue-spawned-from lookup")?;

        let mut grouped: HashMap<String, Vec<Value>> = HashMap::new();
        for row in rows {
            let issue_id: String = row.try_get("spawned_from")?;
            let actor: Option<Value> = row.try_get("actor")?;
            let Some(actor_value) = actor else { continue };
            if let Some(inner_actor_id) = extract_actor_id_from_actor_ref(&actor_value) {
                grouped.entry(issue_id).or_default().push(inner_actor_id);
            }
        }

        let mut single = HashMap::new();
        for (iid, mut actors) in grouped {
            if actors.len() == 1 {
                single.insert(iid, actors.pop().expect("len==1"));
            } else {
                tracing::warn!(
                    target: "actor_variant_cleanup",
                    issue_id = %iid,
                    matches = actors.len(),
                    "Issue actor lookup: 0 or >1 matching tasks_v2 rows; will fall back to External-legacy for the original Issue id"
                );
            }
        }
        Ok(single)
    }

    async fn rewrite_actor_ref_column(
        pool: &PgPool,
        table: &'static str,
        pk_sql: &str,
        issue_to_actor_id: &HashMap<String, Value>,
        counts: &mut Counts,
    ) -> Result<()> {
        let pk_cols = pk_cols_for(pk_sql);
        let select_sql = format!(
            "SELECT {} , actor FROM metis.{table} WHERE actor IS NOT NULL",
            pk_cols.join(", ")
        );
        let rows = sqlx::query(&select_sql)
            .fetch_all(pool)
            .await
            .with_context(|| format!("scan metis.{table}.actor"))?;
        for row in rows {
            let actor: Value = row.try_get("actor")?;
            let rewrite = classify_actor_ref(&actor, issue_to_actor_id);
            apply_actor_ref_rewrite(pool, table, &pk_cols, &row, rewrite, counts).await?;
        }
        Ok(())
    }

    async fn apply_actor_ref_rewrite(
        pool: &PgPool,
        table: &'static str,
        pk_cols: &[&str],
        row: &sqlx::postgres::PgRow,
        rewrite: ActorRefRewrite,
        counts: &mut Counts,
    ) -> Result<()> {
        match rewrite {
            ActorRefRewrite::NoOp => Ok(()),
            ActorRefRewrite::Replace(new_value) => {
                exec_pk_update(pool, table, "actor", pk_cols, row, new_value).await?;
                counts.rewrote(table);
                Ok(())
            }
        }
    }

    async fn rewrite_actor_id_column(
        pool: &PgPool,
        table: &'static str,
        column: &'static str,
        pk_sql: &str,
        issue_to_actor_id: &HashMap<String, Value>,
        counts: &mut Counts,
    ) -> Result<()> {
        let pk_cols = pk_cols_for(pk_sql);
        let select_sql = format!(
            "SELECT {} , {column} AS payload FROM metis.{table} WHERE {column} IS NOT NULL",
            pk_cols.join(", ")
        );
        let rows = sqlx::query(&select_sql)
            .fetch_all(pool)
            .await
            .with_context(|| format!("scan metis.{table}.{column}"))?;
        for row in rows {
            let actor_id: Value = row.try_get("payload")?;
            let final_value =
                match resolve_for_outer_slot(classify_actor_id(&actor_id), issue_to_actor_id) {
                    OuterResolution::NoOp => continue,
                    OuterResolution::Value(v) => v,
                };
            exec_pk_update(pool, table, column, &pk_cols, &row, final_value).await?;
            counts.rewrote(table);
        }
        Ok(())
    }

    fn pk_cols_for(pk_sql: &str) -> Vec<&'static str> {
        match pk_sql {
            "id" => vec!["id"],
            "(session_id, version_number)" => vec!["session_id", "version_number"],
            "(conversation_id, version_number)" => vec!["conversation_id", "version_number"],
            other => panic!("unsupported pk_sql expression for postgres: {other}"),
        }
    }

    async fn exec_pk_update(
        pool: &PgPool,
        table: &'static str,
        column: &'static str,
        pk_cols: &[&str],
        row: &sqlx::postgres::PgRow,
        new_value: Value,
    ) -> Result<()> {
        let where_clause = pk_cols
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c} = ${}", i + 2))
            .collect::<Vec<_>>()
            .join(" AND ");
        let sql = format!("UPDATE metis.{table} SET {column} = $1 WHERE {where_clause}");
        let mut query = sqlx::query(&sql).bind(new_value);
        for col in pk_cols {
            match *col {
                "id" | "session_id" | "conversation_id" => {
                    let v: String = row.try_get(*col)?;
                    query = query.bind(v);
                }
                "version_number" => {
                    let v: i64 = row.try_get(*col)?;
                    query = query.bind(v);
                }
                other => anyhow::bail!("unsupported pk column: {other}"),
            }
        }
        query
            .execute(pool)
            .await
            .with_context(|| format!("update metis.{table}.{column}"))?;
        Ok(())
    }
}

/// Extract the inner `actor_id` from an `ActorRef::Authenticated` /
/// `ActorRef::System` blob and resolve it to a post-cleanup wire
/// shape. Returns `None` for `Automation` (which doesn't carry a flat
/// actor_id), unrecognised shapes, and any pre-cleanup actor that
/// itself fails to resolve (chained `Issue` references — we don't
/// chase chains of issue lookups, only the first hop — or `Fallback`
/// outcomes whose External-legacy form would be a spurious "real"
/// actor to substitute into a different row).
fn extract_actor_id_from_actor_ref(value: &Value) -> Option<Value> {
    let map = value.as_object()?;
    if map.len() != 1 {
        return None;
    }
    let (tag, payload) = map.iter().next()?;
    let raw = match tag.as_str() {
        "Authenticated" => payload.get("actor_id").cloned()?,
        "System" => {
            let v = payload.get("on_behalf_of").cloned()?;
            if v.is_null() {
                return None;
            }
            v
        }
        _ => return None,
    };
    // Classify so the substituted actor_id is always a post-cleanup
    // shape, never an Issue/Username/etc. pre-cleanup carrier.
    match classify_actor_id(&raw) {
        Rewrite::NoOp => Some(raw),
        Rewrite::Replace(v) => Some(v),
        Rewrite::NeedsIssueLookup(_) | Rewrite::Fallback { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_username_rewrites_to_user() {
        let v = json!({"Username": "alice"});
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Replace(json!({"User": {"name": "alice"}}))
        );
    }

    #[test]
    fn classify_session_rewrites_to_adhoc() {
        let v = json!({"Session": "s-abcdef"});
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Replace(json!({"Adhoc": {"session_id": "s-abcdef"}}))
        );
    }

    #[test]
    fn classify_issue_returns_needs_lookup() {
        let v = json!({"Issue": "i-abcdef"});
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::NeedsIssueLookup("i-abcdef".to_string())
        );
    }

    #[test]
    fn classify_service_valid_name_rewrites_to_agent() {
        let v = json!({"Service": "swe"});
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Replace(json!({"Agent": {"name": "swe"}}))
        );
    }

    #[test]
    fn classify_service_invalid_name_returns_external_legacy_fallback() {
        // Invalid AgentName payload preserves the original `<name>` as
        // the External-legacy username so the row stays forensically
        // recoverable instead of NULLing.
        let v = json!({"Service": "has space"});
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Fallback {
                username: "has space".to_string(),
            }
        );
    }

    #[test]
    fn classify_post_cleanup_user_is_noop() {
        let v = json!({"User": {"name": "alice"}});
        assert_eq!(classify_actor_id(&v), Rewrite::NoOp);
    }

    #[test]
    fn classify_post_cleanup_agent_is_noop() {
        let v = json!({"Agent": {"name": "swe"}});
        assert_eq!(classify_actor_id(&v), Rewrite::NoOp);
    }

    #[test]
    fn classify_post_cleanup_adhoc_is_noop() {
        let v = json!({"Adhoc": {"session_id": "s-abcdef"}});
        assert_eq!(classify_actor_id(&v), Rewrite::NoOp);
    }

    #[test]
    fn classify_post_cleanup_external_is_noop() {
        let v = json!({"External": {"system": "github", "username": "jayantk"}});
        assert_eq!(classify_actor_id(&v), Rewrite::NoOp);
    }

    #[test]
    fn classify_bare_string_user_path() {
        let v = json!("users/alice");
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Replace(json!({"User": {"name": "alice"}}))
        );
    }

    #[test]
    fn classify_bare_string_agent_path() {
        let v = json!("agents/swe");
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Replace(json!({"Agent": {"name": "swe"}}))
        );
    }

    #[test]
    fn classify_bare_string_u_shorthand() {
        let v = json!("u-alice");
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Replace(json!({"User": {"name": "alice"}}))
        );
    }

    #[test]
    fn classify_bare_string_s_shorthand_session_to_adhoc() {
        let v = json!("s-abcdef");
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Replace(json!({"Adhoc": {"session_id": "s-abcdef"}}))
        );
    }

    #[test]
    fn classify_bare_string_unparseable_returns_external_legacy_fallback() {
        // Unrecognised bare-string Legacy payload preserves the
        // original string verbatim as the External-legacy username.
        let v = json!("¯\\_(ツ)_/¯");
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Fallback {
                username: "¯\\_(ツ)_/¯".to_string(),
            }
        );
    }

    #[test]
    fn classify_multi_key_map_returns_external_legacy_fallback() {
        // Multi-key legacy catch-all preserves the JSON form of the
        // whole map so operators can recover whatever the row used to
        // be.
        let v = json!({"kind": "user", "name": "alice"});
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Fallback {
                username: v.to_string(),
            }
        );
    }

    #[test]
    fn classify_unknown_tag_returns_external_legacy_fallback() {
        let v = json!({"Robot": {"name": "r2"}});
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Fallback {
                username: v.to_string(),
            }
        );
    }

    #[test]
    fn classify_external_bare_string_round_trips() {
        let v = json!("external/github/jayantk");
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Replace(json!({"External": {"system": "github", "username": "jayantk"}}))
        );
    }

    #[test]
    fn output_round_trips_through_hydra_common_deserialize() {
        // Belt-and-braces: every concrete shape the migration emits
        // must deserialize via the upstream
        // `hydra_common::ActorId::deserialize`.
        for raw in [
            json!({"Username": "alice"}),
            json!({"Session": "s-abcdef"}),
            json!({"Service": "swe"}),
            json!("u-alice"),
            json!("agents/swe"),
        ] {
            let rewrite = classify_actor_id(&raw);
            let Rewrite::Replace(v) = rewrite else {
                panic!("expected Replace for {raw}");
            };
            let _: hydra_common::ActorId = serde_json::from_value(v.clone())
                .unwrap_or_else(|e| panic!("upstream deserialize failed for {v}: {e}"));
        }

        // External-legacy fallback shapes also must round-trip — the
        // §3.3 store-level smoke depends on it.
        for raw in [
            json!({"Service": "has space"}),
            json!("a-i-abc"),
            json!({"kind": "user", "name": "alice"}),
        ] {
            let rewrite = classify_actor_id(&raw);
            let Rewrite::Fallback { username } = rewrite else {
                panic!("expected Fallback for {raw}");
            };
            let v = external_legacy_fallback(username);
            let _: hydra_common::ActorId = serde_json::from_value(v.clone())
                .unwrap_or_else(|e| panic!("upstream deserialize failed for {v}: {e}"));
        }
    }

    #[test]
    fn actor_ref_authenticated_username_rewrites_inner_actor_id() {
        let raw = json!({"Authenticated": {"actor_id": {"Username": "alice"}}});
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(
                json!({"Authenticated": {"actor_id": {"User": {"name": "alice"}}}})
            )
        );
    }

    #[test]
    fn actor_ref_authenticated_with_session_id_preserves_field() {
        let raw = json!({
            "Authenticated": {
                "actor_id": {"Session": "s-abcdef"},
                "session_id": "s-abcdef"
            }
        });
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        let ActorRefRewrite::Replace(v) = rewrite else {
            panic!("expected Replace")
        };
        assert_eq!(
            v,
            json!({
                "Authenticated": {
                    "actor_id": {"Adhoc": {"session_id": "s-abcdef"}},
                    "session_id": "s-abcdef"
                }
            })
        );
    }

    #[test]
    fn actor_ref_authenticated_already_migrated_is_noop() {
        let raw = json!({
            "Authenticated": {
                "actor_id": {"User": {"name": "alice"}},
                "session_id": null
            }
        });
        let lookup = HashMap::new();
        assert_eq!(classify_actor_ref(&raw, &lookup), ActorRefRewrite::NoOp);
    }

    #[test]
    fn actor_ref_system_on_behalf_of_rewritten() {
        let raw = json!({
            "System": {
                "worker_name": "task-spawner",
                "on_behalf_of": {"Username": "alice"}
            }
        });
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(json!({
                "System": {
                    "worker_name": "task-spawner",
                    "on_behalf_of": {"User": {"name": "alice"}}
                }
            }))
        );
    }

    #[test]
    fn actor_ref_authenticated_issue_resolves_via_lookup() {
        let raw = json!({"Authenticated": {"actor_id": {"Issue": "i-abc"}}});
        let mut lookup = HashMap::new();
        lookup.insert("i-abc".to_string(), json!({"User": {"name": "alice"}}));
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(
                json!({"Authenticated": {"actor_id": {"User": {"name": "alice"}}}})
            )
        );
    }

    #[test]
    fn actor_ref_authenticated_issue_no_match_falls_back_to_external_legacy() {
        // Outer `actor` column must stay non-null per `i-rxlmigrv`, so
        // an unresolved Issue lookup wraps into External-legacy with
        // the issue id preserved as the username.
        let raw = json!({"Authenticated": {"actor_id": {"Issue": "i-no-match"}}});
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(json!({
                "Authenticated": {
                    "actor_id": {"External": {"system": "legacy", "username": "i-no-match"}}
                }
            }))
        );
    }

    #[test]
    fn actor_ref_authenticated_service_invalid_falls_back_to_external_legacy() {
        let raw = json!({"Authenticated": {"actor_id": {"Service": "has space"}}});
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(json!({
                "Authenticated": {
                    "actor_id": {"External": {"system": "legacy", "username": "has space"}}
                }
            }))
        );
    }

    #[test]
    fn actor_ref_authenticated_unparseable_bare_string_falls_back_to_external_legacy() {
        let raw = json!({"Authenticated": {"actor_id": "definitely not an actor"}});
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(json!({
                "Authenticated": {
                    "actor_id": {
                        "External": {"system": "legacy", "username": "definitely not an actor"}
                    }
                }
            }))
        );
    }

    #[test]
    fn actor_ref_automation_recurses_into_triggered_by() {
        let raw = json!({
            "Automation": {
                "automation_name": "github_pr_sync",
                "triggered_by": {"Authenticated": {"actor_id": {"Username": "alice"}}}
            }
        });
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(json!({
                "Automation": {
                    "automation_name": "github_pr_sync",
                    "triggered_by": {"Authenticated": {"actor_id": {"User": {"name": "alice"}}}}
                }
            }))
        );
    }

    #[test]
    fn classify_bare_string_adhoc_path() {
        let v = json!("adhoc/s-xxx");
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Replace(json!({"Adhoc": {"session_id": "s-xxx"}}))
        );
    }

    #[test]
    fn classify_bare_string_svc_shorthand() {
        let valid = json!("svc-swe");
        assert_eq!(
            classify_actor_id(&valid),
            Rewrite::Replace(json!({"Agent": {"name": "swe"}}))
        );
        let invalid = json!("svc-has space");
        assert_eq!(
            classify_actor_id(&invalid),
            Rewrite::Fallback {
                username: "svc-has space".to_string(),
            }
        );
    }

    #[test]
    fn classify_bare_string_a_issue_shorthand_falls_back() {
        // `a-<issue_id>` is intentionally unrecognised by the bare-string
        // parser: the corresponding tagged shape `{"Issue":"i-..."}`
        // covers the same case and routes through the lookup, while a
        // bare `a-` string would short-circuit that path with no Issue
        // lookup at all. The unrecognised string lands in the
        // External-legacy fallback so the row stays non-null.
        let v = json!("a-i-abc");
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Fallback {
                username: "a-i-abc".to_string(),
            }
        );
    }

    #[test]
    fn classify_bare_string_external_invalid_system_falls_back() {
        // `ExternalSystem::try_new` rejects whitespace, so the legacy
        // path `external/<has space>/foo` falls through to Fallback —
        // the whole original bare string is preserved verbatim.
        let v = json!("external/has space/foo");
        assert_eq!(
            classify_actor_id(&v),
            Rewrite::Fallback {
                username: "external/has space/foo".to_string(),
            }
        );
    }

    #[test]
    fn classify_non_string_non_map_falls_back() {
        // Numbers, booleans, nulls, arrays land in External-legacy
        // with their JSON form preserved as the username.
        for raw in [json!(42), json!(true), json!(null), json!(["a"])] {
            let expected = raw.to_string();
            match classify_actor_id(&raw) {
                Rewrite::Fallback { username } if username == expected => {}
                other => panic!("unexpected classification for {raw}: {other:?}"),
            }
        }
    }

    #[test]
    fn classify_tagged_payload_not_string_falls_back() {
        // `{"Username": 42}` and similar: payload isn't a string. The
        // Fallback carries the payload's JSON form so operators can
        // see what came in.
        for raw in [
            json!({"Username": 42}),
            json!({"Session": null}),
            json!({"Issue": ["i-abc"]}),
            json!({"Service": {"nested": "thing"}}),
        ] {
            let Rewrite::Fallback { .. } = classify_actor_id(&raw) else {
                panic!("expected Fallback for {raw}");
            };
        }
    }

    #[test]
    fn classify_actor_ref_system_issue_no_match_collapses_to_null_on_behalf_of() {
        // `actor_ref_authenticated_issue_no_match_drops_to_null` (above)
        // exercises the Authenticated arm. The System arm differs: an
        // unresolvable `on_behalf_of` collapses the inner field to
        // `null` rather than NULLing the whole row, because System
        // attribution lives in `worker_name`.
        let raw = json!({
            "System": {
                "worker_name": "task-spawner",
                "on_behalf_of": {"Issue": "i-no-match"}
            }
        });
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(json!({
                "System": {
                    "worker_name": "task-spawner",
                    "on_behalf_of": null
                }
            }))
        );
    }

    #[test]
    fn classify_actor_ref_system_fallback_collapses_to_null_on_behalf_of() {
        // Inner `Option<ActorId>` slots null-collapse on any
        // unresolvable sub-actor — including Fallback outcomes —
        // because the row's outer attribution already lives in
        // `worker_name`. See module docs / `i-rxlmigrv`.
        let raw = json!({
            "System": {
                "worker_name": "task-spawner",
                "on_behalf_of": {"Service": "has space"}
            }
        });
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(json!({
                "System": {
                    "worker_name": "task-spawner",
                    "on_behalf_of": null
                }
            }))
        );
    }

    #[test]
    fn classify_actor_ref_automation_fallback_collapses_to_null_triggered_by() {
        // Same rationale as the System fallback test above —
        // Automation.triggered_by's inner Authenticated.actor_id is an
        // `Option`-shaped sub-actor slot; a Fallback outcome
        // null-collapses rather than wrapping in External-legacy.
        let raw = json!({
            "Automation": {
                "automation_name": "github_pr_sync",
                "triggered_by": {
                    "Authenticated": {"actor_id": {"Service": "has space"}}
                }
            }
        });
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(json!({
                "Automation": {
                    "automation_name": "github_pr_sync",
                    "triggered_by": null
                }
            }))
        );
    }

    #[test]
    fn classify_actor_ref_automation_issue_no_match_collapses_to_null_triggered_by() {
        // Automation.triggered_by carrying an unresolvable Authenticated/
        // Issue ref collapses to `triggered_by: null` rather than NULLing
        // the whole row, mirroring the System.on_behalf_of behaviour.
        let raw = json!({
            "Automation": {
                "automation_name": "github_pr_sync",
                "triggered_by": {
                    "Authenticated": {"actor_id": {"Issue": "i-no-match"}}
                }
            }
        });
        let lookup = HashMap::new();
        let rewrite = classify_actor_ref(&raw, &lookup);
        assert_eq!(
            rewrite,
            ActorRefRewrite::Replace(json!({
                "Automation": {
                    "automation_name": "github_pr_sync",
                    "triggered_by": null
                }
            }))
        );
    }
}
