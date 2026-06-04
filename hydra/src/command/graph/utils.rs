//! Shared helpers used by all `hydra graph` subcommands.
//!
//! - [`Selection`] captures the common selection-flag surface
//!   (`--source`/`--target`/`--object`/`--rel-type`/`--transitive`/`--scope`
//!   plus `--kind` post-filters, verbosity, and the node-budget cap), with
//!   [`Selection::validate`] for mutually-exclusive flag combinations.
//! - [`resolve_node_ids`] runs step 1 of the algorithm: resolve the set of
//!   node ids that the subcommand operates on.

use std::collections::HashSet;

use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use hydra_common::api::v1::relations::{ListRelationsRequest, ListRelationsResponse};
use hydra_common::graph::VerbosityLevel;
use hydra_common::HydraId;

use crate::client::HydraClientInterface;
use crate::command::graph::KindArg;

/// Shared node-selection inputs used by `search`, `diff`, and `log`.
///
/// Each subcommand layers its own additional flags on top of this surface
/// (`--since`/`--until` for `diff`/`log`, etc.) but the resolution of
/// `(source, target, object, rel_type, transitive, scope)` → node-id set
/// is identical.
#[derive(Debug, Clone)]
pub struct Selection {
    pub source: Option<HydraId>,
    pub target: Option<HydraId>,
    pub object: Option<HydraId>,
    pub rel_type: Option<String>,
    pub transitive: bool,
    pub scope: Option<HydraId>,
    pub kinds: Vec<KindArg>,
    pub verbosity: VerbosityLevel,
    pub max_nodes: usize,
}

impl Selection {
    /// Validate the CLI flag combinations. Returns an error message on misuse.
    pub fn validate(&self) -> Result<(), String> {
        if self.scope.is_some()
            && (self.source.is_some() || self.target.is_some() || self.object.is_some())
        {
            return Err(
                "--scope is mutually exclusive with --source/--target/--object".to_string(),
            );
        }
        if self.scope.is_none()
            && self.source.is_none()
            && self.target.is_none()
            && self.object.is_none()
        {
            return Err(
                "at least one of --source, --target, --object, or --scope is required".to_string(),
            );
        }
        Ok(())
    }
}

/// Step 1 of the algorithm: resolve the set of node ids to operate on.
pub async fn resolve_node_ids(
    client: &dyn HydraClientInterface,
    selection: &Selection,
) -> Result<Vec<HydraId>> {
    if let Some(scope) = &selection.scope {
        resolve_scope_node_ids(client, scope).await
    } else {
        let query = ListRelationsRequest {
            source_id: selection.source.clone(),
            source_ids: None,
            target_id: selection.target.clone(),
            target_ids: None,
            object_id: selection.object.clone(),
            object_ids: None,
            rel_type: selection.rel_type.clone(),
            transitive: if selection.transitive {
                Some(true)
            } else {
                None
            },
        };
        let response = client
            .list_relations(&query)
            .await
            .context("failed to list relations")?;
        Ok(node_ids_from_edges(&response))
    }
}

/// Union of `source_id` and `target_id` across each returned edge.
pub(crate) fn node_ids_from_edges(response: &ListRelationsResponse) -> Vec<HydraId> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<HydraId> = Vec::new();
    for edge in &response.relations {
        if seen.insert(edge.source_id.as_ref().to_string()) {
            out.push(edge.source_id.clone());
        }
        if seen.insert(edge.target_id.as_ref().to_string()) {
            out.push(edge.target_id.clone());
        }
    }
    out
}

