from model import purge_action
from sqlbits import function, lit, pk_json, pk_match, q, refs, table, tup

HASH_SETTINGS = (
    "\nSET timezone TO 'UTC'"
    "\nSET extra_float_digits TO 1"
    "\nSET datestyle TO 'ISO, YMD'"
    "\nSET intervalstyle TO 'postgres'"
    "\nSET bytea_output TO 'hex'"
)


def hashed(model):
    return [t for t in model.installed() if t.cls in ("C", "P", "M", "A", "K") and t.pk and (t.exists or t.spec.get("columns"))]


def row_hash(model):
    parts = []
    tables = hashed(model)
    for t in tables:
        body = "SELECT md5(ROW(%s)::text) FROM %s t WHERE %s" % (refs("t", t.replicated), table(t.name), pk_match("t", t))
        parts.append(function("public.scd_row_hash_" + t.name, body, "text", "sql", "k jsonb", " STABLE" + HASH_SETTINGS))
    parts.append(dispatcher("public.scd_row_hash", "text", [(t.name, "RETURN public.scd_row_hash_%s(k);" % t.name) for t in tables], " STABLE"))
    return "\n".join(parts)


def dispatcher(name, returns, cases, extra=""):
    lines = ["BEGIN", "    CASE tbl"]
    for tbl, stmt in cases:
        lines += ["        WHEN %s THEN" % lit(tbl), "            " + stmt]
    lines += ["        ELSE", "            RAISE EXCEPTION '%s: unknown table %%', tbl;" % name, "    END CASE;"]
    if returns == "void":
        lines.append("    RETURN;")
    lines.append("END")
    return function(name, "\n".join(lines), returns, "plpgsql", "tbl text, k jsonb", extra)


def purge_tables(model):
    roots = [t for t in model.installed() if t.cls in ("C", "P") and t.pk]
    seen = {}
    todo = list(roots)
    while todo:
        t = todo.pop()
        if t.name in seen:
            continue
        seen[t.name] = t
        for child, fk in model.children(t.name):
            if child_action(model, child, fk) == "recurse":
                todo.append(child)
    return [seen[n] for n in sorted(seen)]


def child_action(model, child, fk):
    action = purge_action(child, fk)
    if action != "purge":
        return action
    if model.children(child.name):
        if not child.pk:
            raise SystemExit("%s: purge needs a primary key to recurse into its children" % child.name)
        return "recurse"
    return "delete"


def purge(model):
    parts = []
    tables = purge_tables(model)
    for t in tables:
        parts.append(purge_function(model, t))
    parts.append(dispatcher("public.scd_purge", "void", [(t.name, "PERFORM public.scd_purge_%s(k);" % t.name) for t in tables]))
    return "\n".join(parts)


def purge_function(model, t):
    children = model.children(t.name)
    needed = sorted(set(t.pk) | {c for _, fk in children for c in fk["ref_cols"]})
    lines = ["DECLARE", "    p record;"]
    if any(child_action(model, c, fk) == "recurse" for c, fk in children):
        lines.append("    r record;")
    lines += [
        "BEGIN",
        "    SELECT %s INTO p FROM %s t WHERE %s;" % (refs("t", needed), table(t.name), pk_match("t", t)),
        "    IF NOT FOUND THEN",
        "        RETURN;",
        "    END IF;",
    ]
    for child, fk in children:
        lines += child_lines(model, child, fk)
    lines += ["    DELETE FROM %s t WHERE %s;" % (table(t.name), pk_match("t", t)), "END"]
    return function("public.scd_purge_" + t.name, "\n".join(lines), "void", "plpgsql", "k jsonb")


def child_lines(model, child, fk):
    action = child_action(model, child, fk)
    where = "%s = %s" % (tup("c", fk["cols"]), tup("p", fk["ref_cols"]))
    if action == "delete":
        return ["    DELETE FROM %s c WHERE %s;" % (table(child.name), where)]
    if action == "recurse":
        return [
            "    FOR r IN SELECT %s FROM %s c WHERE %s LOOP" % (refs("c", child.pk), table(child.name), where),
            "        PERFORM public.scd_purge_%s(%s);" % (child.name, pk_json("r", child.pk)),
            "    END LOOP;",
        ]
    if action == "null":
        cols = fk["cols"]
        value = "NULL"
    elif action == "default":
        cols = fk["cols"]
        value = "DEFAULT"
    else:
        cols = [c for c in fk["cols"] if not child.notnull.get(c) and c not in child.pk]
        value = "NULL"
    sets = ", ".join("%s = %s" % (q(c), value) for c in cols)
    return ["    UPDATE %s c SET %s WHERE %s;" % (table(child.name), sets, where)]


def publications(model):
    rows = []
    for pub, name, cols in model.publication_lists():
        p = model.publications[pub]
        array = "ARRAY[%s]::text[]" % ", ".join(lit(c) for c in cols)
        rows.append("    (%s, %s, %s, %s, %s)" % (lit(pub), lit(p["node"]), lit(p["publish"]), lit(name), array))
    body = "VALUES\n" + ",\n".join(rows)
    return function(
        "public.scd_publication_lists",
        body,
        "TABLE (pubname text, node text, publish text, tbl text, cols text[])",
        "sql",
        "",
        " IMMUTABLE",
    )
