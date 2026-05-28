//! `hydra graph create` / `hydra graph delete` — implementation.
//!
//! Both subcommands take a positional `<FROM_ID>` plus a positional
//! `<REL>:<TO_ID>` argument and map them straight onto the server's
//! `(source_id, rel_type, target_id)` triple. The parser accepts the same
//! aliases as the server's `RelationshipType::FromStr` (kebab-case canonical
//! plus the `childof` / `child_of` collapses) for all five relation kinds.

use std::io::Write;
use std::process;
use std::str::FromStr;

use anyhow::Result;
use hydra_common::api::v1::relations::{CreateRelationRequest, RemoveRelationRequest};
use hydra_common::HydraId;
use serde_json::json;

use crate::client::HydraClientInterface;
use crate::command::output::{CommandContext, ResolvedOutputFormat};
use crate::output_writer::write_stdout;

/// The relation kinds accepted by `hydra graph create` / `hydra graph delete`.
///
/// Mirrors `hydra-server`'s `store::RelationshipType`. Kept as a CLI-side
/// enum so the CLI does not have to depend on `hydra-server`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationKindCli {
    ChildOf,
    BlockedOn,
    HasPatch,
    HasDocument,
    RefersTo,
}

impl RelationKindCli {
    /// Canonical kebab-case string accepted by the server.
    pub fn as_str(self) -> &'static str {
        match self {
            RelationKindCli::ChildOf => "child-of",
            RelationKindCli::BlockedOn => "blocked-on",
            RelationKindCli::HasPatch => "has-patch",
            RelationKindCli::HasDocument => "has-document",
            RelationKindCli::RefersTo => "refers-to",
        }
    }
}

impl FromStr for RelationKindCli {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Mirror the aliases accepted by hydra-server's
        // `RelationshipType::FromStr` so the CLI accepts the same tokens
        // the server already accepts on the wire.
        let value = s.trim().to_ascii_lowercase();
        match value.as_str() {
            "child-of" | "childof" | "child_of" => Ok(RelationKindCli::ChildOf),
            "blocked-on" | "blockedon" | "blocked_on" => Ok(RelationKindCli::BlockedOn),
            "has-patch" | "haspatch" | "has_patch" => Ok(RelationKindCli::HasPatch),
            "has-document" | "hasdocument" | "has_document" => Ok(RelationKindCli::HasDocument),
            "refers-to" | "refersto" | "refers_to" => Ok(RelationKindCli::RefersTo),
            _ => Err(format!(
                "unsupported relation kind '{s}'; expected one of: \
                 child-of, blocked-on, has-patch, has-document, refers-to",
            )),
        }
    }
}

/// Parsed `<REL>:<TO_ID>` positional. Used by both `graph create` and
/// `graph delete`.
#[derive(Debug, Clone)]
pub struct RelTarget {
    pub rel: RelationKindCli,
    pub target: HydraId,
}

/// Parse a `<REL>:<ID>` positional. The format is identical to the existing
/// `--deps child-of:i-abcd` shape used by `hydra issues create`, just
/// extended to cover all five relation kinds.
pub fn parse_rel_target(raw: &str) -> Result<RelTarget, String> {
    let (rel, id) = raw.split_once(':').ok_or_else(|| {
        "relation must be in the format REL:ID (one of: child-of, blocked-on, \
         has-patch, has-document, refers-to)"
            .to_string()
    })?;
    let rel = RelationKindCli::from_str(rel)?;
    let target = id
        .trim()
        .parse::<HydraId>()
        .map_err(|err| err.to_string())?;
    Ok(RelTarget { rel, target })
}

/// Inputs to [`run_create`] / [`run_delete`] after CLI parsing.
pub struct RelateParams {
    pub from: HydraId,
    pub target: RelTarget,
}

