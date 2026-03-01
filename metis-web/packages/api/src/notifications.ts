/** Types for the notifications API. */

export interface Notification {
  recipient: string;
  source_actor: string | null;
  object_kind: string;
  object_id: string;
  object_version: number;
  event_type: string;
  summary: string;
  source_issue_id: string | null;
  policy: string;
  is_read: boolean;
  created_at: string;
}

export interface NotificationResponse {
  notification_id: string;
  notification: Notification;
}

export interface ListNotificationsResponse {
  notifications: NotificationResponse[];
}

export interface UnreadCountResponse {
  count: number;
}

export interface MarkReadResponse {
  marked: number;
}

export interface ListNotificationsQuery {
  recipient: string | null;
  is_read: boolean | null;
  before: string | null;
  after: string | null;
  limit: number | null;
}
