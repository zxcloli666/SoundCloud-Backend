#!/usr/bin/env bash
set -euo pipefail

MIG_DIRS=("api/migrations" "api/migrations-ops")
FILE_RE='^[0-9]{4,}_[a-z0-9_]+\.sql$'
BASE="${MIG_BASE_REF:-HEAD}"

cd "$(git rev-parse --show-toplevel)"

if [[ "${ALLOW_MIGRATION_REWRITE:-0}" == "1" ]]; then
  echo "check-migrations: ALLOW_MIGRATION_REWRITE=1 → guard skipped"
  exit 0
fi

if [[ $# -gt 0 ]]; then diff_src=("$@"); else diff_src=(--cached); fi

fail=0
all_added=()

for MIG_DIR in "${MIG_DIRS[@]}"; do
  added=()
  while IFS=$'\t' read -r status p1 p2; do
    [[ -z "${status:-}" ]] && continue
    if [[ "$status" == "A" ]]; then
      added+=("$p1")
    else
      printf '  ✖ изменена/удалена применённая миграция (%s): %s %s\n' "$status" "$p1" "${p2:-}" >&2
      fail=1
    fi
  done < <(git diff --name-status "${diff_src[@]}" -- "$MIG_DIR")

  declare -A existing=()
  max_ver=""
  while read -r f; do
    [[ -z "$f" ]] && continue
    v="${f##*/}"; v="${v%%_*}"
    existing["$v"]=1
    [[ "$v" > "$max_ver" ]] && max_ver="$v"
  done < <(git ls-tree -r --name-only "$BASE" -- "$MIG_DIR" 2>/dev/null || true)

  declare -A seen=()
  for f in "${added[@]:-}"; do
    [[ -z "$f" ]] && continue
    all_added+=("$f")
    b="${f##*/}"
    if [[ ! "$b" =~ $FILE_RE ]]; then
      printf '  ✖ имя не по формату NNNN_snake.sql: %s\n' "$b" >&2; fail=1; continue
    fi
    v="${b%%_*}"
    if [[ -n "${existing[$v]:-}" || -n "${seen[$v]:-}" ]]; then
      printf '  ✖ дубль номера в %s: %s\n' "$MIG_DIR" "$v" >&2; fail=1
    fi
    seen["$v"]=1
    if [[ -n "$max_ver" && ! "$v" > "$max_ver" ]]; then
      printf '  ✖ %s: номер %s ≤ максимума %s — нельзя вставлять в середину\n' "$MIG_DIR" "$v" "$max_ver" >&2; fail=1
    fi
  done
  unset existing seen
done

if command -v eugene >/dev/null 2>&1; then
  for f in "${all_added[@]:-}"; do
    [[ -z "$f" || ! -f "$f" ]] && continue
    eugene lint "$f" 2>&1 | sed 's/^/  [eugene] /' >&2 || true
  done
fi

if [[ "$fail" == "1" ]]; then
  cat >&2 <<'EOF'

Миграции append-only. Не редактируй закоммиченные файлы (даже комменты),
не переиспользуй номера — создай НОВЫЙ файл со следующим номером.
Намеренный squash/rewrite: ALLOW_MIGRATION_REWRITE=1 git commit …
EOF
  exit 1
fi
echo "check-migrations: ok"