/// `hydra graph create` entry point.
pub async fn run_create(
    client: &dyn HydraClientInterface,
    params: RelateParams,
    context: &CommandContext,
) -> Result<()> {
    let RelateParams { from, target } = params;
    let created = client
        .create_relation(&CreateRelationRequest {
            source_id: from.clone(),
            target_id: target.target.clone(),
            rel_type: target.rel.as_str().to_string(),
        })
        .await?;

    let mut buffer = Vec::new();
    render_create(
        context.output_format,
        &from,
        target.rel,
        &target.target,
        created,
        &mut buffer,
    )?;
    write_stdout(&buffer)?;
    Ok(())
}

/// `hydra graph delete` entry point. Exits with code 1 when the relation did
/// not exist (`removed: false`), so scripts can detect the no-op.
pub async fn run_delete(
    client: &dyn HydraClientInterface,
    params: RelateParams,
    context: &CommandContext,
) -> Result<()> {
    let RelateParams { from, target } = params;
    let removed = client
        .remove_relation(&RemoveRelationRequest {
            source_id: from.clone(),
            target_id: target.target.clone(),
            rel_type: target.rel.as_str().to_string(),
        })
        .await?;

    let mut buffer = Vec::new();
    render_delete(
        context.output_format,
        &from,
        target.rel,
        &target.target,
        removed,
        &mut buffer,
    )?;
    write_stdout(&buffer)?;

    if !removed {
        process::exit(1);
    }
    Ok(())
}

