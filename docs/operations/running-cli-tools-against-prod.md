# Running CLI tools against the production database

This doc covers the options for running one-off CLI tools — data backfills, ad-hoc
rewrites, debugging queries, migrations that are not yet folded into server
startup — against the **production** Hydra Postgres database, and picks a default.

## When this doc applies

Only when the work cannot live anywhere better:

- A schema change that fits `sqlx`'s `migrations/` folder belongs there. It runs
  on server boot and is the boring path.
- A data migration that can run idempotently on startup belongs in
  `hydra-server::run` (see the sibling PR that moves `hydra-migrate-sessions`
  there). Prefer that over a CLI any time the rewrite is cheap, deterministic,
  and safe to re-run.
- An ad-hoc one-shot data fix, a backfill that is too large for startup, or
  exploratory debugging that needs the prod DB — that is what this doc is for.

If you find yourself reaching for this doc more than a couple of times, that is
a signal to build the operation into the server (admin route, automation, or
startup) instead of normalizing prod-CLI access.

## Production environment, in one paragraph

Hydra runs on the single-node Talos cluster in `dourolabs/metis-cluster` (see
`/repos/dourolabs-hydra-cluster.md` and `/playbooks/deploy-hydra.md` in the
doc store). All workload manifests are reconciled by Flux CD from that repo;
secrets come from 1Password via the 1Password Operator. Cluster access is
gated behind Tailscale — engineers reach the API server through the VPN with
their own kubeconfig. Postgres runs as a pod in the `metis` namespace with a
PVC-backed volume; its `DATABASE_URL` lives in a Kubernetes Secret that the
`hydra-server` Deployment already mounts.

Two practical constraints fall out of that shape:

1. The published `ghcr.io/dourolabs/hydra-server` image is built with
   `cargo build --bin hydra-server` (see `images/hydra-server.Dockerfile`). It
   does **not** contain the workspace's other binaries (`hydra-migrate-sessions`,
   `seed-migration-fixture`, the `hydra` CLI). Any option that "uses the same
   image" only works for tools that are also baked into that image.
2. The cluster is GitOps-managed. Imperative `kubectl apply` works for one-off
   resources, but anything left around will not be reconciled — and anything
   that lives in `metis-cluster` will be reconciled forever.

## Options

### 1. One-shot Kubernetes Job

Apply a `batch/v1 Job` manifest imperatively (`kubectl create -f job.yaml`),
re-using the `hydra-server` image, the `metis` namespace, and the existing
Secret that provides `DATABASE_URL`. Override the entrypoint to invoke the CLI
binary. Watch with `kubectl logs -f job/<name>`, then `kubectl delete job/<name>`
once it succeeds.

Sketch:

```yaml
apiVersion: batch/v1
kind: Job
metadata: { name: backfill-foo, namespace: metis }
spec:
  backoffLimit: 0
  template:
    spec:
      restartPolicy: Never
      serviceAccountName: hydra-server
      containers:
        - name: cli
          image: ghcr.io/dourolabs/hydra-server:vX.Y.Z
          command: ["hydra-migrate-sessions"]
          args: ["--database-url", "$(DATABASE_URL)"]
          envFrom: [{ secretRef: { name: hydra-server-env } }]
```

Caveat: the published image must contain the binary you want. If it does not,
either add it to `images/hydra-server.Dockerfile` (preferred — small layer cost,
makes future Jobs trivial) or build a bespoke image first.

### 2. `kubectl exec` into a running hydra-server pod

`kubectl -n metis exec -it deploy/hydra-server -- hydra-migrate-sessions ...`.
No new pod, no new manifest, `DATABASE_URL` is already in the pod's env. The
fastest path for tools that are baked into the image.

Trade-off: the CLI runs in a serving pod and shares its resource quota; an
expensive backfill can starve real traffic. Don't use this for anything you
wouldn't be comfortable running in a request handler.

### 3. Port-forward + run from a local checkout

