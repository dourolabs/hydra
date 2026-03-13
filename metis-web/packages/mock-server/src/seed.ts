import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import type { Store } from "./store.js";
import type { Issue, Task, Patch, Document, Repository, AgentRecord } from "@metis/api";
import { clearAssociations, addAssociation } from "./routes/labels.js";

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

interface SeedData {
  issues: Record<string, Issue>;
  sessions: Record<string, Task>;
  patches: Record<string, Patch>;
  documents: Record<string, Document>;
  repositories: Record<string, Repository>;
  agents: Record<string, AgentRecord>;
  labels?: Record<string, LabelData>;
  label_associations?: LabelAssociationSeed[];
}

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = resolve(__dirname, "../fixtures/seed.json");

function loadFixture(): SeedData {
  const raw = readFileSync(FIXTURE_PATH, "utf-8");
  return JSON.parse(raw) as SeedData;
}

export function loadSeedData(store: Store): void {
  store.clear();
  clearAssociations();

  const seed = loadFixture();

  for (const [id, issue] of Object.entries(seed.issues)) {
    store.create<Issue>("issues", id, issue, "issue");
  }

  for (const [id, task] of Object.entries(seed.sessions)) {
    store.create<Task>("sessions", id, task, "job");
  }

  for (const [id, patch] of Object.entries(seed.patches)) {
    store.create<Patch>("patches", id, patch, "patch");
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

  const labelCount = seed.labels ? Object.keys(seed.labels).length : 0;
  console.log(
    `Seed data loaded: ${Object.keys(seed.issues).length} issues, ` +
    `${Object.keys(seed.sessions).length} sessions, ` +
    `${Object.keys(seed.patches).length} patches, ` +
    `${Object.keys(seed.documents).length} documents, ` +
    `${Object.keys(seed.repositories).length} repositories, ` +
    `${Object.keys(seed.agents).length} agents, ` +
    `${labelCount} labels`,
  );
}
