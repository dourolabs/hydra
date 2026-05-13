/**
 * Singleton registry that multiplexes per-session log streams over the global
 * `/api/v1/events` EventSource owned by `useSSE`. Each `SessionLogViewer`
 * subscribes here on mount; the registry exposes the active session ID set so
 * `useSSE` can include `session_ids` in its EventSource URL and route incoming
 * `session_log` events back to the right viewer(s).
 *
 * Lives outside the React tree so subscribers don't need to share state with
 * the hook that owns the EventSource.
 */
export type SessionLogChunkHandler = (chunk: string) => void;

class SessionLogRegistry {
  private handlers = new Map<string, Set<SessionLogChunkHandler>>();
  private changeListeners = new Set<() => void>();

  /**
   * Register `handler` to receive log chunks for `sessionId`. Returns an
   * unsubscribe function; callers must invoke it on unmount.
   */
  subscribe(sessionId: string, handler: SessionLogChunkHandler): () => void {
    let set = this.handlers.get(sessionId);
    if (!set) {
      set = new Set();
      this.handlers.set(sessionId, set);
    }
    set.add(handler);
    this.notifyChange();

    return () => {
      const current = this.handlers.get(sessionId);
      if (!current) return;
      current.delete(handler);
      if (current.size === 0) {
        this.handlers.delete(sessionId);
      }
      this.notifyChange();
    };
  }

  /** Dispatch a log chunk to every subscriber of `sessionId`. */
  dispatch(sessionId: string, chunk: string): void {
    this.handlers.get(sessionId)?.forEach((h) => h(chunk));
  }

  /** Sorted list of session IDs with at least one active subscriber. */
  sessionIds(): string[] {
    return Array.from(this.handlers.keys()).sort();
  }

  /**
   * Subscribe to subscriber-set changes. The callback fires whenever a session
   * ID is added or removed; `useSSE` uses this to rebuild the EventSource URL.
   */
  onChange(listener: () => void): () => void {
    this.changeListeners.add(listener);
    return () => {
      this.changeListeners.delete(listener);
    };
  }

  private notifyChange(): void {
    this.changeListeners.forEach((cb) => cb());
  }
}

export const sessionLogRegistry = new SessionLogRegistry();
