use crate::{
    client::MetisClientInterface,
    command::output::{CommandContext, ResolvedOutputFormat},
};
use anyhow::{Context, Result};
use clap::Subcommand;
use metis_common::{
    api::v1::notifications::{
        ListNotificationsQuery, ListNotificationsResponse, MarkReadResponse, NotificationResponse,
        UnreadCountResponse,
    },
    NotificationId,
};
use std::io::{self, Write};

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
    client: &dyn MetisClientInterface,
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

fn render_notifications(
    format: ResolvedOutputFormat,
    response: &ListNotificationsResponse,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            for notification in &response.notifications {
                serde_json::to_writer(&mut *writer, notification)?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            if response.notifications.is_empty() {
                writeln!(writer, "No notifications.")?;
            } else {
                for (index, notification) in response.notifications.iter().enumerate() {
                    write_notification_pretty(notification, writer)?;
                    if index + 1 < response.notifications.len() {
                        writeln!(writer)?;
                    }
                }
            }
            writer.flush()?;
        }
    }
    Ok(())
}

fn write_notification_pretty(record: &NotificationResponse, writer: &mut impl Write) -> Result<()> {
    let read_status = if record.notification.is_read {
        "read"
    } else {
        "unread"
    };
    writeln!(
        writer,
        "Notification {} [{}]",
        record.notification_id, read_status
    )?;
    writeln!(writer, "  summary: {}", record.notification.summary)?;
    writeln!(
        writer,
        "  object: {} {}",
        record.notification.object_kind, record.notification.object_id
    )?;
    writeln!(writer, "  event: {}", record.notification.event_type)?;
    if let Some(ref source) = record.notification.source_actor {
        writeln!(writer, "  source: {source}")?;
    }
    writeln!(writer, "  time: {}", record.notification.created_at)?;
    Ok(())
}

fn render_unread_count(
    format: ResolvedOutputFormat,
    response: &UnreadCountResponse,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            writeln!(writer, "{} unread notifications.", response.count)?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn render_mark_read(
    format: ResolvedOutputFormat,
    notification_id: &NotificationId,
    response: &MarkReadResponse,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            writeln!(writer, "Notification {notification_id} marked as read.")?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn render_mark_all_read(
    format: ResolvedOutputFormat,
    response: &MarkReadResponse,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            writeln!(writer, "{} notifications marked as read.", response.marked)?;
            writer.flush()?;
        }
    }
    Ok(())
}
