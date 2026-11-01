import bisect
import os
import re
from collections import defaultdict

from sqltext import analyze

SERVICES_TOP = ["api", "jobs", "streaming", "storage", "gateway", "backend-contracts"]
CALL_RE = re.compile(
    r"\b(?:sqlx::)?(query_file_as|query_file_scalar|query_file|query_as|query_scalar|query_unchecked|query|raw_sql)(!)?\s*(?:::<[^;{}]*?>)?\s*\("
)
PGC_RE = re.compile(r"\.(query_opt|query_one|query|execute|prepare_cached|prepare|batch_execute|simple_query)\s*\(")
QB_RE = re.compile(r"\bQueryBuilder\s*(?:::<[^;{}]*?>)?\s*::new\s*\(")
TEST_ATTR_RE = re.compile(r"#\[\s*(?:cfg\(test\)|test|tokio::test[^\]]*|sqlx::test[^\]]*|rstest[^\]]*)\s*\]")
FN_RE = re.compile(r"\bfn\s+(\w+)")
CONST_RE = re.compile(r"\b(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+(\w+)\s*:\s*&(?:'static\s+)?str\s*=\s*")
SQLISH = re.compile(
    r"^\s*(?:\(\s*)?(select|insert|update|delete|with|merge|truncate|lock|set|show|create|alter|drop|vacuum|analyze|explain|call"
    r"|do|begin|commit|rollback|listen|notify|copy|refresh|reindex|grant|discard|values|reset|checkpoint)\b",
    re.I,
)
FULL_SQL = re.compile(r"^\s*(select|insert|update|delete|with|merge|truncate)\b", re.I)
TEST_NAME = re.compile(
    r"(^|/)(tests?|fixtures?)/|(^|/)(tests?|test_[^/]*|[^/]*_tests?|[^/]*_fixtures?|live_fixture|test_schema|test_support|testing)\.rs$"
)
RAW_STR = re.compile(r'b?r(#*)"')
CHAR_LIT = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^\\'])'")
ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "0": "\0"}


def read(p):
    with open(p, encoding="utf-8", errors="replace") as f:
        return f.read()


def blank(arr, a, b):
    for k in range(a, b):
        if arr[k] != "\n":
            arr[k] = " "


def starts_word(text, i):
    return i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")


def read_string(text, i):
    n = len(text)
    j = i + 1
    buf = []
    while j < n and text[j] != '"':
        if text[j] != "\\" or j + 1 >= n:
            buf.append(text[j])
            j += 1
            continue
        e = text[j + 1]
        if e in "\r\n":
            j += 2
            while j < n and text[j] in " \t\r\n":
                j += 1
        elif e == "u":
            k = text.find("}", j)
            try:
                buf.append(chr(int(text[j + 3 : k], 16)))
            except ValueError:
                pass
            j = k + 1
        elif e == "x":
            try:
                buf.append(chr(int(text[j + 2 : j + 4], 16)))
            except ValueError:
                pass
            j += 4
        else:
            buf.append(ESCAPES.get(e, e))
            j += 2
    return j, "".join(buf)


def lex(text):
    n = len(text)
    nocomm = list(text)
    masked = list(text)
    lits = []
    i = 0
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(nocomm, i, j)
            blank(masked, i, j)
            i = j
        elif text.startswith("/*", i):
            depth = 1
            j = i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            blank(nocomm, i, j)
            blank(masked, i, j)
            i = j
        elif c in "rb" and starts_word(text, i) and RAW_STR.match(text, i) and (c == "r" or text[i + 1] == "r"):
            m = RAW_STR.match(text, i)
            hashes = m.group(1)
            start = m.end()
            end = text.find('"' + hashes, start)
            end = n if end < 0 else end
            lits.append((i, end + 1 + len(hashes), text[start:end]))
            blank(masked, start, end)
            i = end + 1 + len(hashes)
        elif c == '"' or (c == "b" and i + 1 < n and text[i + 1] == '"' and starts_word(text, i)):
            s0 = i
            if c == "b":
                i += 1
            j, value = read_string(text, i)
            lits.append((s0, j + 1, value))
            blank(masked, i + 1, j)
            i = j + 1
        elif c == "'":
            m = CHAR_LIT.match(text, i)
            if m:
                blank(masked, i + 1, m.end() - 1)
                i = m.end()
            else:
                i += 1
        else:
            i += 1
    return "".join(nocomm), "".join(masked), lits


