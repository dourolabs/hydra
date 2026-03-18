use crate::{
    client::HydraClientInterface,
    command::output::{
        render_mark_all_read, render_mark_read, render_notifications, render_unread_count,
        CommandContext,
    },
};
use anyhow::{Context, Result};
use clap::Subcommand;
use hydra_common::{api::v1::notifications::ListNotificationsQuery, NotificationId};
use std::io;

#[derive(Debug, Subcommand)]
pub enum NotificationsCommand {
    /// List notifications for the current actor.
    List {
        /// Only show unread notifications.
        #[arg(long)]
        unread: bool,

        /// Maximum number of notifications to return.
        #[arg(long, value_name = "N", default_value_t = 50)]
        limit: u32,
    },
    /// Show the count of unread notifications.
    Count,
    /// Mark a single notification as read.
    Read {
        /// The notification ID to mark as read.
        #[arg(value_name = "NOTIFICATION_ID")]
        notification_id: NotificationId,
    },
    /// Mark all notifications as read.
    ReadAll,
}

pub async fn run(
    client: &dyn HydraClientInterface,
    command: NotificationsCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match command {
        NotificationsCommand::List { unread, limit } => {
            let mut query = ListNotificationsQuery::default();
            if unread {
                query.is_read = Some(false);
            }
            query.limit = Some(limit);
            let response = client
                .list_notifications(&query)
                .await
                .context("failed to list notifications")?;
            render_notifications(context.output_format, &response, &mut stdout)?;
        }
        NotificationsCommand::Count => {
            let response = client
                .get_unread_notification_count()
                .await
                .context("failed to get unread notification count")?;
            render_unread_count(context.output_format, &response, &mut stdout)?;
        }
        NotificationsCommand::Read { notification_id } => {
            let response = client
                .mark_notification_read(&notification_id)
                .await
                .context("failed to mark notification as read")?;
            render_mark_read(
                context.output_format,
                &notification_id,
                &response,
                &mut stdout,
            )?;
        }
        NotificationsCommand::ReadAll => {
            let response = client
                .mark_all_notifications_read(None)
                .await
                .context("failed to mark all notifications as read")?;
            render_mark_all_read(context.output_format, &response, &mut stdout)?;
        }
    }
    Ok(())
}
