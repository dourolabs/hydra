use crate::{
    client::MetisClientInterface,
    command::output::{render_relations, CommandContext},
};
use anyhow::{Context, Result};
use clap::Subcommand;
use metis_common::api::v1::relations::ListRelationsRequest;
use std::io;

#[derive(Debug, Subcommand)]
pub enum RelationsCommand {
    /// List relations between objects.
    List {
        /// Filter by source object ID.
        #[arg(long, value_name = "ID")]
        source: Option<String>,

        /// Filter by target object ID.
        #[arg(long, value_name = "ID")]
        target: Option<String>,

        /// Show all relations where this object is source or target.
        #[arg(long, value_name = "ID")]
        object: Option<String>,

        /// Filter by relation type (e.g. child-of, blocked-on, has-patch).
        #[arg(long, value_name = "TYPE")]
        rel_type: Option<String>,

        /// Follow transitive edges (requires --source or --target plus --rel-type).
        #[arg(long)]
        transitive: bool,
    },
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: RelationsCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match command {
        RelationsCommand::List {
            source,
            target,
            object,
            rel_type,
            transitive,
        } => {
            let query = ListRelationsRequest {
                source_id: source,
                source_ids: None,
                target_id: target,
                target_ids: None,
                object_id: object,
                rel_type,
                transitive: if transitive { Some(true) } else { None },
            };
            let response = client
                .list_relations(&query)
                .await
                .context("failed to list relations")?;
            render_relations(context.output_format, &response, &mut stdout)?;
        }
    }
    Ok(())
}
