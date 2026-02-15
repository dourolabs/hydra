# dourolabs/metis-cluster

Kubernetes cluster infrastructure-as-code for the Metis platform. Manages a Talos-based single-node Kubernetes cluster deployed via Flux CD (GitOps). The cluster hosts the Metis AI agent orchestration system along with supporting infrastructure services.

## Repository Structure

```
metis-cluster/
├── .github/workflows/       # CI: auto-approve PRs from maintainer
├── clusters/metis-cluster/
│   ├── README.md             # Cluster setup guide (hardware, bootstrap, networking)
│   ├── cluster.yaml          # Talos machine configuration
│   ├── secrets.yaml          # Encrypted Talos secrets (git-crypt)
│   ├── talosconfig           # Encrypted Talos admin config (git-crypt)
│   ├── kubeconfig            # Kubernetes API access config
│   └── workloads/            # All Kubernetes manifests (17 workload directories)
├── patches/
│   └── watchdog.yaml         # Talos watchdog timer patch
├── scripts/
│   └── install_cilium.sh     # Cilium CNI Helm installation script
└── README.md                 # Symlink to clusters/metis-cluster/README.md
```

## Cluster Overview

- **OS:** Talos Linux v1.12.1 (immutable Kubernetes-optimized OS)
- **Nodes:** Single node (`metis-cluster-1`, IP 64.130.37.115)
- **Access:** Tailscale VPN only (no direct internet exposure)
- **CNI:** Cilium (eBPF-based, replaces kube-proxy)
- **GitOps:** Flux CD v2 reconciles all workloads from this repository
- **Secrets:** 1Password Operator + git-crypt for repository-level encryption

### Storage

| Disk | Device | Size | Purpose |
|------|--------|------|---------|
| Dell BOSS-N1 SSD | nvme2n1 | 960GB | Talos ephemeral storage |
| Dell NVMe PE1030 | nvme1n1 | 3.2TB | local-path-provisioner (dynamic PVCs) |
| Dell NVMe PE1030 | nvme0n1 | 3.2TB | User volume (hostPath mounts) |

Default StorageClass: `local-path` via local-path-provisioner on nvme1n1.

## Key Workloads and Services

### Metis Application (namespace: `metis`)

The primary workload. Runs the Metis AI agent orchestration platform.

- **metis-server** (`ghcr.io/dourolabs/metis-server`) - API server. Config templated from ConfigMap (config.toml). Port 8080.
- **metis-s3** (`ghcr.io/dourolabs/metis-s3`) - S3-compatible object storage. Port 9090. EmptyDir-backed (100Gi limit).
- **postgres** (`postgres:16-alpine`) - PostgreSQL 16 database. PVC-backed persistent storage.
- Secrets injected via 1Password: GitHub App credentials, OpenAI API key, Claude Code OAuth token.
- Background agents configured: "swe" (developer) and "pm" (product manager).

### Infrastructure Services

| Service | Namespace | Type | Purpose |
|---------|-----------|------|---------|
| Flux CD | flux-system | GitOps | Continuous deployment from GitHub |
| Cilium | kube-system | CNI | eBPF networking, Gateway API, load balancing |
| Tailscale Operator | tailscale-operator | Helm (v1.92.5) | VPN access integration |
| 1Password Operator | onepassword-operator | Helm (v2.2.1) | Secrets management from 1Password vaults |
| cert-manager | cert-manager | Helm | TLS certificate lifecycle |
| RBAC Manager | rbac-manager | Helm (v1.21.5) | Declarative RBAC via RBACDefinition CRD |

### Monitoring

| Service | Namespace | Type | Purpose |
|---------|-----------|------|---------|
| Alloy Node | alloy-node | DaemonSet (Helm v1.5.3) | Node-level metrics collection (Grafana) |
| Alloy Pods | alloy-pods | Deployment (Helm v1.5.3) | Pod-level metrics collection (Grafana) |
| Metrics Server | kube-system | Helm (v3.13.0) | Resource metrics (HPA, kubectl top) |
| kubelet-serving-cert-approver | kube-system | Flux GitRepository | Auto-approves kubelet certs (required by metrics-server on Talos) |

### Storage

| Service | Namespace | Purpose |
|---------|-----------|---------|
| Local Path Provisioner | local-path-storage | Dynamic PVC provisioning on local NVMe |

### Test Workloads

Four test deployments validate storage patterns:

- **emptydir** - EmptyDir volume testing (2 replicas)
- **pvc-deployment** - PersistentVolumeClaim with Deployment (1 replica)
- **pvc-statefulset** - StatefulSet with volumeClaimTemplate (2 replicas)
- **ephemeral-statefulset** - Ephemeral volume testing (2 replicas)

## Configuration Patterns

### GitOps with Flux CD

All workloads are deployed via Flux CD. The root Kustomization at `clusters/metis-cluster/workloads/kustomization.yaml` references 17 workload directories. Each workload has its own `kustomization.yaml` and namespace declaration.

Helm charts are managed through Flux `HelmRepository` + `HelmRelease` CRDs. Eight external Helm repositories are configured (Jetstack, Grafana, Fairwinds, Tailscale, 1Password, Kubernetes SIGs, Cilium).

### Secrets Management

- **1Password Operator:** Primary method. Uses `OnePasswordItem` CRD to sync secrets from 1Password vaults into Kubernetes secrets.
- **git-crypt:** AES-256 encryption for sensitive files committed to the repository (Talos secrets, talosconfig). Decryption key stored in 1Password.

### RBAC

Managed declaratively via RBAC Manager's `RBACDefinition` CRD:

- **team-platform:** Full cluster admin access
- **team-eng:** Granular access to application namespaces, Flux resources, networking policies, and Argo Workflows

### Security

- Non-root containers enforced across workloads
- Capability dropping on all pods
- RuntimeDefault seccomp profiles
- Resource quotas per namespace
- VPN-only cluster access

## CI/CD

### GitHub Actions

- **auto-approve.yaml:** Auto-approves PRs from the maintainer (`jayantk`) to streamline GitOps deployments.

### Deployment Flow

1. Changes committed and pushed to GitHub
2. Flux GitRepository detects updates
3. Flux Kustomization reconciles workload manifests
4. HelmReleases updated by Helm controller
5. Workloads apply new configuration

## Key Files

- `clusters/metis-cluster/cluster.yaml` - Talos machine config (network bonding, disk layout, kernel modules, extensions)
- `clusters/metis-cluster/workloads/kustomization.yaml` - Root manifest listing all workloads
- `clusters/metis-cluster/workloads/metis/` - Core Metis application manifests
- `scripts/install_cilium.sh` - Cilium installation with full Helm values
- `patches/watchdog.yaml` - Hardware watchdog configuration