fn render_create(
    format: ResolvedOutputFormat,
    from: &HydraId,
    rel: RelationKindCli,
    target: &HydraId,
    created: bool,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            let value = json!({
                "source_id": from.as_ref(),
                "target_id": target.as_ref(),
                "rel_type": rel.as_str(),
                "created": created,
            });
            serde_json::to_writer(&mut *writer, &value)?;
            writer.write_all(b"\n")?;
        }
        ResolvedOutputFormat::Pretty => {
            let prefix = if created {
                "created relation"
            } else {
                "relation already exists"
            };
            writeln!(
                writer,
                "{}: {} {} {}",
                prefix,
                from.as_ref(),
                rel.as_str(),
                target.as_ref()
            )?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn render_delete(
    format: ResolvedOutputFormat,
    from: &HydraId,
    rel: RelationKindCli,
    target: &HydraId,
    removed: bool,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            let value = json!({
                "source_id": from.as_ref(),
                "target_id": target.as_ref(),
                "rel_type": rel.as_str(),
                "removed": removed,
            });
            serde_json::to_writer(&mut *writer, &value)?;
            writer.write_all(b"\n")?;
        }
        ResolvedOutputFormat::Pretty => {
            let prefix = if removed {
                "deleted relation"
            } else {
                "relation not found"
            };
            writeln!(
                writer,
                "{}: {} {} {}",
                prefix,
                from.as_ref(),
                rel.as_str(),
                target.as_ref()
            )?;
        }
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rel_target_accepts_all_five_kinds() {
        for (raw, expected) in [
            ("child-of:i-abcdef", RelationKindCli::ChildOf),
            ("blocked-on:i-abcdef", RelationKindCli::BlockedOn),
            ("has-patch:p-abcdef", RelationKindCli::HasPatch),
            ("has-document:d-abcdef", RelationKindCli::HasDocument),
            ("refers-to:i-abcdef", RelationKindCli::RefersTo),
        ] {
            let parsed = parse_rel_target(raw).unwrap_or_else(|e| panic!("parse {raw}: {e}"));
            assert_eq!(parsed.rel, expected, "wrong kind for {raw}");
        }
    }

    #[test]
    fn parse_rel_target_accepts_camel_and_snake_aliases() {
        for raw in [
            "childof:i-abcdef",
            "child_of:i-abcdef",
            "ChildOf:i-abcdef",
            "BLOCKED_ON:i-abcdef",
            "refersto:i-abcdef",
        ] {
            assert!(parse_rel_target(raw).is_ok(), "alias '{raw}' should parse",);
        }
    }

    #[test]
    fn parse_rel_target_rejects_missing_colon() {
        let err = parse_rel_target("child-of i-abcdef").unwrap_err();
        assert!(
            err.contains("REL:ID"),
            "error should mention the canonical format: {err}",
        );
        assert!(
            err.contains("child-of") && err.contains("refers-to"),
            "error should list supported kinds: {err}",
        );
    }

    #[test]
    fn parse_rel_target_rejects_unknown_kind() {
        let err = parse_rel_target("bogus:i-abcdef").unwrap_err();
        assert!(
            err.contains("bogus"),
            "error should echo the bad token: {err}",
        );
        assert!(
            err.contains("child-of") && err.contains("refers-to"),
            "error should list supported kinds: {err}",
        );
    }

    #[test]
    fn parse_rel_target_rejects_bad_id() {
        let err = parse_rel_target("child-of:not-an-id").unwrap_err();
        // Whatever HydraId's parser says, the error should not be empty:
        assert!(!err.is_empty(), "expected a non-empty parse error");
    }

    fn parse_id(raw: &str) -> HydraId {
        raw.parse().expect("valid hydra id")
    }

    #[test]
    fn render_create_pretty_emits_pretty_line_when_created() {
        let mut buf = Vec::new();
        render_create(
            ResolvedOutputFormat::Pretty,
            &parse_id("i-aaaaaa"),
            RelationKindCli::ChildOf,
            &parse_id("i-bbbbbb"),
            true,
            &mut buf,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "created relation: i-aaaaaa child-of i-bbbbbb\n",
        );
    }

    #[test]
    fn render_create_pretty_emits_already_exists_when_not_created() {
        let mut buf = Vec::new();
        render_create(
            ResolvedOutputFormat::Pretty,
            &parse_id("i-aaaaaa"),
            RelationKindCli::ChildOf,
            &parse_id("i-bbbbbb"),
            false,
            &mut buf,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "relation already exists: i-aaaaaa child-of i-bbbbbb\n",
        );
    }

    #[test]
    fn render_create_jsonl_includes_created_flag() {
        let mut buf = Vec::new();
        render_create(
            ResolvedOutputFormat::Jsonl,
            &parse_id("i-aaaaaa"),
            RelationKindCli::HasPatch,
            &parse_id("p-bbbbbb"),
            true,
            &mut buf,
        )
        .unwrap();
        let line = String::from_utf8(buf).unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["source_id"], "i-aaaaaa");
        assert_eq!(value["target_id"], "p-bbbbbb");
        assert_eq!(value["rel_type"], "has-patch");
        assert_eq!(value["created"], true);
    }

    #[test]
    fn render_delete_pretty_emits_deleted_line_when_removed() {
        let mut buf = Vec::new();
        render_delete(
            ResolvedOutputFormat::Pretty,
            &parse_id("i-aaaaaa"),
            RelationKindCli::RefersTo,
            &parse_id("i-bbbbbb"),
            true,
            &mut buf,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "deleted relation: i-aaaaaa refers-to i-bbbbbb\n",
        );
    }

    #[test]
    fn render_delete_pretty_emits_not_found_when_missing() {
        let mut buf = Vec::new();
        render_delete(
            ResolvedOutputFormat::Pretty,
            &parse_id("i-aaaaaa"),
            RelationKindCli::ChildOf,
            &parse_id("i-bbbbbb"),
            false,
            &mut buf,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "relation not found: i-aaaaaa child-of i-bbbbbb\n",
        );
    }

    #[test]
    fn render_delete_jsonl_includes_removed_flag() {
        let mut buf = Vec::new();
        render_delete(
            ResolvedOutputFormat::Jsonl,
            &parse_id("i-aaaaaa"),
            RelationKindCli::BlockedOn,
            &parse_id("i-bbbbbb"),
            false,
            &mut buf,
        )
        .unwrap();
        let line = String::from_utf8(buf).unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["source_id"], "i-aaaaaa");
        assert_eq!(value["target_id"], "i-bbbbbb");
        assert_eq!(value["rel_type"], "blocked-on");
        assert_eq!(value["removed"], false);
    }
}
