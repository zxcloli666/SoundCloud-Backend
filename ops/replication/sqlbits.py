import re

KEYWORDS = frozenset(
    """
    all analyse analyze and any array as asc asymmetric authorization between bigint binary bit boolean both case
    cast char character check coalesce collate collation column concurrently constraint create cross
    current_catalog current_date current_role current_schema current_time current_timestamp current_user dec
    decimal default deferrable desc distinct do else end except exists extract false fetch float for foreign
    freeze from full grant greatest group grouping having ilike in initially inner inout int integer intersect
    interval into is isnull join json json_array json_arrayagg json_exists json_object json_objectagg json_query
    json_scalar json_serialize json_table json_value lateral leading least left like limit localtime
    localtimestamp merge_action national natural nchar none normalize not notnull null nullif numeric offset on
    only or order out outer overlaps overlay placing position precision primary real references returning right
    row select session_user setof similar smallint some substring symmetric system_user table tablesample then
    time timestamp to trailing treat trim true union unique user using values varchar variadic verbose when where
    window with xmlattributes xmlconcat xmlelement xmlexists xmlforest xmlnamespaces xmlparse xmlpi xmlroot
    xmlserialize xmltable
    """.split()
)
SIMPLE = re.compile(r"[a-z_][a-z0-9_]*$")
WORD = re.compile(r"[A-Za-z_][A-Za-z0-9_$]*")


def q(name):
    if SIMPLE.match(name) and name not in KEYWORDS:
        return name
    return '"' + name.replace('"', '""') + '"'


def lit(text):
    return "'" + str(text).replace("'", "''") + "'"


def table(name):
    return "public." + q(name)


def ref(alias, name):
    return "%s.%s" % (alias, q(name))


def refs(alias, names):
    return ", ".join(ref(alias, n) for n in names)


def tup(alias, names):
    if len(names) == 1:
        return ref(alias, names[0])
    return "(" + refs(alias, names) + ")"


def same(left, right, names):
    return "%s = %s" % (tup(left, names), tup(right, names))


def pk_json(alias, names):
    return "jsonb_build_object(" + ", ".join("%s, %s" % (lit(n), ref(alias, n)) for n in names) + ")"


def pk_match(alias, t, var="k"):
    return " AND ".join("%s = (%s ->> %s)::%s" % (ref(alias, c), var, lit(c), t.types[c]) for c in t.pk)


def assign(names, src, target=None):
    return ", ".join("%s = %s" % (q(n) if target is None else ref(target, n), ref(src, n)) for n in names)


def qualify(expr, names, alias):
    out = []
    i = 0
    names = set(names)
    while i < len(expr):
        c = expr[i]
        if c == "'":
            j = i + 1
            while j < len(expr):
                if expr[j] == "'" and expr[j + 1 : j + 2] == "'":
                    j += 2
                elif expr[j] == "'":
                    break
                else:
                    j += 1
            out.append(ascii_literal(expr[i + 1 : j].replace("''", "'")))
            i = j + 1
            continue
        if c == '"':
            j = expr.index('"', i + 1)
            word = expr[i + 1 : j]
            out.append(ref(alias, word) if word in names and not after_dot(expr, i) else expr[i : j + 1])
            i = j + 1
            continue
        m = WORD.match(expr, i)
        if m and (i == 0 or not (expr[i - 1].isalnum() or expr[i - 1] in "_$")):
            word = m.group(0)
            if word in names and not after_dot(expr, i) and not before_paren(expr, m.end()):
                out.append(ref(alias, word))
            else:
                out.append(word)
            i = m.end()
            continue
        out.append(c)
        i += 1
    return "".join(out)


def ascii_literal(text):
    if all(32 <= ord(c) < 127 for c in text):
        return lit(text)
    out = []
    for c in text:
        n = ord(c)
        if c in "\\'":
            out.append("\\" + c)
        elif 32 <= n < 127:
            out.append(c)
        elif n <= 0xFFFF:
            out.append("\\u%04X" % n)
        else:
            out.append("\\U%08X" % n)
    return "E'" + "".join(out) + "'"


def after_dot(expr, i):
    j = i - 1
    while j >= 0 and expr[j] == " ":
        j -= 1
    return j >= 0 and expr[j] in ".:"


def before_paren(expr, i):
    while i < len(expr) and expr[i] == " ":
        i += 1
    return i < len(expr) and expr[i] == "("


def index_keys(u, names, alias):
    if u["cols"] is not None:
        return [ref(alias, c) for c in u["cols"]]
    return ["(%s)" % qualify(k, names, alias) for k in u["keys"]]


def index_pred(u, names, alias):
    if not u["where"]:
        return None
    return "(%s)" % qualify(u["where"], names, alias)


def keys_tuple(u, names, alias, with_pred=False):
    keys = index_keys(u, names, alias)
    if with_pred and u["where"]:
        keys.append(index_pred(u, names, alias))
    return keys[0] if len(keys) == 1 else "(" + ", ".join(keys) + ")"


def function(name, body, returns="trigger", lang="plpgsql", args="", extra=""):
    head = "CREATE OR REPLACE FUNCTION %s(%s)\nRETURNS %s\nLANGUAGE %s%s\nAS $f$\n" % (name, args, returns, lang, extra)
    return head + body.rstrip("\n") + "\n$f$;\n"


def indent(lines, n=1):
    pad = "    " * n
    return [pad + line if line else line for line in lines]
