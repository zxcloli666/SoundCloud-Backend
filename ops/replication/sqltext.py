import re

NAME = r'((?:"[^"]+"|[\w⟦⟧]+)(?:\.(?:"[^"]+"|[\w⟦⟧]+))?)'
KEYWORDS = set(
    "select where join left right inner full cross on group order limit offset union except intersect returning set for using "
    "natural lateral window having fetch tablesample with values do when then and or as only if not exists in is null true "
    "false default case end over filter within unnest generate_series jsonb_to_recordset json_to_recordset "
    "jsonb_array_elements jsonb_each rows from by asc desc nulls first last distinct all any some update delete insert into "
    "table partition".split()
)
FN_NAMES = set(
    "unnest generate_series jsonb_to_recordset json_to_recordset jsonb_array_elements jsonb_array_elements_text jsonb_each "
    "jsonb_each_text jsonb_to_record json_array_elements regexp_split_to_table string_to_table lateral".split()
)
WRITE_RE = [
    ("insert", re.compile(r"\binsert\s+into\s+(?:only\s+)?" + NAME)),
    ("update", re.compile(r"(?<!\bdo )(?<!\bfor )(?<!\bshare )(?<!\bkey )(?<!\bon )\bupdate\s+(?:only\s+)?" + NAME + r"(?:\s+(?:as\s+)?\w+)?\s+set\b")),
    ("delete", re.compile(r"\bdelete\s+from\s+(?:only\s+)?" + NAME)),
    ("merge", re.compile(r"\bmerge\s+into\s+" + NAME)),
    ("copy", re.compile(r"\bcopy\s+" + NAME + r"\s*(?:\([^)]*\)\s*)?from\b")),
]
TRUNC_RE = re.compile(r"\btruncate\s+(?:table\s+)?(?:only\s+)?([\w⟦⟧\".]+(?:\s*,\s*[\w⟦⟧\".]+)*)")
LOCKT_RE = re.compile(r"\block\s+table\s+(?:only\s+)?" + NAME)
CTE_RE = re.compile(r"(?:\bwith\s+(?:recursive\s+)?|,\s*)(\w+)\s*(?:\([^()]*\))?\s+as\s+(?:not\s+)?(?:materialized\s+)?\(")
FROM_RE = re.compile(r"\bfrom\s+")
NF = r'(?![\w⟦⟧."]|\s*\()'
JOIN_RE = re.compile(r"\bjoin\s+(?:only\s+)?(?!\()" + NAME + NF)
USING_RE = re.compile(r"\busing\s+(?!\()" + NAME + NF)
MASK_FROM_RE = re.compile(r"\b(extract|substring|trim|overlay|position)\s*\([^()]*?\bfrom\b")
VERB_RE = re.compile(
    r"\b(select|insert|update|delete|merge|truncate|values|call|do|set|show|create|alter|drop|vacuum|analyze|refresh|reindex"
    r"|listen|notify|begin|commit|explain|lock|copy)\b"
)
PH_FMT = re.compile(r"\{(\w*)(?::[^{}]*)?\}")
DOLLAR_TAG = re.compile(r"\$([A-Za-z_]\w*)?\$")


def skip_quoted(s, i):
    n = len(s)
    j = i + 1
    while j < n:
        if s[j] == "'":
            if j + 1 < n and s[j + 1] == "'":
                j += 2
                continue
            break
        j += 1
    return j + 1


def skip_dollar(s, i):
    m = DOLLAR_TAG.match(s, i)
    if not m:
        return None
    tag = m.group(0)
    j = s.find(tag, i + len(tag))
    return len(s) if j < 0 else j + len(tag)


def skip_block_comment(s, i):
    depth = 1
    j = i + 2
    while j < len(s) and depth:
        if s.startswith("/*", j):
            depth += 1
            j += 2
        elif s.startswith("*/", j):
            depth -= 1
            j += 2
        else:
            j += 1
    return j


def strip_sql_comments(s):
    out = []
    i = 0
    n = len(s)
    while i < n:
        c = s[i]
        if c == "'":
            j = skip_quoted(s, i)
            out.append(s[i:j])
            i = j
        elif c == '"':
            j = s.find('"', i + 1)
            j = n - 1 if j < 0 else j
            out.append(s[i : j + 1])
            i = j + 1
        elif c == "$" and DOLLAR_TAG.match(s, i):
            j = skip_dollar(s, i)
            out.append(s[i:j])
            i = j
        elif s.startswith("--", i):
            j = s.find("\n", i)
            i = n if j < 0 else j
        elif s.startswith("/*", i):
            out.append(" ")
            i = skip_block_comment(s, i)
        else:
            out.append(c)
            i += 1
    return "".join(out)


