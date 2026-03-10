/**
 * Apply cursor-based pagination to items sorted by timestamp DESC.
 */
export function applyPagination<T extends { timestamp: string; id: string }>(
  items: T[],
  limit: number | null,
  cursor: string | null,
): { page: T[]; nextCursor: string | null } {
  const sorted = [...items].sort((a, b) => b.timestamp.localeCompare(a.timestamp));

  let startIdx = 0;
  if (cursor) {
    try {
      const decoded = Buffer.from(cursor, "base64").toString("utf-8");
      const sepIdx = decoded.lastIndexOf(":");
      const cursorTs = decoded.slice(0, sepIdx);
      const cursorId = decoded.slice(sepIdx + 1);
      startIdx = sorted.findIndex(
        (item) => item.timestamp < cursorTs || (item.timestamp === cursorTs && item.id <= cursorId),
      );
      if (startIdx < 0) startIdx = sorted.length;
    } catch {
      startIdx = 0;
    }
  }

  if (limit === null) {
    return { page: sorted.slice(startIdx), nextCursor: null };
  }

  const page = sorted.slice(startIdx, startIdx + limit);
  const hasMore = startIdx + limit < sorted.length;
  let nextCursor: string | null = null;
  if (hasMore && page.length > 0) {
    const last = page[page.length - 1];
    nextCursor = Buffer.from(`${last.timestamp}:${last.id}`).toString("base64");
  }
  return { page, nextCursor };
}
