# metis-s3

Minimal S3-compatible service (subset used by `metis-build-cache`) backed by the local filesystem.

## Local development

1. Copy the sample config and adjust if needed:

   ```bash
   cp metis-s3/config.toml.sample metis-s3/config.toml
   ```

2. Run the service with the config on your machine:

   ```bash
   METIS_CONFIG=metis-s3/config.toml cargo run -p metis-s3
   ```

The service listens on `0.0.0.0:9090` by default and stores objects under `/var/lib/metis/s3`.
Cache archives approaching or exceeding `1 GiB` should increase `request_body_limit_bytes`.
Set `S3_REQUEST_BODY_LIMIT_BYTES` (defaults to `1 GiB` in `scripts/service.sh`) or
update your config file accordingly.

## Docker

Build and run the image locally:

```bash
docker build -t metis-s3:latest -f images/metis-s3.Dockerfile .

docker run --rm -p 9090:9090 \
  -e METIS_CONFIG=/etc/metis-s3/config.toml \
  -v "$(pwd)/metis-s3/config.toml.sample:/etc/metis-s3/config.toml:ro" \
  -v "$(pwd)/.metis-s3-data:/var/lib/metis/s3" \
  metis-s3:latest
```

## In-cluster (local Kubernetes)

The `scripts/service.sh` helper now provisions `metis-s3` alongside the server and Postgres.
For kind-based local clusters:

```bash
./scripts/docker-build.sh
./scripts/service.sh start
```

By default this creates a `metis-s3` ClusterIP service on port `9090` and backs storage with an
`emptyDir` volume at `/var/lib/metis/s3`.

You can override defaults with environment variables:

- `S3_IMAGE` (default `metis-s3:latest`)
- `S3_SERVICE_NAME` (default `metis-s3`)
- `S3_SERVICE_PORT` (default `9090`)
- `S3_STORAGE_ROOT` (default `/var/lib/metis/s3`)
- `S3_CONFIGMAP_NAME`, `S3_CONFIG_MOUNT_PATH`, `S3_CONFIG_FILE_NAME`, `S3_METIS_CONFIG_PATH`

Example override:

```bash
S3_STORAGE_ROOT=/data/metis-s3 S3_SERVICE_PORT=9091 ./scripts/service.sh start
```
