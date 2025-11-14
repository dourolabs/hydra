docker build -t metis-codex:latest -f ./images/Dockerfile .

kind load docker-image metis-codex:latest --name local-dev
