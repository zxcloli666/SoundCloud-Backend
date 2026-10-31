from model import purge_action
from sqlbits import function, lit, pk_json, pk_match, q, ref, refs, table, tup

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
    tables = hashed(model)
    return "\n".join([row_hash_function(t) for t in tables] + [row_hash_dispatcher(tables)])


def row_hash_function(t):
    cols = [c for c in t.replicated if c not in t.core_cols]
    body = "SELECT md5(ROW(%s)::text) FROM %s t WHERE %s" % (refs("t", cols), table(t.name), pk_match("t", t))
    return function("public.scd_row_hash_" + t.name, body, "text", "sql", "k jsonb", " STABLE" + HASH_SETTINGS)


def row_hash_dispatcher(tables):
    return dispatcher("public.scd_row_hash", "text", [(t.name, "RETURN public.scd_row_hash_%s(k);" % t.name) for t in tables], " STABLE")


def row_json(model):
    tables = hashed(model)
    return "\n".join([row_json_function(t) for t in tables] + [row_json_dispatcher(tables)])


def row_json_dispatcher(tables):
    return dispatcher("public.scd_row_json", "jsonb", [(t.name, "RETURN public.scd_row_json_%s(k);" % t.name) for t in tables], " STABLE")


def row_json_function(t):
    hidden = [c for c in t.columns if c not in t.replicated]
    expr = "pg_catalog.to_jsonb(t)"
    if hidden:
        expr += " - ARRAY[%s]::text[]" % ", ".join(lit(c) for c in hidden)
    body = "SELECT %s FROM %s t WHERE %s" % (expr, table(t.name), pk_match("t", t))
    return function("public.scd_row_json_" + t.name, body, "jsonb", "sql", "k jsonb", " STABLE" + HASH_SETTINGS)


def registry(model):
    kinds = ",\n".join("    (%s, %s)" % (lit(k), "true" if spec.get("optimistic", True) else "false") for k, spec in sorted(model.kinds.items()))
    classes = ",\n".join("    (%s, %s)" % (lit(t.name), lit(t.cls)) for t in model.installed())
    cores = ",\n".join("    (%s, %s)" % (lit(t.name), lit(c)) for t in model.installed() for c in t.core_cols)
    return "\n".join([
        function("public.scd_kinds", "VALUES\n" + kinds, "TABLE (kind text, optimistic boolean)", "sql", "", " STABLE"),
        function("public.scd_table_class", "SELECT v.cls FROM (VALUES\n%s\n) AS v (tbl, cls) WHERE v.tbl = $1" % classes,
                 "text", "sql", "tbl text", " STABLE"),
        function("public.scd_core_columns", "VALUES\n" + cores, "TABLE (tbl text, col text)", "sql", "", " STABLE"),
        resend(model),
    ])


def resend(model):
    tables = [t for t in hashed(model) if t.cls in ("C", "M", "A", "K")]
    cases = [(t.name, "INSERT INTO public.repl_resend (tbl, pk) SELECT %s, %s FROM %s t ON CONFLICT DO NOTHING;" % (
        lit(t.name), pk_json("t", t.pk), table(t.name))) for t in tables]
    rows = ",\n".join("        (%s, %s)" % (lit(t.name), lit(t.cls)) for t in tables if not t.hold)
    body = "\n".join([
        "DECLARE",
        "    sent text[] := CASE public.scd_role() WHEN 'core' THEN ARRAY['C', 'M', 'A'] ELSE ARRAY['M', 'A', 'K'] END;",
        "    r record;",
        "BEGIN",
        "    FOR r IN SELECT v.tbl FROM (VALUES",
        rows,
        "    ) AS v (tbl, cls) WHERE v.cls = ANY (sent) ORDER BY v.tbl LOOP",
        "        PERFORM public.scd_resend(r.tbl);",
        "    END LOOP;",
        "    RETURN (SELECT count(*) FROM public.repl_resend);",
        "END",
    ])
    return dispatcher("public.scd_resend", "void", cases, args="tbl text") + function("public.scd_resend_all", body, "bigint")


def dispatcher(name, returns, cases, extra="", args="tbl text, k jsonb"):
    lines = ["BEGIN", "    CASE tbl"]
    for tbl, stmt in cases:
        lines += ["        WHEN %s THEN" % lit(tbl), "            " + stmt]
    lines += ["        ELSE", "            RAISE EXCEPTION '%s: unknown table %%', tbl;" % name, "    END CASE;"]
    if returns == "void":
        lines.append("    RETURN;")
    lines.append("END")
    return function(name, "\n".join(lines), returns, "plpgsql", args, extra)


def purge_tables(model):
    roots = [t for t in model.installed() if t.cls in ("C", "P") and t.pk]
    roots += [c for p in model.fk_parents() for c, fk in model.children(p.name) if fk["on_delete"] == "c" and model.children(c.name)]
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
    tables = purge_tables(model)
    receipts = receipt_tables(model)
    return "\n".join(
        [purge_function(model, t) for t in tables] + [purge_dispatcher(tables)]
        + [receipt_function(t) for t in receipts] + [receipt_dispatcher(receipts)]
    )


def purge_dispatcher(tables):
    return dispatcher("public.scd_purge", "void", [(t.name, "PERFORM public.scd_purge_%s(k);" % t.name) for t in tables])


def receipt_tables(model):
    return [t for t in model.installed() if t.cls == "M" and t.pk and t.api_groups()]


def receipt_dispatcher(tables):
    cases = [(t.name, "RETURN public.scd_purge_intent_%s(k, seen);" % t.name) for t in tables]
    drop = "DROP FUNCTION IF EXISTS public.scd_purge_intent(text, jsonb);\n"
    return drop + dispatcher("public.scd_purge_intent", "boolean", cases, args="tbl text, k jsonb, seen bigint")


def receipt_function(t):
    versions = [ref("t", g["version"]) for _, g in t.api_groups()]
    newest = versions[0] if len(versions) == 1 else "greatest(%s)" % ", ".join(versions)
    body = "\n".join([
        "DECLARE",
        "    v bigint;",
        "    key jsonb;",
        "BEGIN",
        "    IF public.scd_role() <> 'core' THEN",
        "        RAISE EXCEPTION 'scd_purge_intent: only core purges %s' USING ERRCODE = 'insufficient_privilege';" % table(t.name),
        "    END IF;",
        "    SELECT %s, %s INTO v, key FROM %s t WHERE %s FOR UPDATE;" % (newest, pk_json("t", t.pk), table(t.name), pk_match("t", t)),
        "    IF NOT FOUND OR v IS DISTINCT FROM seen THEN",
        "        RETURN false;",
        "    END IF;",
        "    INSERT INTO public.repl_purges (tbl, pk, purged_v) VALUES (%s, key, v);" % lit(t.name),
        "    DELETE FROM %s t WHERE %s;" % (table(t.name), pk_match("t", t)),
        "    RETURN true;",
        "END",
    ])
    drop = "DROP FUNCTION IF EXISTS public.scd_purge_intent_%s(jsonb);\n" % t.name
    return drop + function("public.scd_purge_intent_" + t.name, body, "boolean", "plpgsql", "k jsonb, seen bigint")


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
