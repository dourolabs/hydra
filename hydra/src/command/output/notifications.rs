use std::io::Write;

use anyhow::Result;
use hydra_common::{
    api::v1::notifications::{
        ListNotificationsResponse, MarkReadResponse, NotificationResponse, UnreadCountResponse,
    },
    NotificationId,
};

use super::Render;

pub struct MarkReadView<'a> {
    pub notification_id: &'a NotificationId,
    pub response: &'a MarkReadResponse,
}

impl Render for ListNotificationsResponse {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for notification in &self.notifications {
            serde_json::to_writer(&mut *writer, notification)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.notifications.is_empty() {
            writeln!(writer, "No notifications.")?;
        } else {
            for (index, notification) in self.notifications.iter().enumerate() {
                write_notification_pretty(notification, writer)?;
                if index + 1 < self.notifications.len() {
                    writeln!(writer)?;
                }
            }
        }
        writer.flush()?;
        Ok(())
    }
}

impl Render for UnreadCountResponse {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(writer, "{} unread notifications.", self.count)?;
        writer.flush()?;
        Ok(())
    }
}

impl Render for MarkReadView<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self.response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(
            writer,
            "Notification {} marked as read.",
            self.notification_id
        )?;
        writer.flush()?;
        Ok(())
    }
}

impl Render for MarkReadResponse {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(writer, "{} notifications marked as read.", self.marked)?;
        writer.flush()?;
        Ok(())
    }
}

fn write_notification_pretty<W: Write>(
    record: &NotificationResponse,
    writer: &mut W,
) -> Result<()> {
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
