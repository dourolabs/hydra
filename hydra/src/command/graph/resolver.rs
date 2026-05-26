//! Walk a [`LoweredQuery`] against the relations API and produce a vertex set
//! plus the kind post-filters to apply after hydration.
//!
//! The resolver is the runtime counterpart to the parser/lowering library in
//! [`crate::command::graph::query`]. The parser yields a flat sequence of
//! [`LoweredStage`]s; the resolver issues one HTTP call per stage (three for
//! `scope`; zero for `Kind`), evolving the vertex set per the
//! inclusive-by-default contract documented in
//! `/designs/hydra-graph-query-language.md`.
//!
//! Shared between `hydra graph diff` (this PR) and — once PRs 3 and 5 land —
//! `hydra graph search` / `hydra graph log`. Callers feed `result.node_ids`
//! to the existing per-kind hydration path and apply each
//! `result.kind_filters` list to the hydrated set as a post-filter.
//!
//! # Bare-id fast path
//!
//! A single-element source with no following stages is resolved without any
//! `/v1/relations` call. Only the per-kind hydration call is issued.

use std::collections::HashSet;

use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use hydra_common::api::v1::relations::ListRelationsRequest;
use hydra_common::graph::ObjectKind;
use hydra_common::HydraId;

use crate::client::HydraClientInterface;
use crate::command::graph::query::{Direction, LoweredQuery, LoweredStage, RelationsQuery};

/// Output of [`resolve`].
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    /// Terminal vertex set, in stable insertion order (deduplicated).
    pub node_ids: Vec<HydraId>,
    /// Kind filters to apply post-hydration. Multiple lists are intersected
    /// against the hydrated set in the order they appear (which matches the
    /// pipe order in the original query). The lowering pass already collapses
    /// consecutive `kind=` stages into a single intersected list, so in
    /// practice this Vec has length 0 or 1; the API is a list for symmetry
    /// with the lowered stage shape.
    pub kind_filters: Vec<Vec<ObjectKind>>,
}

/// Walk `query` against the relations API. See module docs for semantics.
pub async fn resolve(client: &dyn HydraClientInterface, query: LoweredQuery) -> Result<Resolved> {
    let LoweredQuery { source, stages } = query;
    let mut vertices = dedupe(source);
    let mut kind_filters: Vec<Vec<ObjectKind>> = Vec::new();

    for stage in stages {
        if vertices.is_empty() {
            // The inclusive-by-default contract says empty `V` propagates: no
            // HTTP call is issued, the vertex set remains empty. `Kind`
            // filters still need to be recorded so the caller knows about
            // them, though they apply to an empty hydrated set.
            if let LoweredStage::Kind(ks) = stage {
                kind_filters.push(ks);
            }
            continue;
        }
        match stage {
            LoweredStage::Relations(q) => {
                vertices = apply_relations_stage(client, vertices, &q).await?;
            }
            LoweredStage::Scope => {
                vertices = apply_scope_stage(client, vertices).await?;
            }
            LoweredStage::Kind(ks) => {
                kind_filters.push(ks);
            }
        }
    }

    Ok(Resolved {
        node_ids: vertices,
        kind_filters,
    })
}

