import argparse
import glob
import os
import re
import sys

import codemap
import model
from sqltext import analyze, strip_sql_comments

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
BASELINE = os.path.join(HERE, "baseline.txt")
API_SIDE = ("api", "streaming")
WT_RE = re.compile(r"\bscd_wt\s*\(\s*'([^']*)'")
KIND_FN_RE = re.compile(r"create\s+(?:or\s+replace\s+)?function\s+(?:public\.)?scd_cmd_(\w+)\s*\(", re.I)
DOLLAR_RE = re.compile(r"\$([A-Za-z_]\w*)?\$")


def tables_of(o, name, known):
    if "⟦" not in name:
        return [name] if name in known else []
    return [t for t in o.get("file_tables", []) if t in known]


def code_violations(m, statements):
    out = set()
    core_only = set(m.core_only_code)
    known = m.tables
    for o in statements:
        if o["test"]:
            continue
        where = codemap.site(o)
        if o.get("clock"):
            out.add(("clock", where, "catalog_metadata_clock"))
        for kind in WT_RE.findall(strip_sql_comments(o["sql"])):
            if kind not in m.kinds:
                out.add(("unknown-kind", where, kind))
        api_side = any(s in API_SIDE for s in codemap.services(o)) and o["path"] not in core_only
        for verb, name in o["ops"]:
            for t in tables_of(o, name, known):
                out |= statement_rules(known[t], verb, o, where, api_side)
    return out


def statement_rules(t, verb, o, where, api_side):
    out = set()
    if api_side and t.cls == "C":
        out.add(("api-write-c", where, t.name))
    if api_side and t.cls == "P":
        out.add(("api-write-p", where, t.name))
    if api_side and verb in ("delete", "truncate"):
        if t.cls == "M" or (t.cls == "A" and t.delete != "tombstone"):
            out.add(("api-delete-shared", where, t.name))
    if not api_side and t.cls == "M" and verb in ("delete", "truncate"):
        out.add(("core-delete-m", where, t.name))
    if t.cls == "A" and t.insert_only and (verb in ("update", "merge") or (verb == "insert" and o["upsert_do_update"])):
        out.add(("a-insert-only-updated", where, t.name))
    return out


def kind_functions(migrations):
    found = {}
    for path in sorted(glob.glob(os.path.join(migrations, "*.sql"))):
        text = strip_sql_comments(read(path))
        for m in KIND_FN_RE.finditer(text):
            body = function_body(text, m.end())
            if body is not None:
                found[m.group(1).lower()] = (os.path.relpath(path, ROOT), body)
    return found


def function_body(text, pos):
    m = DOLLAR_RE.search(text, pos)
    if not m:
        return None
    end = text.find(m.group(0), m.end())
    return text[m.end() : end] if end >= 0 else None


def kind_violations(m, migrations):
    out = set()
    by_fn = {k.replace(".", "_"): k for k in m.kinds}
    for fn, (path, body) in sorted(kind_functions(migrations).items()):
        where = "%s#scd_cmd_%s" % (path, fn)
        kind = by_fn.get(fn)
        if kind is None:
            out.add(("kind-unknown", where, fn))
            continue
        a = analyze(body)
        for flag, rule in (("skip_locked", "kind-skip-locked"), ("for_update", "kind-for-update"), ("limit", "kind-limit")):
            if a[flag]:
                out.add((rule, where, kind))
        touches = set(m.kinds[kind].get("touches") or [])
        for t in a["writes"]:
            if t in m.tables and t not in touches and m.tables[t].cls not in ("L", "X"):
                out.add(("kind-touch", where, t))
    return out


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def read_baseline(path):
    if not os.path.exists(path):
        return set()
    return {tuple(line.split("|")) for line in read(path).splitlines() if line.strip()}


def write_baseline(path, items):
    with open(path, "w", encoding="utf-8") as f:
        for item in sorted(items):
            f.write("|".join(item) + "\n")


def main():
    ap = argparse.ArgumentParser(description="check code against ownership.yaml")
    ap.add_argument("--root", default=ROOT)
    ap.add_argument("--yaml")
    ap.add_argument("--schema")
    ap.add_argument("--baseline", default=BASELINE)
    ap.add_argument("--update-baseline", action="store_true")
    args = ap.parse_args()
    m = model.load(args.yaml, args.schema)
    problems = m.problems()
    for p in problems:
        print("ownership.yaml: " + p)
    if problems:
        sys.exit(1)
    found = code_violations(m, codemap.scan(args.root))
    found |= kind_violations(m, os.path.join(args.root, "api", "migrations"))
    if args.update_baseline:
        write_baseline(args.baseline, found)
        print("baseline: %d entries" % len(found))
        return
    known = read_baseline(args.baseline)
    new = sorted(found - known)
    stale = sorted(known - found)
    for rule, where, table in new:
        print("%s: %s %s (%s)" % (rule, where, table, explain(rule)))
    for item in stale:
        print("fixed, remove from baseline.txt: " + "|".join(item))
    counts = {}
    for rule, _, _ in found:
        counts[rule] = counts.get(rule, 0) + 1
    print("ownership: %d known, %d new, %d stale; %s" % (len(found & known), len(new), len(stale),
          ", ".join("%s=%d" % kv for kv in sorted(counts.items())) or "clean"))
    if new or stale:
        sys.exit(1)


def explain(rule):
    return {
        "api-write-c": "api writes a core-owned table; go through scd_wt and a kind",
        "api-write-p": "api writes a core-private table; go through scd_wt and a kind",
        "api-delete-shared": "only core hard-deletes M and non-tombstone A rows (R7)",
        "core-delete-m": "core deletes an M row only through scd_purge_intent, which writes the receipt first (R7)",
        "a-insert-only-updated": "insert_only A table is updated; version it in ownership.yaml",
        "clock": "use scd_clock() instead of nextval('catalog_metadata_clock')",
        "unknown-kind": "scd_wt kind not declared in ownership.yaml",
        "kind-unknown": "scd_cmd_ function without a kind in ownership.yaml",
        "kind-skip-locked": "kinds are key-addressed, no SKIP LOCKED",
        "kind-for-update": "kinds are key-addressed, no FOR UPDATE",
        "kind-limit": "kinds are key-addressed, no LIMIT",
        "kind-touch": "kind writes a table missing from its touches list",
    }[rule]


if __name__ == "__main__":
    main()