def match_close(masked, open_pos):
    pairs = {"(": ")", "{": "}", "[": "]"}
    o = masked[open_pos]
    cl = pairs[o]
    depth = 0
    for k in range(open_pos, len(masked)):
        if masked[k] == o:
            depth += 1
        elif masked[k] == cl:
            depth -= 1
            if depth == 0:
                return k
    return len(masked) - 1


def split_args(masked, a, b):
    parts = []
    depth = 0
    last = a
    for k in range(a, b):
        ch = masked[k]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif ch == "," and depth == 0:
            parts.append((last, k))
            last = k + 1
    if masked[last:b].strip():
        parts.append((last, b))
    return parts


def statement_end(masked, k):
    depth = 0
    while k < len(masked):
        ch = masked[k]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
            if depth < 0:
                break
        elif ch == ";" and depth == 0:
            break
        k += 1
    return k


def load_crates(root):
    dirs = {top: os.path.join(root, top) for top in SERVICES_TOP}
    utils = os.path.join(root, "utils")
    for d in sorted(os.listdir(utils)):
        p = os.path.join(utils, d)
        if not os.path.isdir(p):
            continue
        if os.path.exists(os.path.join(p, "Cargo.toml")):
            dirs["utils/" + d] = p
            continue
        for dd in sorted(os.listdir(p)):
            pp = os.path.join(p, dd)
            if os.path.exists(os.path.join(pp, "Cargo.toml")):
                dirs["utils/%s/%s" % (d, dd)] = pp
    deps = {}
    for name, p in dirs.items():
        deps[name] = set()
        ct = os.path.join(p, "Cargo.toml")
        if not os.path.exists(ct):
            continue
        for m in re.finditer(r'path\s*=\s*"([^"]+)"', read(ct)):
            tp = os.path.normpath(os.path.join(p, m.group(1)))
            deps[name] |= {n2 for n2, p2 in dirs.items() if os.path.normpath(p2) == tp}
    used_by = defaultdict(set)
    for name in dirs:
        used_by[name].add(name)
        seen = set()
        stack = [name]
        while stack:
            for y in deps.get(stack.pop(), ()):
                if y not in seen:
                    seen.add(y)
                    stack.append(y)
        for y in seen:
            used_by[y].add(name)
    return dirs, used_by


def crate_of(path, dirs):
    best = None
    for name, p in dirs.items():
        if path.startswith(p + os.sep) and (best is None or len(p) > len(dirs[best])):
            best = name
    return best


def walk(root, dirs):
    rs = set()
    sqlf = set()
    for p in dirs.values():
        for dp, dn, fn in os.walk(p):
            dn[:] = [d for d in dn if d not in ("target", ".git", "node_modules")]
            parts = os.path.relpath(dp, root).split(os.sep)
            for f in fn:
                if f.endswith(".rs"):
                    rs.add(os.path.join(dp, f))
                elif f.endswith(".sql") and "queries" in parts:
                    sqlf.add(os.path.join(dp, f))
    return sorted(rs), sorted(sqlf)


