#!/bin/sh
set -eu

case "${FLAVOR:-}" in
  cpu) set -- --group cpu "$@" ;;
  cu126 | cu130) set -- --group "$FLAVOR" --extra llm-local "$@" ;;
  *)
    echo "FLAVOR must be cpu, cu126 or cu130, got '${FLAVOR:-}'" >&2
    exit 64
    ;;
esac

exec uv sync --frozen --no-default-groups "$@"
