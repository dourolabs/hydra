# hydra-s3

Minimal S3-compatible service (subset used by `hydra-build-cache`) backed by the local filesystem.

## Local development

1. Copy the sample config and adjust if needed:

   ```bash
   cp hydra-s3/config.toml.sample hydra-s3/config.toml
   ```

2. Run the service with the config on your machine:

   ```bash
   HYDRA_CONFIG=hydra-s3/config.toml cargo run -p hydra-s3
   ```

The service listens on `0.0.0.0:9090` by default and stores objects under `/var/lib/hydra/s3`.

## Docker

Build and run the image locally:

```bash
docker build -t hydra-s3:latest -f images/hydra-s3.Dockerfile .

docker run --rm -p 9090:9090 \
  -e HYDRA_CONFIG=/etc/hydra-s3/config.toml \
  -v "$(pwd)/hydra-s3/config.toml.sample:/etc/hydra-s3/config.toml:ro" \
  -v "$(pwd)/.hydra-s3-data:/var/lib/hydra/s3" \
  hydra-s3:latest
```

## In-cluster (local Kubernetes)

The `scripts/service.sh` helper now provisions `hydra-s3` alongside the server and Postgres.
For kind-based local clusters:

```bash
./scripts/docker-build.sh
./scripts/service.sh start
```

By default this creates a `hydra-s3` ClusterIP service on port `9090` and backs storage with an
`emptyDir` volume at `/var/lib/hydra/s3`.

You can override defaults with environment variables:

- `S3_IMAGE` (default `hydra-s3:latest`)
- `S3_SERVICE_NAME` (default `hydra-s3`)
- `S3_SERVICE_PORT` (default `9090`)
- `S3_STORAGE_ROOT` (default `/var/lib/hydra/s3`)
- `S3_CONFIGMAP_NAME`, `S3_CONFIG_MOUNT_PATH`, `S3_CONFIG_FILE_NAME`, `S3_HYDRA_CONFIG_PATH`

Example override:

```bash
S3_STORAGE_ROOT=/data/hydra-s3 S3_SERVICE_PORT=9091 ./scripts/service.sh start
```
