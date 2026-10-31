#!/usr/bin/env bash
# Build the deploy images on the Oracle server instead of GitHub Actions, and optionally deploy them. Run from a
# checkout on your computer; it sends the committed HEAD, so uncommitted changes are not included.
#   deploy/oracle/build-on-server.sh [both|frontend|backend] [--deploy]
# The builder is limited to 1.5 CPUs and 8 GB so the live site keeps running, and keeps its cache between builds.
# Images are tagged with the commit: to roll back, set OC_RUNTIME_IMAGE/OC_GATEWAY_IMAGE in .env to an older tag and
# run ./update.sh. The last 3 of each are kept.
set -euo pipefail
target=${1:-both}
deploy=${2:-}
host=${OC_SERVER:?set OC_SERVER to your server, for example ubuntu@203.0.113.10}
case "$target" in both|frontend|backend) ;; *) echo "usage: $0 [both|frontend|backend] [--deploy]" >&2; exit 1 ;; esac
cd "$(git rev-parse --show-toplevel)"
sha=$(git rev-parse --short HEAD)

git archive --format=tar HEAD | ssh "$host" 'rm -rf ~/opencanvas/src && mkdir -p ~/opencanvas/src && tar -x -C ~/opencanvas/src'

# The build runs detached on the server, so a dropped connection doesn't stop it.
ssh "$host" "cat > ~/opencanvas/build-job.sh" <<REMOTE
#!/usr/bin/env bash
set -euo pipefail
cd ~/opencanvas/src
if ! swapon --show --noheadings | grep -q .; then
  sudo fallocate -l 4G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile >/dev/null && sudo swapon /swapfile
  grep -q '^/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab >/dev/null
fi
docker buildx inspect pinselo >/dev/null 2>&1 || docker buildx create --name pinselo --driver docker-container \
  --driver-opt cpu-quota=150000,cpu-period=100000,memory=8g,memory-swap=12g >/dev/null
build() { docker buildx build --builder pinselo --progress=plain --load -f deploy/Dockerfile --target "\$1" -t "\$2:$sha" .; }
if [ "$target" != frontend ]; then build runtime pinselo-runtime; fi
if [ "$target" != backend ]; then build gateway pinselo-gateway; fi
keep() { docker images "\$1" --format '{{.Repository}}:{{.Tag}}' | tail -n +4 | xargs -r docker rmi >/dev/null 2>&1 || true; }
cd ~/opencanvas
set_image() { sed -i -E "s|^\$1=.*|\$1=\$2|" .env; }
if [ "$deploy" = --deploy ]; then
  if [ "$target" != frontend ]; then set_image OC_RUNTIME_IMAGE pinselo-runtime:$sha; fi
  if [ "$target" != backend ]; then set_image OC_GATEWAY_IMAGE pinselo-gateway:$sha; fi
  if [ "$target" = frontend ]; then ./update.sh web; else ./update.sh; fi
fi
keep pinselo-runtime; keep pinselo-gateway
docker buildx prune --builder pinselo --keep-storage 20g -f >/dev/null 2>&1 || true
echo "build-job: done $sha $target"
REMOTE
ssh "$host" 'chmod +x ~/opencanvas/build-job.sh; cd ~/opencanvas; rm -f build.status; nohup sh -c "./build-job.sh > build.log 2>&1; echo \$? > build.status" >/dev/null 2>&1 &'
echo "Building $sha ($target) on the server. Log: ssh $host tail -f opencanvas/build.log"
until status=$(ssh "$host" 'cat ~/opencanvas/build.status 2>/dev/null') && [ -n "$status" ]; do sleep 60; done
ssh "$host" 'tail -n 3 ~/opencanvas/build.log'
exit "$status"
