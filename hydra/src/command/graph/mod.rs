//! `hydra graph` — query the knowledge graph (nodes, with version-aware views).
//!
//! Exposed subcommands: `search`, `diff`, and `log` all consume the
//! positional pipe-grammar query parsed in `hydra_common::graph::query` and
//! walked by [`resolver`]. Per-kind hydration and version-history fetching
//! live in [`dispatch`].

pub mod diff;
pub mod dispatch;
pub mod log;
pub mod resolver;
pub mod search;
pub mod utils;

use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use hydra_common::graph::{ObjectKind, VerbosityLevel};
use hydra_common::time::HydraTime;
use std::str::FromStr;

use crate::client::HydraClientInterface;
use crate::command::output::CommandContext;

/// Default cap on the resolved node-id set size, mirroring `MAX_BATCH_IDS`
/// on the server-side relations route.
pub const DEFAULT_MAX_NODES: usize = 10_000;

/// Long-form `--help` block for `hydra graph search`. Replaces the prior
/// flag-by-flag listing with a grammar one-liner plus three worked examples
/// and a pointer at the long-form reference (lands in PR 6).
pub const SEARCH_LONG_ABOUT: &str = "\
Return the set of hydrated graph nodes matching a pipe-grammar query.

Grammar: SOURCE_IDS ('|' STAGE)* where STAGE is one of:
  parents [rel=R] [transitive] [exclusive]
  children [rel=R] [transitive] [exclusive]
  neighbors [rel=R] [exclusive]
  ancestors rel=R [exclusive]
  descendants rel=R [exclusive]
  scope
  kind=K[,K...]

Relation stages are inclusive by default (V | stage = V ∪ traversal(V));
add `exclusive` to drop the seed.

Examples:
  hydra graph search 'i-abc123'
  hydra graph search 'i-abc123 | scope | kind=patch'
  hydra graph search 'i-abc123 | descendants rel=child-of'

See hydra/docs/graph-query.md for the full reference.";

/// Maximum number of in-flight per-id hydration requests.
pub const DEFAULT_HYDRATION_CONCURRENCY: usize = 32;

/// Default cap on the number of log events emitted by `hydra graph log`.
pub const DEFAULT_LOG_LIMIT: usize = 50;

/// Default `--since` value (Unix epoch, i.e. "from the beginning of time")
/// used when the user omits the flag on `diff` / `log`.
pub const DEFAULT_SINCE: &str = "1970-01-01T00:00:00Z";