`kubectl -n metis port-forward svc/postgres 5432:5432`, then
`DATABASE_URL=postgres://... cargo run -p hydra-server --bin <tool> -- ...`
from the engineer's laptop. No image build, the CLI is whatever is on the
working branch, and iteration is fast.

Trade-offs: requires the prod DB credentials to be visible on the developer's
machine (read them out of the Kubernetes Secret with `kubectl get secret`,
which is the moment they leave the cluster boundary); the connection runs over
the Tailscale tunnel, so throughput is bounded; there is no record of what
actually ran beyond shell history. Genuinely useful for read-only debugging
and small one-offs, not for anything stateful you want a paper trail on.

### 4. Long-lived bastion / debug pod

A permanent pod in `metis` with a shell, the source tree (or pre-built CLIs),
and the prod Secret mounted. Engineers `kubectl exec` into it for arbitrary
work.

Trade-offs: ergonomics are excellent, but it is a permanent attack surface
— a compromise of the bastion is a compromise of the DB credentials — and it
needs a manifest in `metis-cluster` and matching RBAC. Worth it only if we
were running many ad-hoc tools per week, which we are not.

### 5. GitHub Action with cluster credentials

A workflow in `dourolabs/hydra` that builds the CLI, authenticates to the
cluster using a kubeconfig stored as a GitHub secret (or runs the same
one-shot-Job flow as option 1 via `kubectl`), and is invoked with
`workflow_dispatch`.

Trade-offs: every invocation is logged and tied to a GitHub identity — the
best auditability of any option. But the cluster is Tailscale-only, so the
runner needs to join the tailnet (Tailscale GitHub Action or a self-hosted
runner inside the VPN); both add real setup work. Latency is also higher
because each run is a fresh CI job.

## Scoring

| Option | Safety | Auditability | Ergonomics | Cred exposure | Infra cost |
|---|---|---|---|---|---|
| 1. One-shot Job | High — isolated pod, deleted after | Medium — `kubectl events` + logs only while the Job exists; capture manually | Medium — write a tiny manifest each time | Low — never leaves the cluster | Low (one-time: add bin to image) |
| 2. `kubectl exec` | Low — runs in a serving pod | Low — only shell history | High | Low | None |
| 3. Port-forward + local run | Medium — runs on a laptop, easy to Ctrl-C | Low — shell history only | High | **High** — secret on the laptop | None |
| 4. Bastion pod | Medium — depends on RBAC | Medium — pod history | High | Medium — credentials live in the cluster permanently | High (new manifest, ongoing) |
| 5. GitHub Action | High — runs in CI, gated by review | **High** — workflow run log per invocation | Low — must dispatch a workflow each time | Low — secrets only in CI | Medium (workflow + VPN egress) |

Out of scope, mentioned only to close the option: exposing CLI commands as
authenticated HTTP admin routes is a fine direction long-term, but it is a
much bigger change than a runbook entry and is tracked separately if at all.

## Recommendation

**Default to option 1: a one-shot Kubernetes Job, applied imperatively with
`kubectl create -f`, using the `hydra-server` image and the existing
`hydra-server` ServiceAccount and Secret, deleted after it succeeds.**

It fits the cluster as it actually exists: credentials never leave the
cluster boundary, the workload runs with the same RBAC and network identity
as the server it is mutating, and there is nothing permanent left behind for
Flux to reconcile. The main friction is that the published image currently
contains only `hydra-server`; the fix is to add any genuinely prod-bound CLI
to `images/hydra-server.Dockerfile` so the same Job pattern works for the
next tool too — a one-line change with a small layer-cache cost. When the
tool is not in the image and adding it is not worth the build, fall back to
**option 3** (port-forward + local `cargo run`) for read-mostly debugging,
treating the temporarily-exported `DATABASE_URL` as a credential to be
revoked-by-rotation if it leaks.

We deliberately don't default to `kubectl exec` (it competes with serving
traffic), to a bastion (permanent attack surface for a use case we expect to
be rare), or to a GitHub Action (real setup cost for VPN access, and the
audit win is not yet worth it). If our cadence of prod-CLI work changes
materially, option 5 is the right next step.