/// Resolve `--scope <ID>` to its full node set:
/// `{scope} ∪ descendants(child-of, transitive) ∪ has-patch targets ∪
/// has-document targets`. `refers-to` is intentionally **not** fanned out
/// (it expresses a soft reference, not containment).
async fn resolve_scope_node_ids(
    client: &dyn HydraClientInterface,
    scope: &HydraId,
) -> Result<Vec<HydraId>> {
    // 1. Descendants via child-of (transitive).
    let descendants_query = ListRelationsRequest {
        target_id: Some(scope.clone()),
        rel_type: Some("child-of".to_string()),
        transitive: Some(true),
        ..Default::default()
    };
    let descendants_response = client
        .list_relations(&descendants_query)
        .await
        .context("failed to list child-of descendants for --scope")?;

    // For child-of edges with target=scope (transitive), each edge's source is
    // a descendant issue.
    let mut s_set: Vec<HydraId> = vec![scope.clone()];
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(scope.as_ref().to_string());
    for edge in &descendants_response.relations {
        if seen.insert(edge.source_id.as_ref().to_string()) {
            s_set.push(edge.source_id.clone());
        }
    }

    // 2. In parallel, for each of has-patch and has-document, fetch targets
    // whose source is in `s_set`.
    let source_ids_csv = s_set
        .iter()
        .map(|id| id.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(",");

    let mut futures = FuturesUnordered::new();
    for rel in ["has-patch", "has-document"] {
        let query = ListRelationsRequest {
            source_ids: Some(source_ids_csv.clone()),
            rel_type: Some(rel.to_string()),
            ..Default::default()
        };
        futures.push(async move { client.list_relations(&query).await });
    }

    let mut all_ids = s_set;
    while let Some(result) = futures.next().await {
        let response =
            result.context("failed to list has-patch/has-document targets for --scope")?;
        for edge in &response.relations {
            if seen.insert(edge.target_id.as_ref().to_string()) {
                all_ids.push(edge.target_id.clone());
            }
        }
    }
    Ok(all_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_selection() -> Selection {
        Selection {
            source: None,
            target: None,
            object: None,
            rel_type: None,
            transitive: false,
            scope: None,
            kinds: Vec::new(),
            verbosity: VerbosityLevel::L1,
            max_nodes: 10_000,
        }
    }

    #[test]
    fn validate_rejects_empty_selection() {
        let err = empty_selection().validate().unwrap_err();
        assert!(err.contains("at least one of"), "got: {err}");
    }

    #[test]
    fn validate_rejects_scope_with_source() {
        let selection = Selection {
            scope: Some("i-aaaaaa".parse().unwrap()),
            source: Some("i-bbbbbb".parse().unwrap()),
            ..empty_selection()
        };
        let err = selection.validate().unwrap_err();
        assert!(err.contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn validate_rejects_scope_with_target() {
        let selection = Selection {
            scope: Some("i-aaaaaa".parse().unwrap()),
            target: Some("i-bbbbbb".parse().unwrap()),
            ..empty_selection()
        };
        let err = selection.validate().unwrap_err();
        assert!(err.contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn validate_rejects_scope_with_object() {
        let selection = Selection {
            scope: Some("i-aaaaaa".parse().unwrap()),
            object: Some("i-bbbbbb".parse().unwrap()),
            ..empty_selection()
        };
        let err = selection.validate().unwrap_err();
        assert!(err.contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn validate_accepts_object_only() {
        let selection = Selection {
            object: Some("i-aaaaaa".parse().unwrap()),
            ..empty_selection()
        };
        assert!(selection.validate().is_ok());
    }

    #[test]
    fn validate_accepts_scope_alone() {
        let selection = Selection {
            scope: Some("i-aaaaaa".parse().unwrap()),
            ..empty_selection()
        };
        assert!(selection.validate().is_ok());
    }

    #[test]
    fn node_ids_from_edges_dedupes_and_preserves_order() {
        use chrono::Utc;
        use hydra_common::api::v1::relations::RelationResponse;
        let now = Utc::now();
        let response = ListRelationsResponse {
            relations: vec![
                RelationResponse {
                    source_id: "i-aaaaaa".parse().unwrap(),
                    target_id: "i-bbbbbb".parse().unwrap(),
                    rel_type: "child-of".to_string(),
                    created_at: now,
                },
                RelationResponse {
                    source_id: "i-aaaaaa".parse().unwrap(),
                    target_id: "p-cccccc".parse().unwrap(),
                    rel_type: "has-patch".to_string(),
                    created_at: now,
                },
            ],
        };
        let ids = node_ids_from_edges(&response);
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0].as_ref(), "i-aaaaaa");
        assert_eq!(ids[1].as_ref(), "i-bbbbbb");
        assert_eq!(ids[2].as_ref(), "p-cccccc");
    }
}
