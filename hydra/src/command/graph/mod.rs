//! `hydra graph` — query the knowledge graph (nodes, with version-aware views).
//!
//! This module currently exposes a single subcommand, `search`. PRs 4 and 5
//! will add `diff` and `log` alongside; the dispatch and node-set helpers in
//! `dispatch.rs` are shared with those subcommands.

pub mod dispatch;
pub mod search;

use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use hydra_common::graph::{ObjectKind, VerbosityLevel};
use hydra_common::HydraId;
use std::str::FromStr;

use crate::client::HydraClientInterface;
use crate::command::output::CommandContext;

/// Default cap on the resolved node-id set size, mirroring `MAX_BATCH_IDS`
/// on the server-side relations route.
pub const DEFAULT_MAX_NODES: usize = 10_000;

/// Maximum number of in-flight per-id hydration requests.
pub const DEFAULT_HYDRATION_CONCURRENCY: usize = 32;

#[derive(Debug, Subcommand)]
pub enum GraphCommand {
    /// Return the set of hydrated graph nodes matching a relation query.
    Search {
        /// Filter by source object ID.
        #[arg(long, value_name = "ID")]
        source: Option<HydraId>,

        /// Filter by target object ID.
        #[arg(long, value_name = "ID")]
        target: Option<HydraId>,

        /// Show all relations where this object is source or target.
        #[arg(long, value_name = "ID")]
        object: Option<HydraId>,

        /// Filter by relation type (e.g. child-of, blocked-on, has-patch).
        #[arg(long, value_name = "TYPE")]
        rel_type: Option<String>,

        /// Follow transitive edges (requires --source or --target plus --rel-type).
        #[arg(long)]
        transitive: bool,

        /// Convenience: include the issue plus all transitively-reachable
        /// child issues plus all attached patches and documents. Mutually
        /// exclusive with --source/--target/--object.
        #[arg(long, value_name = "ID")]
        scope: Option<HydraId>,

        /// Post-filter the hydrated nodes to one or more kinds (repeatable).
        #[arg(long = "kind", value_enum, value_name = "KIND")]
        kinds: Vec<KindArg>,

        /// Verbosity level (1 = terse, 2 = intermediate, 3 = full).
        #[arg(long, value_name = "LEVEL", default_value = "1")]
        verbosity: VerbosityArg,

        /// Maximum size of the resolved node-id set before aborting.
        #[arg(long, value_name = "N", default_value_t = DEFAULT_MAX_NODES)]
        max_nodes: usize,
    },
}

/// CLI-facing kind filter, repeatable as `--kind issue --kind patch ...`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum KindArg {
    Issue,
    Patch,
    Document,
    Conversation,
}

impl KindArg {
    pub fn as_object_kind(self) -> ObjectKind {
        match self {
            KindArg::Issue => ObjectKind::Issue,
            KindArg::Patch => ObjectKind::Patch,
            KindArg::Document => ObjectKind::Document,
            KindArg::Conversation => ObjectKind::Conversation,
        }
    }
}

/// Verbosity level (1, 2, or 3) parsed from the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerbosityArg(pub VerbosityLevel);

impl FromStr for VerbosityArg {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1" => Ok(Self(VerbosityLevel::L1)),
            "2" => Ok(Self(VerbosityLevel::L2)),
            "3" => Ok(Self(VerbosityLevel::L3)),
            _ => Err(format!("verbosity must be 1, 2, or 3 (got '{s}')")),
        }
    }
}

pub async fn run(
    client: &dyn HydraClientInterface,
    command: GraphCommand,
    context: &CommandContext,
) -> Result<()> {
    match command {
        GraphCommand::Search {
            source,
            target,
            object,
            rel_type,
            transitive,
            scope,
            kinds,
            verbosity,
            max_nodes,
        } => {
            search::run_search(
                client,
                search::SearchParams {
                    source,
                    target,
                    object,
                    rel_type,
                    transitive,
                    scope,
                    kinds,
                    verbosity: verbosity.0,
                    max_nodes,
                },
                context,
            )
            .await
        }
    }
}