/// Issue one `/v1/relations` call for a [`RelationsQuery`] and apply the
/// inclusive-by-default vertex-set update rule.
async fn apply_relations_stage(
    client: &dyn HydraClientInterface,
    vertices: Vec<HydraId>,
    q: &RelationsQuery,
) -> Result<Vec<HydraId>> {
    let csv = csv_ids(&vertices);
    let request = match q.direction {
        Direction::Source => ListRelationsRequest {
            source_ids: Some(csv),
            rel_type: q.rel.map(|r| r.as_str().to_string()),
            transitive: if q.transitive { Some(true) } else { None },
            ..Default::default()
        },
        Direction::Target => ListRelationsRequest {
            target_ids: Some(csv),
            rel_type: q.rel.map(|r| r.as_str().to_string()),
            transitive: if q.transitive { Some(true) } else { None },
            ..Default::default()
        },
        Direction::Object => {
            // Use the singular `object_id` parameter when V has one element;
            // it's the original (pre-PR-1) shape and avoids touching the new
            // plural code path unnecessarily.
            if vertices.len() == 1 {
                ListRelationsRequest {
                    object_id: Some(vertices[0].clone()),
                    rel_type: q.rel.map(|r| r.as_str().to_string()),
                    ..Default::default()
                }
            } else {
                ListRelationsRequest {
                    object_ids: Some(csv),
                    rel_type: q.rel.map(|r| r.as_str().to_string()),
                    ..Default::default()
                }
            }
        }
    };

    let response = client
        .list_relations(&request)
        .await
        .context("failed to list relations")?;

    // Extract T(V): the per-direction set of nodes reached by the response.
    let mut traversed: Vec<HydraId> = Vec::new();
    let mut traversed_seen: HashSet<String> = HashSet::new();
    for edge in &response.relations {
        match q.direction {
            Direction::Source => push_unique(&mut traversed, &mut traversed_seen, &edge.target_id),
            Direction::Target => push_unique(&mut traversed, &mut traversed_seen, &edge.source_id),
            Direction::Object => {
                push_unique(&mut traversed, &mut traversed_seen, &edge.source_id);
                push_unique(&mut traversed, &mut traversed_seen, &edge.target_id);
            }
        }
    }

    let v_set: HashSet<String> = vertices.iter().map(|id| id.as_ref().to_string()).collect();

    if q.exclusive {
        Ok(traversed
            .into_iter()
            .filter(|id| !v_set.contains(id.as_ref()))
            .collect())
    } else {
        let mut out = vertices;
        let mut seen = v_set;
        for id in traversed {
            if seen.insert(id.as_ref().to_string()) {
                out.push(id);
            }
        }
        Ok(out)
    }
}

/// `scope` stage: 3 calls, distributed over the input vertex set. Output is
/// `V ∪ D ∪ P ∪ Doc` per the existing scope algorithm.
async fn apply_scope_stage(
    client: &dyn HydraClientInterface,
    vertices: Vec<HydraId>,
) -> Result<Vec<HydraId>> {
    let v_csv = csv_ids(&vertices);

    // 1. Descendants via child-of (transitive). The edges returned have
    //    target ∈ V; their `source_id`s are the descendants.
    let descendants_query = ListRelationsRequest {
        target_ids: Some(v_csv),
        rel_type: Some("child-of".to_string()),
        transitive: Some(true),
        ..Default::default()
    };
    let descendants_response = client
        .list_relations(&descendants_query)
        .await
        .context("failed to list child-of descendants for 'scope' stage")?;

    let mut out = vertices;
    let mut seen: HashSet<String> = out.iter().map(|id| id.as_ref().to_string()).collect();
    for edge in &descendants_response.relations {
        if seen.insert(edge.source_id.as_ref().to_string()) {
            out.push(edge.source_id.clone());
        }
    }

    // 2-3. has-patch and has-document children of V ∪ D, fetched in parallel.
    let source_ids_csv = csv_ids(&out);
    let mut futures = FuturesUnordered::new();
    for rel in ["has-patch", "has-document"] {
        let query = ListRelationsRequest {
            source_ids: Some(source_ids_csv.clone()),
            rel_type: Some(rel.to_string()),
            ..Default::default()
        };
        futures.push(async move { client.list_relations(&query).await });
    }
    while let Some(result) = futures.next().await {
        let response =
            result.context("failed to list has-patch/has-document targets for 'scope' stage")?;
        for edge in &response.relations {
            if seen.insert(edge.target_id.as_ref().to_string()) {
                out.push(edge.target_id.clone());
            }
        }
    }
    Ok(out)
}

/// Deduplicate a vertex Vec while preserving insertion order.
fn dedupe(ids: Vec<HydraId>) -> Vec<HydraId> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if seen.insert(id.as_ref().to_string()) {
            out.push(id);
        }
    }
    out
}

fn push_unique(out: &mut Vec<HydraId>, seen: &mut HashSet<String>, id: &HydraId) {
    if seen.insert(id.as_ref().to_string()) {
        out.push(id.clone());
    }
}

fn csv_ids(ids: &[HydraId]) -> String {
    ids.iter()
        .map(|id| id.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(",")
}
