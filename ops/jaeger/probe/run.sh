#!/bin/sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
run=$here/run
cfg=$(cd "$here/.." && pwd)/config.yaml
img=docker.io/jaegertracing/jaeger:2.22.0
"$here/certs.sh" "$run/certs" >/dev/null 2>&1
podman network create jprobe >/dev/null 2>&1 || true
start() {
  name=$1; alias=$2; certs=$3; peer=$4; shift 4
  podman volume create "$name" >/dev/null 2>&1 || true
  podman run -d --name "$name" --network jprobe --network-alias "$alias" "$@" --user 0 \
    -v "$cfg:/etc/jaeger/config.yaml:ro" -v "$run/certs/$certs:/certs:ro" -v "$name:/badger" \
    -e "TELEMETRY_PEER=$peer" -e SCD_HOST=probe "$img" --config /etc/jaeger/config.yaml >/dev/null
}
start jprobe-b jaeger-b.peer.test b jaeger-a.peer.test:14317 --network-alias jaeger-bwrong.peer.test -p 127.0.0.1:26687:16686
start jprobe-a jaeger-a.peer.test a jaeger-b.peer.test:14317 -p 127.0.0.1:24317:4317 -p 127.0.0.1:26686:16686
start jprobe-w jaeger-w.peer.test w jaeger-bwrong.peer.test:14317 -p 127.0.0.1:24318:4317
sleep 8
for n in jprobe-a jprobe-b jprobe-w; do printf '%s: ' "$n"; podman inspect -f '{{.State.Status}}' "$n"; done
podman run --rm --network jprobe -v "$run/certs:/certs:ro" -v "$here:/p:ro" docker.io/library/python:3.12-alpine \
  python -I /p/tls_client.py jaeger-b.peer.test 14317 /certs
