//! Shared node-set selection logic for `hydra graph` subcommands.
//!
//! The flag surface — `--source` / `--target` / `--object` / `--rel-type` /
//! `--transitive` / `--scope` — is identical across `search`, `diff`, and
//! `log`. The validation and the corresponding relation-query fan-out live
//! here so each subcommand only worries about what to do *with* the resolved
//! id set.

use std::collections::HashSet;

use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use hydra_common::api::v1::relations::{ListRelationsRequest, ListRelationsResponse};
use hydra_common::HydraId;

use crate::client::HydraClientInterface;

/// Inputs to the shared node-id resolution step.
///
/// Mirrors the relation-query flags that `search` / `diff` / `log` all expose.
#[derive(Debug, Clone)]
pub struct SelectionFlags {
    pub source: Option<HydraId>,
    pub target: Option<HydraId>,
    pub object: Option<HydraId>,
    pub rel_type: Option<String>,
    pub transitive: bool,
    pub scope: Option<HydraId>,
}

/// Validate the CLI flag combinations. Returns an error message on misuse
/// (intended for the caller to print and exit with code 2).
pub fn validate(flags: &SelectionFlags) -> Result<(), String> {
    if flags.scope.is_some()
        && (flags.source.is_some() || flags.target.is_some() || flags.object.is_some())
    {
        return Err("--scope is mutually exclusive with --source/--target/--object".to_string());
    }
    if flags.scope.is_none()
        && flags.source.is_none()
        && flags.target.is_none()
        && flags.object.is_none()
    {
        return Err(
            "at least one of --source, --target, --object, or --scope is required".to_string(),
        );
    }
    Ok(())
}

/// Resolve the set of node ids selected by these flags.
///
/// `--scope` fans out `child-of` (transitively) plus `has-patch` and
/// `has-document` (per the locked design decision; `refers-to` is **not**
/// fanned out). Otherwise the flags translate directly into a single
/// `GET /v1/relations` call and the node id set is the union of source and
/// target ids across the returned edges.
pub async fn resolve_node_ids(
    client: &dyn HydraClientInterface,
    flags: &SelectionFlags,
) -> Result<Vec<HydraId>> {
    if let Some(scope) = &flags.scope {
        resolve_scope_node_ids(client, scope).await
    } else {
        let query = ListRelationsRequest {
            source_id: flags.source.clone(),
            source_ids: None,
            target_id: flags.target.clone(),
            target_ids: None,
            object_id: flags.object.clone(),
            rel_type: flags.rel_type.clone(),
            transitive: if flags.transitive { Some(true) } else { None },
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

/// Resolve `--scope <ID>` to the full node set per the design doc:
/// `{scope} ∪ descendants(child-of, transitive) ∪ has-patch targets ∪ has-document targets`.
/// `refers-to` is intentionally **not** fanned out.
async fn resolve_scope_node_ids(
    client: &dyn HydraClientInterface,
    scope: &HydraId,
) -> Result<Vec<HydraId>> {
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

    let mut s_set: Vec<HydraId> = vec![scope.clone()];
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(scope.as_ref().to_string());
    for edge in &descendants_response.relations {
        if seen.insert(edge.source_id.as_ref().to_string()) {
            s_set.push(edge.source_id.clone());
        }
    }

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
    use hydra_common::api::v1::relations::RelationResponse;

    fn empty_flags() -> SelectionFlags {
        SelectionFlags {
            source: None,
            target: None,
            object: None,
            rel_type: None,
            transitive: false,
            scope: None,
        }
    }

    #[test]
    fn validate_rejects_empty_selection() {
        let err = validate(&empty_flags()).unwrap_err();
        assert!(err.contains("at least one of"), "got: {err}");
    }

    #[test]
    fn validate_rejects_scope_with_source() {
        let flags = SelectionFlags {
            scope: Some("i-aaaaaa".parse().unwrap()),
            source: Some("i-bbbbbb".parse().unwrap()),
            ..empty_flags()
        };
        let err = validate(&flags).unwrap_err();
        assert!(err.contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn validate_rejects_scope_with_target() {
        let flags = SelectionFlags {
            scope: Some("i-aaaaaa".parse().unwrap()),
            target: Some("i-bbbbbb".parse().unwrap()),
            ..empty_flags()
        };
        let err = validate(&flags).unwrap_err();
        assert!(err.contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn validate_rejects_scope_with_object() {
        let flags = SelectionFlags {
            scope: Some("i-aaaaaa".parse().unwrap()),
            object: Some("i-bbbbbb".parse().unwrap()),
            ..empty_flags()
        };
        let err = validate(&flags).unwrap_err();
        assert!(err.contains("mutually exclusive"), "got: {err}");
    }

    #[test]
    fn validate_accepts_object_only() {
        let flags = SelectionFlags {
            object: Some("i-aaaaaa".parse().unwrap()),
            ..empty_flags()
        };
        assert!(validate(&flags).is_ok());
    }

    #[test]
    fn validate_accepts_scope_alone() {
        let flags = SelectionFlags {
            scope: Some("i-aaaaaa".parse().unwrap()),
            ..empty_flags()
        };
        assert!(validate(&flags).is_ok());
    }

    #[test]
    fn node_ids_from_edges_dedupes_and_preserves_order() {
        let response = ListRelationsResponse {
            relations: vec![
                RelationResponse {
                    source_id: "i-aaaaaa".parse().unwrap(),
                    target_id: "i-bbbbbb".parse().unwrap(),
                    rel_type: "child-of".to_string(),
                },
                RelationResponse {
                    source_id: "i-aaaaaa".parse().unwrap(),
                    target_id: "p-cccccc".parse().unwrap(),
                    rel_type: "has-patch".to_string(),
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
