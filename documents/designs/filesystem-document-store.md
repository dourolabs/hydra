# Design: Filesystem-Accessible Document Store

## Problem Statement

The Metis document store (`metis documents`) provides a versioned, path-organized store for markdown documents and arbitrary assets. However, documents are only accessible via CLI commands (`metis documents list/get/create/update`) or the REST API. This creates friction:

1. **Agents can't explore documents naturally.** Tools like `ls`, `cat`, `grep`, `find`, and file-reading capabilities built into LLM agents cannot be used. Agents must know about and call `metis documents` explicitly.
2. **Editing is cumbersome.** Modifying a document requires fetching it, editing locally, then uploading via `metis documents update`. There's no way to edit in-place with standard editors or tools.
3. **No integration with standard tooling.** Code search, IDE features, and scripts that operate on files cannot work with documents.

## Goals

- **Primary (Read/Explore):** Make documents browsable and readable as a local filesystem directory, so agents and users can `ls`, `cat`, `grep`, and use standard file tools.
- **Secondary (Write):** Allow modifications to documents to be synced back, even if via an explicit command.

## Non-Goals

- Real-time collaborative editing with conflict resolution.
- Mounting the entire version history as files.
- Replacing the `metis documents` CLI — it remains the canonical interface for creating documents and managing metadata (title, path changes, etc.).

## Approaches Considered

### Option A: FUSE Virtual Filesystem

Mount the document store as a FUSE filesystem. Reads are proxied to the server API on-demand; writes are buffered and pushed on `close()`.

**Pros:**
- Transparent integration — all file tools work immediately.
- No local disk usage for unaccessed documents.
- Always sees the latest server state.

**Cons:**
- FUSE requires the `fuse` kernel module and `SYS_ADMIN` or `CAP_SYS_ADMIN` capability in containers. Worker pods currently have no security context, and adding privileged capabilities is a significant security concern in a multi-tenant Kubernetes cluster.
- FUSE adds ~4 context switches per file operation and all reads go through the network, adding latency. For agents doing repeated `grep`/`find` operations, this compounds.
- Cross-platform support is poor (macOS requires macFUSE/osxfuse; Windows requires WinFsp).
- Added runtime dependency: a FUSE daemon must run alongside the worker process.
- Complexity: implementing a correct FUSE filesystem (inode management, caching, write-back) is non-trivial.

### Option B: CLI Sync (Download-to-Directory) — **Recommended**

Add a `metis documents sync` command that downloads all documents (or a subset by path prefix) into a local directory, using their `path` field to create the directory structure. An optional `metis documents push` command uploads local changes back.

**Pros:**
- Simple to implement — uses existing API and standard filesystem operations.
- No kernel module, no special container capabilities required.
- Works on all platforms (Linux, macOS, Windows).
- Once synced, reads are at native filesystem speed with zero latency.
- Agents can freely explore, search, and edit the local copy.
- Fits naturally into the existing worker-run lifecycle: sync at job start, push at job end.

**Cons:**
- Requires explicit sync invocation (not automatic).
- Local copy may become stale if documents change on the server during a long-running job (acceptable — agent jobs are relatively short-lived and operate on a snapshot).
- Uses local disk space proportional to the synced document set (acceptable — document store is small, mostly markdown).

### Option C: Hybrid (Sync + Watch)

Like Option B, but add a background watcher that periodically re-syncs from the server. This is more complex without clear benefit given the agent use case (jobs are short-lived, snapshot semantics are fine).

## Recommendation: Option B — CLI Sync

The sync-based approach is the right fit for this system because:

1. **Kubernetes compatibility:** No FUSE kernel module or elevated capabilities needed. Works in any container.
2. **Agent workflow fit:** Agents already clone git repos at job start. Adding a document sync step is the same pattern — fetch a snapshot, work on it locally, push results.
3. **Simplicity:** The implementation is straightforward and leverages existing API endpoints.
4. **Performance:** After sync, all reads are at native filesystem speed — critical for agents that extensively search/read documents.

## Detailed Design

### New CLI Commands

#### `metis documents sync [OPTIONS] <directory>`

Downloads documents from the server into a local directory.

```
Arguments:
  <directory>          Target directory to sync documents into

Options:
  --path-prefix <PATH>   Only sync documents under this path prefix
  --clean                Remove local files not present on server (default: false)
  --manifest <FILE>      Path to manifest file for tracking sync state
                         (default: <directory>/.metis-documents.json)
```

**Behavior:**
1. Call `list_documents()` API with optional `path_prefix` filter.
2. For each document with a non-null `path`:
   - Create parent directories as needed.
   - Write `body_markdown` to `<directory>/<document.path>`.
3. For documents without a `path`, skip them (or place in a flat `_unpathed/` directory using document ID as filename — TBD based on whether unpathed documents exist in practice).
4. Write a manifest file (`.metis-documents.json`) mapping local file paths to document IDs, versions, and content hashes. This enables incremental sync and push.
5. If `--clean` is set, remove any local files in the directory that are not in the server's document set.