def blank_literals(s):
    out = []
    i = 0
    n = len(s)
    while i < n:
        c = s[i]
        if c == "'":
            esc = i > 0 and s[i - 1] in "eE"
            j = i + 1
            while j < n:
                if esc and s[j] == "\\":
                    j += 2
                    continue
                if s[j] == "'":
                    if j + 1 < n and s[j + 1] == "'":
                        j += 2
                        continue
                    break
                j += 1
            if esc and out and out[-1] in "eE":
                out.pop()
            out.append("?")
            i = j + 1
        elif c == '"':
            j = s.find('"', i + 1)
            j = n - 1 if j < 0 else j
            out.append(s[i : j + 1])
            i = j + 1
        elif c == "$" and DOLLAR_TAG.match(s, i):
            out.append("?")
            i = skip_dollar(s, i)
        elif c == "$" and re.match(r"\$\d+", s[i:]):
            m = re.match(r"\$\d+", s[i:])
            out.append("?")
            i += len(m.group(0))
        else:
            out.append(c)
            i += 1
    return "".join(out)


def split_statements(s):
    parts = []
    cur = []
    i = 0
    n = len(s)
    while i < n:
        c = s[i]
        if c == "'":
            j = skip_quoted(s, i)
            cur.append(s[i:j])
            i = j
        elif c == "$" and DOLLAR_TAG.match(s, i):
            j = skip_dollar(s, i)
            cur.append(s[i:j])
            i = j
        elif c == ";":
            parts.append("".join(cur))
            cur = []
            i += 1
        else:
            cur.append(c)
            i += 1
    parts.append("".join(cur))
    return [p.strip() for p in parts if p.strip()]


def placeholders(s):
    s = s.replace("{{", "\x01").replace("}}", "\x02")
    s = PH_FMT.sub(lambda m: "⟦" + m.group(1) + "⟧", s)
    return s.replace("\x01", "{").replace("\x02", "}")


def clean_name(n):
    n = n.replace('"', "").lower()
    return n[7:] if n.startswith("public.") else n


def strip_depth(s):
    out = []
    depth = 0
    for c in s:
        if c == "(":
            depth += 1
        elif c == ")":
            depth = max(0, depth - 1)
        elif depth == 0:
            out.append(c)
    return "".join(out)


def parse_from_list(s, pos):
    names = []
    while True:
        m = re.compile(r"\s*" + NAME + NF).match(s, pos)
        if not m:
            break
        names.append(m.group(1))
        pos = m.end()
        m2 = re.compile(r"\s+(?:as\s+)?(\w+)").match(s, pos)
        if m2 and m2.group(1) not in KEYWORDS:
            pos = m2.end()
        m3 = re.compile(r"\s*,").match(s, pos)
        if not m3:
            break
        pos = m3.end()
    return names


def analyze(sql_text, dynamic=False):
    raw = placeholders(sql_text) if dynamic else sql_text
    clean = blank_literals(strip_sql_comments(raw)).lower()
    clean = re.sub(r"\s+", " ", clean)
    cte = {m.group(1) for m in CTE_RE.finditer(clean)}
    clean_nf = MASK_FROM_RE.sub(lambda m: m.group(1) + "(", clean)
    writes = []
    verbs = []
    skip_from = set()
    for kind, rx in WRITE_RE:
        for m in rx.finditer(clean):
            t = clean_name(m.group(1))
            if t in cte or t in KEYWORDS:
                continue
            if kind == "delete":
                skip_from.add(m.start() + clean[m.start() :].find("from"))
            if t not in writes:
                writes.append(t)
            verbs.append((kind, t))
    for m in TRUNC_RE.finditer(clean):
        for t in m.group(1).split(","):
            t = clean_name(t.strip())
            if t and t not in writes:
                writes.append(t)
            verbs.append(("truncate", t))
    reads = []
    for m in FROM_RE.finditer(clean_nf):
        if m.start() in skip_from or clean_nf[max(0, m.start() - 12) : m.start()].endswith("distinct "):
            continue
        for nm in parse_from_list(clean_nf, m.end()):
            t = clean_name(nm)
            if t and t not in cte and t not in KEYWORDS and t not in FN_NAMES and t not in reads:
                reads.append(t)
    for rx in (JOIN_RE, USING_RE):
        for m in rx.finditer(clean_nf):
            t = clean_name(m.group(1))
            if t and t not in cte and t not in KEYWORDS and t not in FN_NAMES and t not in reads:
                reads.append(t)
    top = re.sub(r"^\s*with\s+(?:recursive\s+)?", "", strip_depth(clean))
    pm = VERB_RE.search(top)
    primary = pm.group(1) if pm else None
    kinds = []
    if primary in ("select", "values"):
        kinds.append("select")
    elif primary in ("insert", "update", "delete", "merge", "truncate", "copy", "lock"):
        kinds.append(primary)
    elif primary:
        kinds.append("other:" + primary)
    for v, _ in verbs:
        if v not in kinds:
            kinds.append(v)
    if "on conflict" in clean and "upsert" not in kinds:
        kinds.append("upsert")
    return {
        "kinds": kinds,
        "writes": writes,
        "ops": sorted(set(verbs)),
        "reads": reads,
        "upsert_do_update": bool(re.search(r"on conflict[^;]*?do update", clean)),
        "for_update": bool(re.search(r"\bfor\s+(no\s+key\s+)?update\b", clean)),
        "skip_locked": "skip locked" in clean,
        "limit": bool(re.search(r"\blimit\b", clean)),
        "clock": uses_clock(raw),
    }


def uses_clock(sql_text):
    return re.search(r"nextval\s*\(\s*'(?:public\.)?catalog_metadata_clock'", strip_sql_comments(sql_text).lower()) is not None
