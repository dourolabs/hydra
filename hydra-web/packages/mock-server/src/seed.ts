import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import type { Store } from "./store.js";
import type {
  Issue,
  Session,
  Patch,
  Document,
  Repository,
  AgentRecord,
  Conversation,
  ConversationEvent,
  SessionEvent,
} from "@hydra/api";
import { clearAssociations, addAssociation } from "./routes/labels.js";
import { clearConversationEvents, setConversationEvents } from "./routes/conversations.js";
import { clearSessionEvents, setSessionEvents } from "./routes/sessions.js";
import { clearSeededRelations, addSeededRelation } from "./routes/relations.js";

interface LabelData {
  name: string;
  color: string;
  recurse?: boolean;
  hidden?: boolean;
}

interface LabelAssociationSeed {
  label_id: string;
  object_id: string;
}

// A fixture entity may carry an optional `history` array describing prior
// versions (oldest first). Each entry is a partial diff merged onto the
// previous version. The fixture's top-level state (minus `history` and
// `last_updated_at`) is the latest version.
interface VersionedFixture<T> {
  history?: VersionDelta<T>[];
  last_updated_at?: string;
}

interface VersionDelta<T> {
  timestamp: string;
  patch: Partial<T>;
}

type IssueFixture = Issue & VersionedFixture<Issue>;
type SessionFixture = Session & VersionedFixture<Session>;
type PatchFixture = Patch & VersionedFixture<Patch>;

interface RelationSeed {
  source_id: string;
  target_id: string;
  rel_type: string;
}

interface SeedData {
  issues: Record<string, IssueFixture>;
  sessions: Record<string, SessionFixture>;
  patches: Record<string, PatchFixture>;
  documents: Record<string, Document>;
  repositories: Record<string, Repository>;
  agents: Record<string, AgentRecord>;
  labels?: Record<string, LabelData>;
  label_associations?: LabelAssociationSeed[];
  conversations?: Record<string, Conversation>;
  conversation_events?: Record<string, ConversationEvent[]>;
  session_events?: Record<string, SessionEvent[]>;
  relations?: RelationSeed[];
}

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = resolve(__dirname, "../fixtures/seed.json");

function loadFixture(): SeedData {
  const raw = readFileSync(FIXTURE_PATH, "utf-8");
  return JSON.parse(raw) as SeedData;
}

function splitVersioned<T>(fixture: T & VersionedFixture<T>): {
  current: T;
  history: VersionDelta<T>[];
  lastUpdatedAt: string | null;
} {
  const { history, last_updated_at, ...rest } = fixture as T & {
    history?: VersionDelta<T>[];
    last_updated_at?: string;
  };
  return {
    current: rest as T,
    history: history ?? [],
    lastUpdatedAt: last_updated_at ?? null,
  };
}

function seedVersionedEntity<T extends object>(
  store: Store,
  collection: string,
  id: string,
  fixture: T & VersionedFixture<T>,
  finalize: (latest: T) => T,
): void {
  const { current, history, lastUpdatedAt } = splitVersioned<T>(fixture);

  if (history.length === 0) {
    const data = finalize(current);
    const timestamp = lastUpdatedAt ?? new Date().toISOString();
    store.seedVersion<T>(collection, id, data, timestamp);
    return;
  }

  // Apply forward-chronological partial diffs starting from history[0] (the
  // creation state). Each subsequent entry is merged onto the running state.
  let running = { ...history[0].patch } as T;
  store.seedVersion<T>(collection, id, finalize(running), history[0].timestamp);

  for (let i = 1; i < history.length; i++) {
    running = { ...running, ...history[i].patch } as T;
    store.seedVersion<T>(collection, id, finalize(running), history[i].timestamp);
  }

  // Final version: the fixture's main state. Timestamp defaults to one
  // minute after the last history entry if not explicitly provided.
  const lastHistoryTs = history[history.length - 1].timestamp;
  const finalTimestamp =
    lastUpdatedAt ?? new Date(new Date(lastHistoryTs).getTime() + 60_000).toISOString();
  store.seedVersion<T>(collection, id, finalize(current), finalTimestamp);
}

function normalizeIssue(issue: Issue): Issue {
  return {
    ...issue,
    todo_list: issue.todo_list ?? [],
    dependencies: issue.dependencies ?? [],
    patches: issue.patches ?? [],
  };
}

function normalizePatch(patch: Patch): Patch {
  return {
    ...patch,
    reviews: patch.reviews ?? [],
  };
}

export function loadSeedData(store: Store): void {
  store.clear();
  clearAssociations();
  clearConversationEvents();
  clearSessionEvents();
  clearSeededRelations();

  const seed = loadFixture();

  for (const [id, issue] of Object.entries(seed.issues)) {
    seedVersionedEntity<Issue>(store, "issues", id, issue, normalizeIssue);
  }

  for (const [id, task] of Object.entries(seed.sessions)) {
    seedVersionedEntity<Session>(store, "sessions", id, task, (s) => s);
  }

  for (const [id, patch] of Object.entries(seed.patches)) {
    seedVersionedEntity<Patch>(store, "patches", id, patch, normalizePatch);
  }

  for (const [id, doc] of Object.entries(seed.documents)) {
    store.create<Document>("documents", id, doc, "document");
  }

  for (const [id, repo] of Object.entries(seed.repositories)) {
    store.create<Repository>("repositories", id, repo, null);
  }

  for (const [id, agent] of Object.entries(seed.agents)) {
    store.create<AgentRecord>("agents", id, agent, null);
  }

  if (seed.labels) {
    for (const [id, label] of Object.entries(seed.labels)) {
      const normalized = {
        name: label.name,
        color: label.color,
        recurse: label.recurse ?? true,
        hidden: label.hidden ?? false,
      };
      store.create("labels", id, normalized, null);
    }
  }

  if (seed.label_associations) {
    for (const assoc of seed.label_associations) {
      addAssociation(assoc.label_id, assoc.object_id);
    }
  }

  if (seed.conversations) {
    for (const [id, conversation] of Object.entries(seed.conversations)) {
      store.create<Conversation>("conversations", id, conversation, "conversation");
    }
  }

  let conversationEventCount = 0;
  if (seed.conversation_events) {
    for (const [id, events] of Object.entries(seed.conversation_events)) {
      setConversationEvents(id, events);
      conversationEventCount += events.length;
    }
  }

  let sessionEventCount = 0;
  if (seed.session_events) {
    for (const [id, events] of Object.entries(seed.session_events)) {
      setSessionEvents(id, events);
      sessionEventCount += events.length;
    }
  }

  if (seed.relations) {
    for (const rel of seed.relations) {
      addSeededRelation(rel);
    }
  }

  const labelCount = seed.labels ? Object.keys(seed.labels).length : 0;
  const conversationCount = seed.conversations ? Object.keys(seed.conversations).length : 0;
  const relationCount = seed.relations ? seed.relations.length : 0;
  console.log(
    `Seed data loaded: ${Object.keys(seed.issues).length} issues, ` +
    `${Object.keys(seed.sessions).length} sessions, ` +
    `${Object.keys(seed.patches).length} patches, ` +
    `${Object.keys(seed.documents).length} documents, ` +
    `${Object.keys(seed.repositories).length} repositories, ` +
    `${Object.keys(seed.agents).length} agents, ` +
    `${labelCount} labels, ` +
    `${conversationCount} conversations, ` +
    `${conversationEventCount} conversation events, ` +
    `${sessionEventCount} session events, ` +
    `${relationCount} relations`,
  );
}