#[derive(Debug, Subcommand)]
pub enum GraphCommand {
    /// Return the set of hydrated graph nodes matching a pipe-grammar query.
    #[command(long_about = SEARCH_LONG_ABOUT)]
    Search {
        /// Pipe-grammar query (single-quote it at the shell to protect '|').
        ///
        /// Grammar: SOURCE_IDS ('|' STAGE)*. Stages: parents, children,
        /// neighbors, ancestors, descendants, scope, kind=KINDS. See
        /// `hydra/docs/graph-query.md` for the full reference.
        #[arg(value_name = "QUERY")]
        query: String,

        /// Verbosity level (1 = terse, 2 = intermediate, 3 = full).
        #[arg(long, value_name = "LEVEL", default_value = "1")]
        verbosity: VerbosityArg,

        /// Maximum size of the resolved node-id set before aborting.
        #[arg(long, value_name = "N", default_value_t = DEFAULT_MAX_NODES)]
        max_nodes: usize,
    },
    /// Show what changed between two timestamps for the matched nodes.
    Diff {
        /// Pipe-grammar query (see `hydra graph --help` for syntax).
        ///
        /// Examples:
        ///   i-abc123                                # single node
        ///   'i-abc123 | scope'                      # i-abc123 + descendants + patches + documents
        ///   'i-abc123 | children rel=child-of'      # i-abc123 + direct children
        ///   'i-abc123 | scope | kind=patch'         # patches in scope, post-filtered
        ///
        /// Grammar: SOURCE (`|` STAGE)*, where SOURCE is one or more
        /// comma-separated ids and STAGE is one of `parents`, `children`,
        /// `neighbors`, `ancestors rel=R`, `descendants rel=R`, `scope`, or
        /// `kind=K[,K…]`. Relation stages default to inclusive
        /// (V ∪ traversal(V)); add the bare `exclusive` keyword to drop the
        /// seeds. Quote the query in your shell to protect `|`.
        #[arg(value_name = "QUERY")]
        query: String,

        /// Start of the time window (RFC 3339 timestamp, '-Nh'/'-Nd'
        /// relative duration, or 'now'). Optional; when omitted, defaults to
        /// the Unix epoch (i.e. "from the beginning of time").
        #[arg(
            long,
            value_name = "TS",
            default_value = DEFAULT_SINCE,
            allow_hyphen_values = true,
        )]
        since: HydraTime,

        /// End of the time window (same syntax as --since). Defaults to 'now'.
        #[arg(
            long,
            value_name = "TS",
            default_value = "now",
            allow_hyphen_values = true
        )]
        until: HydraTime,

        /// Verbosity level (1 = terse, 2 = intermediate, 3 = full).
        #[arg(long, value_name = "LEVEL", default_value = "1")]
        verbosity: VerbosityArg,

        /// Maximum size of the resolved node-id set before aborting.
        #[arg(long, value_name = "N", default_value_t = DEFAULT_MAX_NODES)]
        max_nodes: usize,
    },
    /// Stream a time-ordered event log of `created` / `updated` records for
    /// the matched nodes.
    ///
    /// Selection uses the pipe-form query DSL (see `hydra graph --help` for
    /// syntax). Examples:
    ///   hydra graph log 'i-abc123' --since -7d
    ///   hydra graph log 'i-abc123 | scope' --since -7d --verbosity 2
    ///   hydra graph log 'i-abc123 | scope | kind=patch' --since -7d --limit 10
    Log {
        /// Pipe-form query selecting the node set to stream events for.
        ///
        /// Grammar: `<id>[,<id>...] [| <stage>]*`. Stages: `parents`,
        /// `children`, `neighbors`, `ancestors`, `descendants`, `scope`,
        /// `kind=<list>`. Relation stages accept `rel=<type>`, optional
        /// `transitive`, optional `exclusive`. Inclusive-by-default: the
        /// seed set is preserved through each relation stage unless
        /// `exclusive` is specified.
        #[arg(value_name = "QUERY")]
        query: String,

        /// Start of the time window (RFC 3339 timestamp, '-Nh'/'-Nd'
        /// relative duration, or 'now'). Optional; when omitted, defaults to
        /// the Unix epoch (i.e. "from the beginning of time").
        #[arg(
            long,
            value_name = "TS",
            default_value = DEFAULT_SINCE,
            allow_hyphen_values = true,
        )]
        since: HydraTime,

        /// End of the time window (same syntax as --since). Defaults to 'now'.
        #[arg(
            long,
            value_name = "TS",
            default_value = "now",
            allow_hyphen_values = true
        )]
        until: HydraTime,

        /// Verbosity level (1 = terse, 2 = intermediate, 3 = full).
        #[arg(long, value_name = "LEVEL", default_value = "1")]
        verbosity: VerbosityArg,

        /// Maximum number of events to emit (most recent first). Defaults to 50.
        #[arg(long, value_name = "N", default_value_t = DEFAULT_LOG_LIMIT)]
        limit: usize,

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
            query,
            verbosity,
            max_nodes,
        } => {
            search::run_search(
                client,
                search::SearchParams {
                    query,
                    verbosity: verbosity.0,
                    max_nodes,
                },
                context,
            )
            .await
        }
        GraphCommand::Diff {
            query,
            since,
            until,
            verbosity,
            max_nodes,
        } => {
            diff::run_diff(
                client,
                diff::DiffParams {
                    query,
                    since,
                    until,
                    verbosity: verbosity.0,
                    max_nodes,
                },
                context,
            )
            .await
        }
        GraphCommand::Log {
            query,
            since,
            until,
            verbosity,
            limit,
            max_nodes,
        } => {
            log::run_log(
                client,
                log::LogParams {
                    query,
                    since,
                    until,
                    verbosity: verbosity.0,
                    limit,
                    max_nodes,
                },
                context,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use chrono::{DateTime, Utc};
    use clap::Parser;

    fn parse_graph(args: &[&str]) -> GraphCommand {
        let mut full = vec!["hydra", "graph"];
        full.extend_from_slice(args);
        let cli = Cli::try_parse_from(full).expect("parse");
        match cli.command.expect("command") {
            Commands::Graph { command } => command,
            _ => panic!("expected Graph subcommand"),
        }
    }

    fn epoch() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(0, 0).unwrap()
    }

    #[test]
    fn log_default_since_is_unix_epoch() {
        let cmd = parse_graph(&["log", "i-abcdef"]);
        match cmd {
            GraphCommand::Log { since, query, .. } => {
                assert_eq!(since.into_inner(), epoch());
                assert_eq!(query, "i-abcdef");
            }
            other => panic!("expected Log, got {other:?}"),
        }
    }

    #[test]
    fn diff_default_since_is_unix_epoch() {
        let cmd = parse_graph(&["diff", "i-abcdef"]);
        match cmd {
            GraphCommand::Diff { since, .. } => {
                assert_eq!(since.into_inner(), epoch());
            }
            other => panic!("expected Diff, got {other:?}"),
        }
    }

    #[test]
    fn diff_takes_positional_query() {
        let cmd = parse_graph(&["diff", "i-abcdef | scope"]);
        match cmd {
            GraphCommand::Diff { query, .. } => {
                assert_eq!(query, "i-abcdef | scope");
            }
            other => panic!("expected Diff, got {other:?}"),
        }
    }

    #[test]
    fn diff_rejects_removed_selection_flags() {
        use clap::Parser;
        for flag in [
            "--source",
            "--target",
            "--object",
            "--rel-type",
            "--scope",
            "--kind",
        ] {
            let result =
                Cli::try_parse_from(["hydra", "graph", "diff", "i-abcdef", flag, "i-other"]);
            assert!(
                result.is_err(),
                "expected clap to reject removed flag '{flag}' on `graph diff`",
            );
        }
        // --transitive is a bare bool, parse without value:
        let result = Cli::try_parse_from(["hydra", "graph", "diff", "i-abcdef", "--transitive"]);
        assert!(
            result.is_err(),
            "expected clap to reject removed flag '--transitive' on `graph diff`",
        );
    }

    #[test]
    fn log_default_limit_is_50() {
        let cmd = parse_graph(&["log", "i-abcdef"]);
        match cmd {
            GraphCommand::Log { limit, .. } => assert_eq!(limit, 50),
            other => panic!("expected Log, got {other:?}"),
        }
    }

    #[test]
    fn log_explicit_since_overrides_default() {
        let cmd = parse_graph(&["log", "i-abcdef", "--since", "2026-05-01T00:00:00Z"]);
        match cmd {
            GraphCommand::Log { since, .. } => {
                let expected: DateTime<Utc> =
                    "2026-05-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
                assert_eq!(since.into_inner(), expected);
            }
            other => panic!("expected Log, got {other:?}"),
        }
    }

    #[test]
    fn log_explicit_limit_overrides_default() {
        let cmd = parse_graph(&["log", "i-abcdef", "--limit", "200"]);
        match cmd {
            GraphCommand::Log { limit, .. } => assert_eq!(limit, 200),
            other => panic!("expected Log, got {other:?}"),
        }
    }

    #[test]
    fn log_accepts_pipe_grammar_query() {
        let cmd = parse_graph(&["log", "i-abcdef | scope | kind=patch"]);
        match cmd {
            GraphCommand::Log { query, .. } => {
                assert_eq!(query, "i-abcdef | scope | kind=patch");
            }
            other => panic!("expected Log, got {other:?}"),
        }
    }

    #[test]
    fn log_rejects_deleted_source_flag() {
        let res = Cli::try_parse_from(["hydra", "graph", "log", "--source", "i-abcdef"]);
        let err = match res {
            Ok(_) => panic!("clap should reject --source after the cutover"),
            Err(e) => e,
        };
        let msg = err.to_string();
        // clap's "unexpected argument" or "unrecognized argument" error.
        assert!(
            msg.contains("--source") || msg.contains("unexpected"),
            "expected clap to flag --source as unknown, got: {msg}",
        );
    }
}
