import argparse
import glob
import json
import os
import re
import sys

import codemap
from sqltext import (
    CTE_RE,
    JOIN_RE,
    KEYWORDS,
    MASK_FROM_RE,
    USING_RE,
    FROM_RE,
    blank_literals,
    parse_from_list,
    placeholders,
    strip_sql_comments,
)

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SCOPES = (
    "api/queries/chat/",
    "api/queries/rooms/",
    "api/queries/auth/",
    "api/src/modules/chat/",
    "api/src/modules/rooms/",
    "api/src/rt/",
)
IDENT = r'(?:"[^"]+"|\w+)'
CREATE_RE = re.compile(
    r"\bcreate\s+(?:unlogged\s+|temp(?:orary)?\s+)?(?:materialized\s+)?(?:or\s+replace\s+)?(?:table|view)\s+(?:if\s+not\s+exists\s+)?"
    r"(?:(" + IDENT + r")\s*\.\s*)?(" + IDENT + ")"
)
DROP_RE = re.compile(
    r"\bdrop\s+(?:materialized\s+)?(?:table|view)\s+(?:if\s+exists\s+)?(?:(" + IDENT + r")\s*\.\s*)?(" + IDENT + ")"
)
RENAME_RE = re.compile(
    r"\balter\s+(?:materialized\s+)?(?:table|view)\s+(?:if\s+exists\s+)?(?:only\s+)?(" + IDENT + r")\s*\.\s*(" + IDENT + r")\s+rename\s+to\s+(" + IDENT + ")"
)
WRITE_REFS = [
    re.compile(r"\binsert\s+into\s+(?:only\s+)?(?P<n>" + IDENT + r")(?P<q>\s*\.)?"),
    re.compile(r"\bdelete\s+from\s+(?:only\s+)?(?P<n>" + IDENT + r")(?P<q>\s*\.)?"),
    re.compile(r"\bmerge\s+into\s+(?P<n>" + IDENT + r")(?P<q>\s*\.)?"),
    re.compile(r"(?<!\bdo )(?<!\bfor )(?<!\bshare )(?<!\bkey )(?<!\bon )\bupdate\s+(?:only\s+)?(?P<n>" + IDENT + r")(?P<q>\s*\.)?"),
    re.compile(r"\btable\s+(?:only\s+)?(?P<n>" + IDENT + r")(?P<q>\s*\.)?"),
]


def unquote(n):
    return n.replace('"', "").lower()


def migration_names(migrations):
    chat = set()
    public = set()
    for path in sorted(glob.glob(os.path.join(migrations, "*.sql"))):
        with open(path, encoding="utf-8", errors="replace") as f:
            text = blank_literals(strip_sql_comments(f.read())).lower()
        events = []
        for m in CREATE_RE.finditer(text):
            events.append((m.start(), "add", m.group(1), m.group(2)))
        for m in DROP_RE.finditer(text):
            events.append((m.start(), "drop", m.group(1), m.group(2)))
        for m in RENAME_RE.finditer(text):
            events.append((m.start(), "drop", m.group(1), m.group(2)))
            events.append((m.start(), "add", m.group(1), m.group(3)))
        for _, kind, schema, name in sorted(events):
            schema = unquote(schema) if schema else "public"
            target = chat if schema == "chat" else public if schema == "public" else None
            if target is None:
                continue
            if kind == "add":
                target.add(unquote(name))
            else:
                target.discard(unquote(name))
    return chat, public


def snapshot_tables(schema_json):
    with open(schema_json, encoding="utf-8") as f:
        data = json.load(f)
    return {unquote(t) for tables in data.values() for t in tables}


def clashing(root):
    chat, public = migration_names(os.path.join(root, "api", "migrations"))
    return sorted(chat & (public | snapshot_tables(os.path.join(root, "ops", "replication", "schema.json"))))


def references(sql, names, dynamic=False):
    raw = placeholders(sql) if dynamic else sql
    stripped = strip_sql_comments(raw)
    clean = blank_literals(stripped)
    low = MASK_FROM_RE.sub(lambda m: m.group(1) + "(", clean.lower())
    cte = {m.group(1) for m in CTE_RE.finditer(re.sub(r"\s+", " ", low))}
    hits = []

    def add(pos, raw_name):
        name = unquote(raw_name)
        if name in names and name not in cte and name not in KEYWORDS:
            hits.append((low.count("\n", 0, pos) + 1, name))

    flat = re.sub(r"\s", lambda m: m.group(0) if m.group(0) == "\n" else " ", low)
    for m in FROM_RE.finditer(flat):
        pos = m.end()
        for nm in parse_from_list(flat, pos):
            if "." not in nm.replace('"', ""):
                at = flat.find(nm, pos)
                add(at, nm)
                pos = at + len(nm)
    for rx in (JOIN_RE, USING_RE):
        for m in rx.finditer(flat):
            if "." not in m.group(1).replace('"', ""):
                add(m.start(1), m.group(1))
    for rx in WRITE_REFS:
        for m in rx.finditer(flat):
            if not m.group("q"):
                add(m.start("n"), m.group("n"))
    return sorted(set(hits))


def scan(root, names):
    found = []
    for o in codemap.scan(root):
        if not o["path"].startswith(SCOPES):
            continue
        for line, name in references(o["sql"], names, o["dynamic"]):
            at = line if o["source"] == "sql_file" else (o["line"] or 1) + line - 1
            found.append((o["path"], at, name))
    return sorted(set(found))


def main():
    ap = argparse.ArgumentParser(description="chat-api SQL must qualify names that exist in both chat and public")
    ap.add_argument("--root", default=ROOT)
    args = ap.parse_args()
    names = clashing(args.root)
    found = scan(args.root, set(names))
    for path, line, name in found:
        print("%s:%d: unqualified %s, qualify as public.%s" % (path, line, name, name))
    print("schema-clash: names %s, %d unqualified" % (",".join(names) or "none", len(found)))
    if found:
        sys.exit(1)


if __name__ == "__main__":
    main()
