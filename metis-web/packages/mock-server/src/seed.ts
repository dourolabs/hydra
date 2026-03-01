import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import type { Store } from "./store.js";
import type { Issue, Task, Patch, Document, Repository, AgentRecord, Notification } from "@metis/api";

interface SeedData {
  issues: Record<string, Issue>;
  jobs: Record<string, Task>;
  patches: Record<string, Patch>;
  documents: Record<string, Document>;
  repositories: Record<string, Repository>;
  agents: Record<string, AgentRecord>;
  notifications: Record<string, Notification>;
}

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = resolve(__dirname, "../fixtures/seed.json");

function loadFixture(): SeedData {
  const raw = readFileSync(FIXTURE_PATH, "utf-8");
  return JSON.parse(raw) as SeedData;
}

export function loadSeedData(store: Store): void {
  store.clear();

  const seed = loadFixture();

  for (const [id, issue] of Object.entries(seed.issues)) {
    store.create<Issue>("issues", id, issue, "issue");
  }

  for (const [id, task] of Object.entries(seed.jobs)) {
    store.create<Task>("jobs", id, task, "job");
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

  for (const [id, notification] of Object.entries(seed.notifications)) {
    store.create<Notification>("notifications", id, notification, null);
  }

  console.log(
    `Seed data loaded: ${Object.keys(seed.issues).length} issues, ` +
    `${Object.keys(seed.jobs).length} jobs, ` +
    `${Object.keys(seed.patches).length} patches, ` +
    `${Object.keys(seed.documents).length} documents, ` +
    `${Object.keys(seed.repositories).length} repositories, ` +
    `${Object.keys(seed.agents).length} agents, ` +
    `${Object.keys(seed.notifications).length} notifications`,
  );
}
