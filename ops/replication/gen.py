import argparse
import difflib
import os
import sys

import gen_funcs
import gen_install
import gen_tables
import gen_triggers
import model

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "generated")
MIGRATIONS = os.path.join(os.path.dirname(os.path.dirname(HERE)), "api", "migrations")


def render(m):
    return {
        "tables.sql": gen_tables.render(m),
        "triggers.sql": gen_triggers.render(m),
        "row_hash.sql": gen_funcs.row_hash(m),
        "row_json.sql": gen_funcs.row_json(m),
        "registry.sql": gen_funcs.registry(m),
        "purge_children.sql": gen_funcs.purge(m),
        "publications.sql": gen_funcs.publications(m),
    }


def main():
    ap = argparse.ArgumentParser(description="generate replication SQL from ownership.yaml")
    ap.add_argument("--check", action="store_true", help="fail when generated/ differs from a fresh render")
    ap.add_argument("--yaml")
    ap.add_argument("--schema")
    ap.add_argument("--out", default=OUT)
    ap.add_argument("--install", metavar="MIGRATION", help="write the install migration (api/migrations/NNNN_scd_install.sql)")
    ap.add_argument("--migrations", default=MIGRATIONS)
    args = ap.parse_args()
    m = model.load(args.yaml, args.schema)
    problems = m.problems()
    if problems:
        for p in problems:
            print("ownership.yaml: " + p, file=sys.stderr)
        sys.exit(1)
    files = render(m)
    install = gen_install.render(m)
    if args.check:
        sys.exit(check(files, args.out) | check_install(install, args.migrations))
    if args.install:
        with open(args.install, "w", encoding="utf-8", newline="") as f:
            f.write(install)
    os.makedirs(args.out, exist_ok=True)
    for name, text in sorted(files.items()):
        with open(os.path.join(args.out, name), "w", encoding="utf-8", newline="") as f:
            f.write(text)
    stale = sorted(set(os.listdir(args.out)) - set(files))
    for name in stale:
        os.remove(os.path.join(args.out, name))


def check(files, out):
    bad = 0
    present = set(os.listdir(out)) if os.path.isdir(out) else set()
    for name in sorted(present - set(files)):
        print("generated/%s: not produced by gen.py" % name, file=sys.stderr)
        bad = 1
    for name, text in sorted(files.items()):
        path = os.path.join(out, name)
        old = read(path) if os.path.exists(path) else ""
        if old != text:
            diff = difflib.unified_diff(old.splitlines(), text.splitlines(), "generated/" + name, "gen.py", lineterm="", n=1)
            print("\n".join(list(diff)[:40]), file=sys.stderr)
            bad = 1
    if bad:
        print("generated SQL is stale: run python3 ops/replication/gen.py and commit", file=sys.stderr)
    return bad


def check_install(text, migrations):
    path = gen_install.latest(migrations)
    if path is None:
        print("no *_scd_install.sql migration: run gen.py --install api/migrations/NNNN_scd_install.sql", file=sys.stderr)
        return 1
    if read(path) == text:
        return 0
    diff = difflib.unified_diff(read(path).splitlines(), text.splitlines(), os.path.basename(path), "gen.py", lineterm="", n=1)
    print("\n".join(list(diff)[:40]), file=sys.stderr)
    print("%s is stale: add a new migration with gen.py --install api/migrations/NNNN_scd_install.sql" % os.path.basename(path),
          file=sys.stderr)
    return 1


def read(path):
    with open(path, encoding="utf-8", newline="") as f:
        return f.read()


if __name__ == "__main__":
    main()