**Manifest format:**
```json
{
  "synced_at": "2026-02-11T00:00:00Z",
  "server_url": "http://metis-server:8080",
  "path_prefix": "/playbooks",
  "documents": {
    "playbooks/add-new-repo.md": {
      "document_id": "d-acjndk",
      "version": 3,
      "content_hash": "sha256:abc123..."
    }
  }
}
```

#### `metis documents push [OPTIONS] <directory>`

Uploads local changes back to the server.

```
Arguments:
  <directory>          Directory previously synced with `metis documents sync`

Options:
  --dry-run            Show what would be uploaded without making changes
  --path-prefix <PATH>  Only push documents under this path prefix
```

**Behavior:**
1. Read the manifest file from `<directory>/.metis-documents.json`.
2. For each file in the directory:
   - Compute content hash and compare to manifest.
   - If changed: call `update_document()` API with new body.
   - If new (not in manifest): call `create_document()` API with path derived from relative file path, and a title derived from the filename.
3. Optionally, for files in manifest but missing locally: call `delete_document()` API (behind a `--delete-removed` flag, off by default).
4. Update the manifest with new versions and hashes.

### Integration with Worker Lifecycle

**File:** `metis/src/command/jobs/worker_run.rs`

Add document sync as a step in the worker-run lifecycle, after git clone and before worker execution:

```
1. Clone git repository (existing)
2. Apply build cache (existing)
3. NEW: Sync documents to a well-known directory (e.g., /tmp/metis-documents or a subdirectory of the working directory)
4. Set METIS_DOCUMENTS_DIR environment variable pointing to the synced directory
5. Execute worker (existing)
6. NEW: Push document changes back to server
7. Commit & upload build cache (existing)
```

The `METIS_DOCUMENTS_DIR` environment variable allows agent prompts to reference the documents directory. The system prompt can include instructions like "Documents are available at $METIS_DOCUMENTS_DIR".

### Key Files to Modify

| File/Directory | Change |
|---|---|
| `metis/src/command/documents.rs` | Add `Sync` and `Push` subcommands with argument structs |
| `metis/src/command/documents/sync.rs` (new) | Implement sync logic: list documents, write files, write manifest |
| `metis/src/command/documents/push.rs` (new) | Implement push logic: diff manifest, upload changes |
| `metis/src/command/jobs/worker_run.rs` | Add document sync/push steps to worker lifecycle |
| `metis-common/src/constants.rs` | Add `METIS_DOCUMENTS_DIR` constant |
| `metis/src/client/mod.rs` | No changes needed — existing API methods suffice |

### Edge Cases

1. **Documents without paths:** Documents created without a `path` field cannot be mapped to a filesystem path. Options:
   - Skip them during sync (recommended — they're accessible via `metis documents get <id>`).
   - Place them in a `_by_id/<document-id>.md` subdirectory.

2. **Path collisions:** Two documents with the same `path` but different IDs. The server currently allows this. During sync, the latest document wins and a warning is printed.

3. **Large documents / binary assets:** The document store accepts arbitrary markdown content. For very large documents, sync should stream content rather than buffer entirely in memory. In practice, documents are small markdown files and this is unlikely to be an issue.

4. **Concurrent modifications:** If the server document changes while the agent is working, the local copy becomes stale. This is acceptable — the agent works on a snapshot, just like git. The push step should check versions and warn on conflicts (version mismatch).

5. **Manifest missing or corrupt:** If the manifest is missing, `push` should refuse to operate (to avoid accidentally overwriting server documents). `sync` should create a fresh manifest.

### Testing Strategy

- **Unit tests:** Test manifest serialization/deserialization, content hash computation, path derivation from document paths.
- **Integration tests:** Test sync/push round-trip against in-memory server store.
- **CLI tests:** Verify command-line argument parsing and error messages.

## Implementation Plan

### Task 1: `metis documents sync` command
- Implement the `sync` subcommand and manifest format.
- Write documents to local directory with correct path structure.
- Include manifest tracking (document ID, version, content hash).
- Tests: unit tests for manifest, integration test for sync against in-memory store.

### Task 2: `metis documents push` command  
- Implement the `push` subcommand.
- Detect local changes via content hash comparison.
- Upload changed documents, create new documents.
- Version conflict detection (warn if server version has advanced).
- Tests: integration test for push round-trip.

### Task 3: Integrate sync/push into worker-run lifecycle
- Add sync step after git clone in `worker_run.rs`.
- Add push step before finalization.
- Set `METIS_DOCUMENTS_DIR` environment variable.
- Tests: end-to-end test of worker lifecycle with document sync.

## Open Questions

1. Should documents without a `path` be synced? (Recommendation: no — skip them.)
2. Should the sync directory be inside the git working directory or separate? (Recommendation: separate, to avoid accidentally committing documents into the repo.)
3. Should `push` support creating new documents from new local files, or only updating existing ones? (Recommendation: support both, with path and title derived from filename.)