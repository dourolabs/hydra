# `metis caches`

The `metis caches` command family packages build artifacts into portable archives so repeated CI jobs (or local dev shells) can skip re-building dependencies. Each cache entry is keyed by `org/repo` plus a Git SHA, and storage can live on a shared filesystem or any S3-compatible bucket.

## Authentication & output

All subcommands inherit global `metis` flags such as `--server-url`, `--token`, and `--output-format` (pretty/jsonl). Pretty output summarizes uploads and cache hits; switch to `--output-format jsonl` to capture structured rows for automation or logging.

## Storage & prerequisites

* Repositories must pass `--repo-name org/repo`; the Git SHA comes from `HEAD` unless overridden via `--git-sha`.
* Exactly one storage backend is required:
  * Filesystem: `--storage-root /mnt/cache`.
  * S3-compatible: supply `--s3-endpoint-url`, `--s3-bucket`, `--s3-region`, and (optionally) access key, secret, and session token flags.
* For shared language toolchains, include `--home-dir ~/.cache/rust` so archives ship reproducible dependency trees.
* Ensure the calling environment can reach the chosen storage target (NFS mount present, S3 credentials configured, etc.).

## Subcommands

### Build

```bash
metis caches build \
  --repo-name org/service \
  --storage-root /mnt/cache \
  [--root .] \
  [--git-sha $(git rev-parse main)] \
  [--dry-run] \
  [--home-dir ~/.cache/rust]
```

* Packages the workspace at `--root` plus any `--home-dir` overlay into a tarball and uploads it under `<repo>/<sha>.tar.zst`.
* `--git-sha` defaults to `HEAD`; use it to publish cache keys for pre-merge commits or release branches.
* `--dry-run` lists the paths that would be archived without creating or uploading a file—helpful when tuning ignore patterns.
* Supply either `--storage-root` or the S3 flags, not both. Missing storage configuration fails fast before expensive archive work begins.

### List

```bash
metis caches list \
  --repo-name org/service \
  --s3-endpoint-url https://s3.example.com \
  --s3-bucket builds \
  --s3-region us-west-2
```

Returns all cache keys for the repo with their last-modified timestamps. Use this to confirm retention policies or identify stale SHA coverage.

### Apply

```bash
metis caches apply \
  --repo-name org/service \
  --storage-root /mnt/cache \
  [--root .] \
  [--git-sha 1a2b3c4d | --nearest] \
  [--home-dir ~/.cache/rust]
```

* `--git-sha` applies the exact cache archive uploaded under that SHA.
* `--nearest` inspects local Git history and chooses the closest available cache entry, useful for topic branches rebased onto main.
* `--home-dir` restores shared toolchains (for example `~/.cache/pip`) into the caller’s home directory alongside the repository root.
* Output shows whether a cache was applied and, for nearest matches, the number of commits between HEAD and the cache key.

## Troubleshooting

* **“No cache entry found to apply.”** Ensure the requested SHA exists (check `metis caches list`) or re-run build with a known SHA before applying.
* **“failed to configure filesystem/S3 storage.”** Filesystem mode requires an existing `--storage-root`; S3 mode needs non-empty endpoint URL, bucket, and region plus valid credentials. Never set filesystem and S3 flags simultaneously.
* **Auth failures when talking to S3.** Break-glass with temporary tokens via `--s3-session-token` or source credentials via environment variables; the CLI simply passes them through to the lib.

## Examples

```bash
# Build a cache for the current commit and upload to local storage
metis caches build --repo-name org/service --storage-root /mnt/cache

# Preview which paths would be archived (no upload)
metis caches build --repo-name org/service --storage-root /mnt/cache --dry-run

# List caches stored in S3 for audit logging
metis caches list \
  --repo-name org/service \
  --s3-endpoint-url https://s3.us-west-2.amazonaws.com \
  --s3-bucket service-caches \
  --s3-region us-west-2

# Apply the nearest cache for feature branches while restoring shared toolchains
metis caches apply \
  --repo-name org/service \
  --storage-root /mnt/cache \
  --nearest \
  --home-dir ~/.cache/rust
```
