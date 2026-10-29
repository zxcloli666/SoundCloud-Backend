import argparse
import glob
import os
import re
import sys

import model
from sqltext import DOLLAR_TAG, analyze, blank_literals, split_statements, strip_sql_comments, uses_clock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
MIGRATIONS = os.path.join(ROOT, "api", "migrations")
PUBLISHED = ("C", "M", "A", "K", "X")
DML = ("insert", "update", "delete", "merge", "truncate", "copy")
NAME = r'(?:"?public"?\.)?"?(\w+)"?'
ALTER_RE = re.compile(r"^alter\s+table\s+(?:if\s+exists\s+)?(?:only\s+)?" + NAME + r"\s+(.*)$", re.S)
UNPUBLISH_RE = re.compile(r"scd_unpublish_column\s*\(\s*'(\w+)'\s*,\s*'(\w+)'")
FK_ACTION_RE = re.compile(r"\bon\s+(?:delete|update)\s+cascade\b")
NOT_COLUMN = {"constraint", "default", "not", "identity", "expression"}


def units(text):
    out = []
    for stmt in split_statements(strip_sql_comments(text)):
        head = stmt.lstrip().lower()
        if head.startswith("do") and re.match(r"do\s*(\$|language\b)", head):
            body = do_body(stmt)
            gated = bool(re.search(r"\bscd_role\s*\(\s*\)", body))
            out += [(s, gated) for s in split_statements(body)]
        else:
            out.append((stmt, False))
    return out


def do_body(stmt):
    m = DOLLAR_TAG.search(stmt)
    if not m:
        return ""
    end = stmt.find(m.group(0), m.end())
    return stmt[m.end() : end if end >= 0 else len(stmt)]


def statement_problems(m, stmt, gated, unpublished):
    out = []
    clean = re.sub(r"\s+", " ", blank_literals(stmt).lower()).strip()
    if re.match(r"(create|alter|drop)\s+(or\s+replace\s+)?(function|procedure|trigger)\b", clean):
        return out
    a = analyze(stmt)
    if not gated and (a["writes"] or (a["kinds"] and a["kinds"][0] in DML)):
        out.append(("dml-outside-gate", clean[:80]))
    if re.search(r"\bcascade\b", FK_ACTION_RE.sub(" ", clean)):
        out.append(("cascade", clean[:80]))
    if uses_clock(stmt):
        out.append(("clock", clean[:80]))
    am = ALTER_RE.match(clean)
    if am:
        out += alter_problems(m, am.group(1), am.group(2), unpublished)
    return out


def alter_problems(m, table, actions, unpublished):
    t = m.tables.get(table)
    if t is None or t.cls not in PUBLISHED:
        return []
    out = []
    words = re.findall(r'"[^"]+"|\w+', actions)
    words = [w.strip('"') for w in words]
    for i, w in enumerate(words):
        if w == "drop" and i + 1 < len(words):
            j = i + 1
            if words[j] == "column":
                j += 1
            elif words[j] in NOT_COLUMN:
                continue
            if words[j : j + 2] == ["if", "exists"]:
                j += 2
            col = words[j] if j < len(words) else None
            if col in t.replicated and (table, col) not in unpublished:
                out.append(("drop-column", "%s.%s is published; unpublish it with scd_unpublish_column in an earlier migration" % (table, col)))
        if w == "rename" and i + 1 < len(words):
            j = i + 1
            if words[j] == "to":
                out.append(("rename-table", "%s is published; subscribers match tables by name" % table))
                continue
            if words[j] == "constraint":
                continue
            if words[j] == "column":
                j += 1
            col = words[j] if j < len(words) else None
            if col in t.replicated and (table, col) not in unpublished:
                out.append(("rename-column", "%s.%s is published; unpublish it with scd_unpublish_column in an earlier migration" % (table, col)))
    return out


def unpublish_marks(migrations):
    marks = []
    for p in sorted(glob.glob(os.path.join(migrations, "*.sql"))):
        text = read(p)
        if "scd_unpublish_column" in text:
            marks.append((os.path.basename(p), set(UNPUBLISH_RE.findall(strip_sql_comments(text)))))
    return marks


def check_file(m, path, marks):
    name = os.path.basename(path)
    unpublished = set().union(*[s for f, s in marks if f < name])
    out = []
    for stmt, gated in units(read(path)):
        out += statement_problems(m, stmt, gated, unpublished)
    return out


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def main():
    ap = argparse.ArgumentParser(description="replication rules for added migrations (R12)")
    ap.add_argument("files", nargs="*")
    ap.add_argument("--migrations", default=MIGRATIONS)
    ap.add_argument("--yaml")
    ap.add_argument("--schema")
    args = ap.parse_args()
    m = model.load(args.yaml, args.schema)
    marks = unpublish_marks(args.migrations)
    bad = 0
    for f in args.files:
        path = os.path.abspath(f)
        if os.path.dirname(path) != os.path.abspath(args.migrations) or not os.path.exists(path):
            continue
        for rule, text in check_file(m, path, marks):
            print("%s: %s: %s" % (os.path.relpath(path, ROOT), rule, text))
            bad = 1
    if bad:
        print("replication rules: DML only inside IF public.scd_role() = '...' in a DO block; no CASCADE; "
              "drop or rename a published column only after scd_unpublish_column in an earlier release; "
              "scd_clock() instead of catalog_metadata_clock", file=sys.stderr)
    sys.exit(bad)


if __name__ == "__main__":
    main()
