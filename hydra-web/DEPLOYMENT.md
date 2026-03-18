# Deploying hydra-web to Kubernetes

This document describes the Kubernetes manifests needed to deploy hydra-web to the `dourolabs/metis-cluster` repository.

## Overview

hydra-web runs as a single container that serves both:
- The React SPA (static assets)
- The BFF (Backend-for-Frontend) API proxy that forwards authenticated requests to metis-server

The container listens on port **4000** and expects a `METIS_SERVER_URL` environment variable pointing to the metis-server instance.

## Required Manifests

Add the following files to the `dourolabs/metis-cluster` repository and include them in `kustomization.yaml`.

### web-deployment.yaml

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: metis-web
  namespace: metis
  labels:
    app: metis-web
spec:
  replicas: 1
  selector:
    matchLabels:
      app: metis-web
  template:
    metadata:
      labels:
        app: metis-web
    spec:
      containers:
        - name: metis-web
          image: ghcr.io/dourolabs/metis-web:latest
          ports:
            - containerPort: 4000
              protocol: TCP
          env:
            - name: METIS_SERVER_URL
              value: "http://server.metis.svc.cluster.local"
            - name: PORT
              value: "4000"
          resources:
            requests:
              cpu: 100m
              memory: 128Mi
            limits:
              cpu: 500m
              memory: 256Mi
          livenessProbe:
            httpGet:
              path: /health
              port: 4000
            initialDelaySeconds: 5
            periodSeconds: 10
          readinessProbe:
            httpGet:
              path: /health
              port: 4000
            initialDelaySeconds: 3
            periodSeconds: 5
      imagePullSecrets:
        - name: ghcr-credentials
```

### web-service.yaml

```yaml
apiVersion: v1
kind: Service
metadata:
  name: metis-web
  namespace: metis
  labels:
    app: metis-web
  annotations:
    tailscale.com/expose: "true"
    tailscale.com/hostname: metis-web
spec:
  type: LoadBalancer
  selector:
    app: metis-web
  ports:
    - port: 80
      targetPort: 4000
      protocol: TCP
```

### Update kustomization.yaml

Add the new manifests to the existing `kustomization.yaml`:

```yaml
resources:
  # ... existing resources ...
  - web-deployment.yaml
  - web-service.yaml
```

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `METIS_SERVER_URL` | `http://server.metis.svc.cluster.local` | URL of the metis-server API |
| `PORT` | `4000` | Port the BFF server listens on |
| `NODE_ENV` | `production` | Node.js environment (set in Dockerfile) |

## Container Details

- **Image**: `ghcr.io/dourolabs/metis-web`
- **Port**: 4000
- **Health check**: `GET /health` returns `{"status":"ok"}`
- **Base image**: `node:22-slim`

## Notes

- The `ghcr-credentials` imagePullSecret must already exist in the `metis` namespace (same as used by metis-server).
- The Tailscale annotations expose the service on the tailnet with hostname `metis-web`.
- `METIS_SERVER_URL` defaults to the in-cluster DNS name for metis-server. Adjust if the server runs in a different namespace or has a different service name.