def test_ranges(masked):
    ranges = []
    ext_mods = []
    whole = re.search(r"#!\[cfg\(test\)\]", masked) is not None
    for m in TEST_ATTR_RE.finditer(masked):
        pos = m.end()
        while True:
            mm = re.compile(r"\s*#\[[^\]]*\]").match(masked, pos)
            if not mm:
                break
            pos = mm.end()
        k = pos
        sc = None
        while k < len(masked):
            ch = masked[k]
            if ch in "{;":
                sc = (ch, k)
                break
            if ch == "(":
                k = match_close(masked, k)
            k += 1
        if not sc:
            continue
        if sc[0] == "{":
            ranges.append((m.start(), match_close(masked, sc[1])))
            continue
        mm = re.compile(r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;").match(masked, pos)
        if mm:
            ext_mods.append(mm.group(1))
        ranges.append((m.start(), sc[1]))
    return ranges, ext_mods, whole


class Source:
    def __init__(self, path):
        self.path = path
        self.text = read(path)
        self.nocomm, self.masked, self.lits = lex(self.text)
        self.lit_starts = [l[0] for l in self.lits]
        self.newlines = [m.start() for m in re.finditer("\n", self.text)]
        self.tests, self.ext_mods, self.whole_test = test_ranges(self.masked)

    def line(self, pos):
        return bisect.bisect_right(self.newlines, pos - 1) + 1

    def lits_in(self, a, b):
        i = bisect.bisect_left(self.lit_starts, a)
        out = []
        while i < len(self.lits) and self.lits[i][0] < b:
            out.append(self.lits[i])
            i += 1
        return out

    def enclosing_fn(self, pos):
        last = None
        for m in FN_RE.finditer(self.masked, 0, pos):
            last = m
        return last

    def include_str(self, a, b):
        m = re.search(r"include_str!\s*\(", self.masked[a:b])
        if not m:
            return None
        ls = self.lits_in(a + m.end(), b)
        return os.path.normpath(os.path.join(os.path.dirname(self.path), ls[0][2])) if ls else None


def scan(root):
    root = os.path.abspath(root)
    dirs, used_by = load_crates(root)
    rs_files, sql_files = walk(root, dirs)
    rel = lambda p: os.path.relpath(p, root)
    sources = {fp: Source(fp) for fp in rs_files}
    ext_test_files = set()
    for fp, src in sources.items():
        d = os.path.dirname(fp)
        stem = os.path.basename(fp)[:-3]
        base = d if stem in ("mod", "lib", "main") else os.path.join(d, stem)
        for mname in src.ext_mods:
            for cand in (os.path.join(base, mname + ".rs"), os.path.join(base, mname, "mod.rs"), os.path.join(d, mname + ".rs")):
                if os.path.exists(cand):
                    ext_test_files.add(cand)
                    break

    def is_test_file(fp):
        return bool(TEST_NAME.search(rel(fp))) or fp in ext_test_files

    consts = defaultdict(dict)
    for fp, src in sources.items():
        crate = crate_of(fp, dirs)
        for m in CONST_RE.finditer(src.masked):
            consts[crate].setdefault(m.group(1), []).append((fp, m.end(), statement_end(src.masked, m.end()), m.start()))

    statements = []
    sql_by_path = {}

    def base_stmt(fp, line, source, dynamic, test, fn):
        crate = crate_of(fp, dirs)
        return {
            "path": rel(fp),
            "line": line,
            "service": crate,
            "used_by": sorted(used_by[crate]),
            "source": source,
            "dynamic": dynamic,
            "callsites": [],
            "test": test,
            "fn": fn,
            "sql": "",
        }

    for sp in sql_files:
        text = read(sp)
        o = base_stmt(sp, None, "sql_file", False, False, None)
        o["sql"] = text
        o.update(analyze(text))
        statements.append(o)
        sql_by_path[sp] = o

    def new_stmt(src, pos, source, sqltext, dynamic, test, fn):
        o = base_stmt(src.path, src.line(pos), source, dynamic, test, fn)
        o["sql"] = sqltext
        o.update(analyze(sqltext, dynamic=dynamic))
        statements.append(o)
        return o

    def unresolved(src, pos, argtxt, test, fn, cs):
        o = base_stmt(src.path, src.line(pos), "unresolved:" + argtxt[:60], True, test, fn)
        o.update({"kinds": [], "writes": [], "ops": [], "reads": [], "upsert_do_update": False, "for_update": False,
                  "skip_locked": False, "limit": False, "clock": False, "assigned": []})
        o["callsites"].append(cs)
        statements.append(o)

    inline = {}
    const_stmt = {}

    def const_statement(ident, crate, fp, cs):
        cands = consts.get(crate, {}).get(ident)
        if not cands:
            return False
        cand = next((c for c in cands if c[0] == fp), cands[0])
        cfp, ca, cb, cs0 = cand
        csrc = sources[cfp]
        parts = [l for l in csrc.lits if ca <= l[0] < cb]
        if not parts:
            return False
        key = (cfp, cs0)
        o = const_stmt.get(key)
        if not o:
            test = is_test_file(cfp) or any(a <= cs0 <= b for a, b in csrc.tests)
            o = new_stmt(csrc, ca, "const", "".join(l[2] for l in parts), False, test, None)
            o["const"] = ident
            const_stmt[key] = o
            covered.update((cfp, l[0]) for l in parts)
        o["callsites"].append(cs)
        return True

    def literal_statement(src, lit, source, dynamic, cs):
        key = (src.path, lit[0])
        o = inline.get(key)
        if not o:
            o = new_stmt(src, lit[0], source, lit[2], dynamic, cs["test"], cs["fn"])
            inline[key] = o
        o["callsites"].append(cs)
        covered.add((src.path, lit[0]))
        return o

    def resolve_let(src, fnstart, callpos, ident):
        best = None
        for m in re.finditer(r"\blet\s+(?:mut\s+)?" + re.escape(ident) + r"\b[^=;]*=\s*", src.masked[fnstart:callpos]):
            best = m
        if not best:
            return None
        a = fnstart + best.end()
        return a, statement_end(src.masked, a)

    covered = set()
    for fp, src in sources.items():
        crate = crate_of(fp, dirs)
        file_test = src.whole_test or is_test_file(fp)

        def in_test(pos):
            return file_test or any(a <= pos <= b for a, b in src.tests)

        def callsite(pos):
            fnm = src.enclosing_fn(pos)
            return {"path": rel(fp), "line": src.line(pos), "fn": fnm.group(1) if fnm else None, "test": in_test(pos)}

        masked = src.masked
        for m in CALL_RE.finditer(masked):
            kind = m.group(1)
            is_macro = bool(m.group(2))
            if not is_macro and kind.startswith("query_file"):
                continue
            if not is_macro and "sqlx::" not in masked[max(0, m.start() - 6) : m.start() + 8] and kind in ("query", "query_as", "query_scalar"):
                if re.search(r"[\w.]\s*$", masked[max(0, m.start() - 3) : m.start()]):
                    continue
                if not re.search(r"\buse\s+sqlx|sqlx::\{", src.text[:2000]):
                    continue
            op = m.end() - 1
            cl = match_close(masked, op)
            args = split_args(masked, op + 1, cl)
            cs = callsite(m.start())
            if kind.startswith("query_file"):
                pl = next((l for l in src.lits_in(op + 1, cl) if l[2].endswith(".sql")), None)
                if pl:
                    sp = os.path.normpath(os.path.join(dirs[crate], pl[2]))
                    if sp in sql_by_path:
                        sql_by_path[sp]["callsites"].append(cs)
                    covered.add((fp, pl[0]))
                continue
            if not args:
                continue
            a0, b0 = args[0]
            argtxt = masked[a0:b0].strip()
            ips = src.include_str(a0, b0)
            if ips and ips.endswith(".sql"):
                if ips in sql_by_path:
                    sql_by_path[ips]["callsites"].append(cs)
                continue
            al = src.lits_in(a0, b0)
            direct = bool(al) and re.fullmatch(r'&?\s*(?:\()?\s*r?#*"[^"]*"#*\s*(?:\))?', argtxt or "x") is not None
            if not direct and al and re.match(r"\s*&?\s*\"", masked[a0:b0]) and not re.search(r"format!|concat|push", masked[a0:b0]):
                direct = True
            if direct:
                covered.update((fp, l[0]) for l in al)
                literal_statement(src, al[0], "raw_sql" if kind == "raw_sql" else "inline", False, cs)
                continue
            fmtm = re.search(r"format!\s*\(", masked[a0:b0])
            if fmtm:
                fl = src.lits_in(a0 + fmtm.end(), b0)
                if fl:
                    literal_statement(src, fl[0], "format", True, cs)
                    continue
            mm = re.match(r"&?\s*(?:mut\s+)?(?:\*\s*)?([A-Za-z_][\w:]*)\s*$", masked[a0:b0]) if argtxt else None
            if not mm:
                unresolved(src, m.start(), argtxt, cs["test"], cs["fn"], cs)
                continue
            ident = mm.group(1).split("::")[-1]
            fnm = src.enclosing_fn(m.start())
            found = resolve_let(src, fnm.start() if fnm else 0, m.start(), ident)
            if found:
                ea, eb = found
                ips2 = src.include_str(ea, eb)
                if ips2 and ips2.endswith(".sql") and ips2 in sql_by_path:
                    sql_by_path[ips2]["callsites"].append(cs)
                    continue
                el = src.lits_in(ea, eb)
                covered.update((fp, l[0]) for l in el)
                seg = masked[ea:eb]
                fm = re.search(r"format!\s*\(", seg)
                pieces = src.lits_in(ea + fm.end(), eb)[:1] if fm else [l for l in el if SQLISH.match(l[2])]
                if pieces:
                    variants = len(pieces) > 1
                    o = literal_statement(src, pieces[0], "var_format" if fm else "var", bool(fm), cs)
                    if variants and "variants" not in o:
                        o["variants"] = len(pieces)
                        for pc in pieces[1:]:
                            an = analyze(pc[2])
                            o["sql"] += "\n;\n" + pc[2]
                            for k in ("writes", "reads", "kinds"):
                                o[k] += [t for t in an[k] if t not in o[k]]
                            o["ops"] = sorted(set(o["ops"]) | set(an["ops"]))
                            o["assigned"] = sorted(set(o["assigned"]) | set(an["assigned"]))
                            for k in ("for_update", "skip_locked", "limit", "clock", "upsert_do_update"):
                                o[k] = o[k] or an[k]
                    continue
            if const_statement(ident, crate, fp, cs):
                continue
            unresolved(src, m.start(), argtxt, cs["test"], cs["fn"], cs)

        if "tokio_postgres" in src.text or "deadpool_postgres" in src.text or "postgres::" in src.text:
            for m in PGC_RE.finditer(masked):
                op = m.end() - 1
                cl = match_close(masked, op)
                args = split_args(masked, op + 1, cl)
                if not args:
                    continue
                a0, b0 = args[0]
                seg = masked[a0:b0].strip()
                cs = callsite(m.start())
                al = src.lits_in(a0, b0)
                if al and re.fullmatch(r'&?\s*r?#*"[^"]*"#*', seg):
                    if (fp, al[0][0]) in inline or SQLISH.match(al[0][2]):
                        literal_statement(src, al[0], "pg_client", bool(re.search(r"\{[^}]*\}", al[0][2])), cs)
                    continue
                mm = re.fullmatch(r"&?\s*([A-Za-z_][\w:]*)", seg)
                if mm:
                    const_statement(mm.group(1).split("::")[-1], crate, fp, cs)

        for m in QB_RE.finditer(masked):
            op = m.end() - 1
            cl = match_close(masked, op)
            cs = callsite(m.start())
            fnm = src.enclosing_fn(m.start())
            fend = len(masked)
            if fnm:
                bo = masked.find("{", fnm.end())
                if bo >= 0:
                    fend = match_close(masked, bo)
            frags = [l[2] for l in src.lits_in(op + 1, cl)]
            covered.update((fp, l[0]) for l in src.lits_in(op + 1, cl))
            for pm in re.finditer(r"\.push(?:_str)?\s*\(", masked[m.start() : fend]):
                pa = m.start() + pm.end() - 1
                for l in src.lits_in(pa, match_close(masked, pa)):
                    frags.append(l[2])
                    covered.add((fp, l[0]))
            o = new_stmt(src, m.start(), "query_builder", " ".join(frags), True, cs["test"], cs["fn"])
            o["callsites"].append(cs)

        for (s, e, v) in src.lits:
            if (fp, s) in covered or (fp, s) in inline:
                continue
            if len(v) < 25 or not FULL_SQL.match(v) or not re.search(r"\b(from|into|set|table|values|select)\b", v, re.I):
                continue
            has_ph = re.search(r"\{[^}]*\}", v) is not None
            if not has_ph and "\n" not in v and len(v.split()) < 4:
                continue
            in_const = any(c[0] == fp and c[1] <= s < c[2] for cv in consts.get(crate, {}).values() for c in cv)
            fnm = src.enclosing_fn(s)
            new_stmt(src, s, "const_unused" if in_const else "literal", v, has_ph, in_test(s), fnm.group(1) if fnm else None)

    for o in statements:
        if o["source"] == "sql_file":
            o["test"] = bool(o["callsites"]) and all(c["test"] for c in o["callsites"])
            o["orphan"] = not o["callsites"]
        elif len(o["callsites"]) > 1:
            o["test"] = all(c["test"] for c in o["callsites"])
    for o in statements:
        lits = sources[os.path.join(root, o["path"])].lits if o["dynamic"] and o["path"].endswith(".rs") else []
        o["file_tables"] = sorted({l[2] for l in lits if re.fullmatch(r"[a-z_][a-z0-9_]*", l[2])})
    statements.sort(key=lambda o: (o["path"], o["line"] or 0, o["source"]))
    return statements


def services(o):
    if o["service"].startswith("utils/"):
        return sorted(x for x in o["used_by"] if not x.startswith("utils/"))
    return [o["service"]]


def site(o):
    if o["source"] == "sql_file":
        return o["path"]
    return "%s#%s" % (o["path"], o.get("fn") or o.get("const") or o["line"])
