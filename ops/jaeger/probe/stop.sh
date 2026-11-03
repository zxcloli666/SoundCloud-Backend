#!/bin/sh
podman rm -f jprobe-a jprobe-b jprobe-w >/dev/null 2>&1
podman network rm jprobe >/dev/null 2>&1
podman volume rm jprobe-a jprobe-b jprobe-w >/dev/null 2>&1
rm -rf "$(cd "$(dirname "$0")" && pwd)/run"
