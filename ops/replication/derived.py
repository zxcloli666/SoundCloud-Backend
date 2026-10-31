import re

from sqltext import blank_literals, strip_sql_comments

NAME = r'(?:"?public"?\.)?"?(\w+)"?'
ASSIGN = re.compile(r"\bnew\.(\w+)\s*:=")
UPDATE = re.compile(r"(?<!\bdo )\bupdate\s+(?:only\s+)?" + NAME + r"(?:\s+(?:as\s+)?(?!set\b)\w+)?\s+set\s+")
INSERT = re.compile(r"\binsert\s+into\s+(?:only\s+)?" + NAME)
DELETE = re.compile(r"\bdelete\s+from\s+(?:only\s+)?" + NAME)
DROPS = re.compile(r"\breturn\s+null\b")
END = re.compile(r" (?:where|from|returning)\b|;")


def writes(source, own, before):
    text = re.sub(r"\s+", " ", blank_literals(strip_sql_comments(source or "")).lower())
    cols = {(own, c) for c in ASSIGN.findall(text)}
    for m in UPDATE.finditer(text):
        cols |= {(m.group(1), c) for c in set_columns(text, m.end())}
    rows = {m.group(1) for m in INSERT.finditer(text)} | {m.group(1) for m in DELETE.finditer(text)}
    if before and DROPS.search(text):
        rows.add(own)
    return cols, rows


def set_columns(text, pos):
    depth = 0
    start = pos
    out = []
    i = pos
    while i < len(text):
        c = text[i]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
        elif depth == 0 and (c == "," or END.match(text, i)):
            out.append(text[start:i])
            if c != ",":
                break
            start = i + 1
        i += 1
    else:
        out.append(text[start:])
    return [m.group(1) for m in (re.match(r"\s*(\w+)\s*=", part) for part in out) if m]


def declared(decl):
    cols = set()
    rows = set()
    for item in (decl.get("writes") or []) + (decl.get("vetoes") or []):
        if "." in item:
            cols.add(tuple(item.split(".", 1)))
        else:
            rows.add(item)
    rows |= set(decl.get("rows") or [])
    return cols, rows


def core_columns(model):
    out = {}
    for t in model.tables.values():
        for decl in t.derived.values():
            for item in (decl or {}).get("writes") or []:
                tbl, col = item.split(".", 1)
                out.setdefault(tbl, []).append(col)
    return {tbl: sorted(set(cols)) for tbl, cols in out.items()}


def problems(model):
    out = []
    for t in model.tables.values():
        for name, decl in sorted(t.derived.items()):
            out += trigger_problems(model, t, name, decl or {})
    for tbl, cols in sorted(core_columns(model).items()):
        target = model.tables.get(tbl)
        if target is None:
            out.append("core column %s: unknown table %s" % (", ".join(cols), tbl))
            continue
        if target.cls != "C":
            out.append("%s: core columns only on C tables" % tbl)
        for c in cols:
            if c not in target.replicated or c in target.pk:
                out.append("%s.%s: a core column must be a replicated non-key column" % (tbl, c))
    return out


def trigger_problems(model, t, name, decl):
    out = []
    for key in sorted(set(decl) - {"writes", "vetoes", "rows"}):
        out.append("%s: derived trigger %s: unknown key %r" % (t.name, name, key))
    info = t.trigger_info.get(name)
    if not info:
        return out
    cols, rows = writes(info.get("source"), t.name, info.get("before"))
    want_cols, want_rows = declared(decl)
    for tbl, col in sorted(cols):
        if (tbl, col) not in want_cols and tbl not in want_rows:
            out.append("%s: derived trigger %s writes %s.%s; register it under writes (a core column) or vetoes" % (t.name, name, tbl, col))
    for tbl in sorted(rows - want_rows):
        out.append("%s: derived trigger %s writes rows of %s; list it under rows or vetoes" % (t.name, name, tbl))
    for tbl, col in sorted(want_cols - cols):
        out.append("%s: derived trigger %s does not write %s.%s; drop it from the declaration" % (t.name, name, tbl, col))
    for tbl in sorted(set(decl.get("rows") or []) - rows):
        out.append("%s: derived trigger %s does not write rows of %s; drop it from rows" % (t.name, name, tbl))
    for tbl in sorted(set(decl.get("rows") or [])):
        for kind, k in sorted(model.kinds.items()):
            touched = set(k.get("touches") or [])
            if t.name in touched and tbl in touched:
                out.append("kind %s: touches %s and %s, whose rows the derived trigger %s writes on core only" % (kind, t.name, tbl, name))
    return out
