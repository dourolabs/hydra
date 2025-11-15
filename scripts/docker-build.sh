docker build -t metis-codex:latest -f ./images/codex.Dockerfile .
docker build -t metis-server:latest -f ./images/metis-server.Dockerfile .

kind load docker-image metis-codex:latest --name local-dev
kind load docker-image metis-server:latest --name local-dev
