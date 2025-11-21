docker build -t metis-worker:latest -f ./images/metis-worker.Dockerfile .
docker build -t metis-server:latest -f ./images/metis-server.Dockerfile .

kind load docker-image metis-worker:latest --name local-dev
kind load docker-image metis-server:latest --name local-dev
