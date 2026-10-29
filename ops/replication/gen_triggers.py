from sqlbits import assign, function, indent, keys_tuple, index_pred, lit, pk_json, q, ref, same, table, tup

GENERATED = (
    "zz_guard", "zz_guard_intent", "zz_guard_result", "zz_max", "zz_merge", "zz_owner",
    "zz_purge_guard", "zz_skip", "zz_tomb", "zz_tomb_check", "zz_ver", "zzz_suppress",
)
SPLIT = "public.scd_mode() = 'split'"
API = "public.scd_role() = 'api'"
CORE = "public.scd_role() = 'core'"


def render(model):
    parts = [guard_function()]
    counters = []
    expected = []
    for t in model.installed():
        if not t.exists and not t.spec.get("columns"):
            continue
        for name, sql, seqs in table_triggers(model, t):
            parts.append(sql)
            expected.append((t.name, name))
            counters += seqs
    parts.insert(0, "".join("CREATE SEQUENCE IF NOT EXISTS public.%s;\n" % seq for _, _, seq in sorted(counters)))
    parts.append(counters_view(sorted(counters)))
    parts.append(drop_stale(sorted(expected)))
    return "\n".join(parts)


def table_triggers(model, t):
    out = []
    if t.cls in ("C", "P"):
        out.append(guard(t, "zz_guard", "", API, "scd_guard_" + t.name, ["cmd", "migration", "repl"], "guard"))
    if t.cls == "C":
        out.append(owner(t))
    if t.cls == "M":
        intent = t.group_cols(("api",))
        result = t.group_cols(("core",))
        if intent:
            out.append(guard(t, "zz_guard_intent", " OF " + ", ".join(q(c) for c in intent), CORE,
                             "scd_guard_intent_" + t.name, ["mirror", "migration", "repl", "api"], "guard_intent"))
        if result:
            out.append(guard(t, "zz_guard_result", " OF " + ", ".join(q(c) for c in result), API,
                             "scd_guard_result_" + t.name, ["migration", "repl"], "guard_result"))
        out.append(purge_guard(t))
    if t.cls in ("M", "A") and t.versioned:
        out.append(ver(t))
        out.append(merge(t))
    if t.cls == "A" and t.insert_only:
        out.append(skip(t))
    if t.cls == "A" and t.delete == "tombstone":
        out.append(tomb(t))
        out.append(tomb_check(t))
    if t.cls == "K":
        out.append(register(t))
    if t.cls in ("C", "M", "A", "K"):
        out.append(("zzz_suppress", suppress(t), []))
    return sorted(out, key=lambda x: x[0])


def trigger(t, name, timing, fn, when=None, row=True, replica=False):
    sql = "CREATE OR REPLACE TRIGGER %s %s ON %s FOR EACH %s" % (name, timing, table(t.name), "ROW" if row else "STATEMENT")
    if when:
        sql += " WHEN (%s)" % when
    sql += " EXECUTE FUNCTION %s;\n" % fn
    if replica:
        sql += "ALTER TABLE %s ENABLE REPLICA TRIGGER %s;\n" % (table(t.name), name)
    return sql


def fn_name(kind, t):
    return "public.scd_%s_%s" % (kind, t.name)


def guard_function():
    body = "\n".join([
        "BEGIN",
        "    IF 'cmd' = ANY (TG_ARGV) AND coalesce(current_setting('scd.cmd', true), '') <> '' THEN",
        "        RETURN NULL;",
        "    END IF;",
        "    IF coalesce(current_setting('scd.writer', true), '') = ANY (TG_ARGV[1:]) THEN",
        "        RETURN NULL;",
        "    END IF;",
        "    PERFORM nextval(TG_ARGV[0]::regclass);",
        "    IF current_setting('scd.guard', true) = 'enforce' THEN",
        "        RAISE EXCEPTION 'scd guard: % on public.% by writer %', TG_OP, TG_TABLE_NAME,",
        "            coalesce(nullif(current_setting('scd.writer', true), ''), 'none')",
        "            USING ERRCODE = 'insufficient_privilege';",
        "    END IF;",
        "    RETURN NULL;",
        "END",
    ])
    return function("public.scd_guard", body)


def guard(t, name, of, when, seq, writers, counter):
    events = "BEFORE UPDATE" + of if of else "BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE"
    args = ", ".join(lit(a) for a in ["public." + seq] + writers)
    return name, trigger(t, name, events, "public.scd_guard(%s)" % args, when, row=False), [(counter, t.name, seq)]


def ver(t):
    lines = ["BEGIN", "    IF TG_OP = 'INSERT' THEN"]
    for gname, g in sorted(t.groups.items()):
        bump = "        NEW.%s := public.scd_hlc(NEW.%s);" % (q(g["version"]), q(g["version"]))
        cond = owns(g["owner"])
        if cond:
            lines += ["        IF %s THEN" % cond, "    " + bump, "        END IF;"]
        else:
            lines.append(bump)
    lines += ["        RETURN NEW;", "    END IF;"]
    for gname, g in sorted(t.groups.items()):
        v = q(g["version"])
        lines += [
            "    IF %s IS DISTINCT FROM %s THEN" % (tup("NEW", g["cols"]), tup("OLD", g["cols"])),
            "        NEW.%s := public.scd_hlc(greatest(OLD.%s, NEW.%s));" % (v, v, v),
            "    ELSE",
            "        NEW.%s := greatest(OLD.%s, NEW.%s);" % (v, v, v),
            "    END IF;",
        ]
    lines += ["    RETURN NEW;", "END"]
    sql = function(fn_name("ver", t), "\n".join(lines))
    sql += trigger(t, "zz_ver", "BEFORE INSERT OR UPDATE", fn_name("ver", t) + "()")
    return "zz_ver", sql, []


def owns(owner):
    if owner == "api":
        return "public.scd_role() = 'api' OR current_setting('scd.writer', true) = 'api'"
    if owner == "core":
        return "public.scd_role() = 'core' AND coalesce(current_setting('scd.writer', true), '') <> 'api'"
    return None


def merge(t):
    groups = sorted(t.groups.items())
    all_cols = [c for _, g in groups for c in g["cols"] + [g["version"]]]
    newer = " OR ".join("NEW.%s > o.%s" % (q(g["version"]), q(g["version"])) for _, g in groups)
    lines = ["DECLARE", "    o record;", "BEGIN", "    IF TG_OP = 'UPDATE' THEN"]
    for _, g in groups:
        v = q(g["version"])
        lines += ["        IF NEW.%s <= OLD.%s THEN" % (v, v)]
        lines += ["            NEW.%s := OLD.%s;" % (q(c), q(c)) for c in g["cols"] + [g["version"]]]
        lines += ["        END IF;"]
    lines += [
        "        RETURN NEW;",
        "    END IF;",
        "    SELECT %s INTO o FROM %s t WHERE %s;" % (", ".join(ref("t", g["version"]) for _, g in groups), table(t.name), same("t", "NEW", t.pk)),
        "    IF FOUND THEN",
        "        IF %s THEN" % newer,
        "            UPDATE %s t SET %s WHERE %s;" % (table(t.name), assign(all_cols, "NEW"), same("t", "NEW", t.pk)),
        "        END IF;",
        "        RETURN NULL;",
        "    END IF;",
    ]
    seqs = []
    for name, how in sorted(t.unique_decl.items()):
        u = t.unique_index(name)
        if how != "id" or u is None:
            continue
        seq = "scd_conflict_" + t.name
        seqs = [("conflict", t.name, seq)]
        pred_new = index_pred(u, t.columns, "NEW")
        pred_t = index_pred(u, t.columns, "t")
        where = "%s = %s" % (keys_tuple(u, t.columns, "t"), keys_tuple(u, t.columns, "NEW"))
        if pred_t:
            where += " AND " + pred_t
        body = [
            "SELECT %s INTO o FROM %s t WHERE %s;" % (", ".join(ref("t", c) for c in t.pk), table(t.name), where),
            "IF FOUND THEN",
            "    PERFORM nextval('public.%s'::regclass);" % seq,
            "    UPDATE %s t SET %s WHERE %s;" % (table(t.name), assign(all_cols, "NEW"), same("t", "o", t.pk)),
            "    RETURN NULL;",
            "END IF;",
        ]
        if pred_new:
            body = ["IF %s THEN" % pred_new] + indent(body) + ["END IF;"]
        lines += indent(body)
    lines += ["    RETURN NEW;", "END"]
    sql = function(fn_name("merge", t), "\n".join(lines))
    sql += trigger(t, "zz_merge", "BEFORE INSERT OR UPDATE", fn_name("merge", t) + "()", SPLIT, replica=True)
    return "zz_merge", sql, seqs


def skip(t):
    body = "\n".join([
        "BEGIN",
        "    PERFORM 1 FROM %s t WHERE %s;" % (table(t.name), same("t", "NEW", t.pk)),
        "    IF FOUND THEN",
        "        RETURN NULL;",
        "    END IF;",
        "    RETURN NEW;",
        "END",
    ])
    sql = function(fn_name("skip", t), body)
    sql += trigger(t, "zz_skip", "BEFORE INSERT", fn_name("skip", t) + "()", SPLIT, replica=True)
    return "zz_skip", sql, []


def row_version(t):
    return t.groups["row"]["version"] if "row" in t.groups else None


def tomb(t):
    v = row_version(t)
    body = "\n".join([
        "BEGIN",
        "    INSERT INTO public.repl_tombstones (tbl, pk, v, at)",
        "    VALUES (%s, %s, %s, now())" % (lit(t.name), pk_json("OLD", t.pk), "OLD." + q(v) if v else "0"),
        "    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;",
        "    RETURN NULL;",
        "END",
    ])
    sql = function(fn_name("tomb", t), body)
    sql += trigger(t, "zz_tomb", "AFTER DELETE", fn_name("tomb", t) + "()", SPLIT)
    return "zz_tomb", sql, []


def tomb_check(t):
    v = row_version(t)
    blocked = "FOUND AND NEW.%s <= tv" % q(v) if v else "FOUND"
    body = "\n".join([
        "DECLARE",
        "    tv bigint;",
        "BEGIN",
        "    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = %s AND x.pk = %s;" % (lit(t.name), pk_json("NEW", t.pk)),
        "    IF %s THEN" % blocked,
        "        RETURN NULL;",
        "    END IF;",
        "    RETURN NEW;",
        "END",
    ])
    sql = function(fn_name("tomb_check", t), body)
    sql += trigger(t, "zz_tomb_check", "BEFORE INSERT", fn_name("tomb_check", t) + "()", SPLIT, replica=True)
    return "zz_tomb_check", sql, []


def owner(t):
    keys = [u for u in t.uniques]
    lines = ["DECLARE", "    r record;"]
    lines += ["    k%d boolean := true;" % i for i in range(len(keys))]
    lines += [
        "BEGIN",
        "    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = %s AND p.pk = %s) THEN" % (lit(t.name), pk_json("NEW", t.pk)),
        "        RETURN NULL;",
        "    END IF;",
    ]
    if keys:
        lines.append("    IF TG_OP = 'UPDATE' THEN")
        for i, u in enumerate(keys):
            lines.append("        k%d := %s IS DISTINCT FROM %s;" % (i, keys_tuple(u, t.columns, "NEW", True), keys_tuple(u, t.columns, "OLD", True)))
        lines.append("    END IF;")
    for i, u in enumerate(keys):
        pred_new = index_pred(u, t.columns, "NEW")
        pred_t = index_pred(u, t.columns, "t")
        cond = "k%d" % i + (" AND " + pred_new if pred_new else "")
        where = "%s = %s" % (keys_tuple(u, t.columns, "t"), keys_tuple(u, t.columns, "NEW"))
        if pred_t:
            where += " AND " + pred_t
        where += " AND %s IS DISTINCT FROM %s" % (tup("t", t.pk), tup("NEW", t.pk))
        lines += [
            "    IF %s THEN" % cond,
            "        FOR r IN SELECT %s FROM %s t WHERE %s LOOP" % (", ".join(ref("t", c) for c in t.pk), table(t.name), where),
            "            PERFORM %s(%s);" % (fn_name("purge", t), pk_json("r", t.pk)),
            "        END LOOP;",
            "    END IF;",
        ]
    cols = [c for c in t.replicated if c not in t.pk]
    lines.append("    IF TG_OP = 'INSERT' THEN")
    if cols:
        lines += [
            "        UPDATE %s t SET %s WHERE %s;" % (table(t.name), assign(cols, "NEW"), same("t", "NEW", t.pk)),
            "        IF FOUND THEN",
        ]
    else:
        lines.append("        IF EXISTS (SELECT 1 FROM %s t WHERE %s) THEN" % (table(t.name), same("t", "NEW", t.pk)))
    lines += ["            RETURN NULL;", "        END IF;", "    END IF;", "    RETURN NEW;", "END"]
    sql = function(fn_name("owner", t), "\n".join(lines))
    sql += trigger(t, "zz_owner", "BEFORE INSERT OR UPDATE", fn_name("owner", t) + "()", API + " AND " + SPLIT, replica=True)
    return "zz_owner", sql, []


def purge_guard(t):
    versions = [q(g["version"]) for _, g in t.api_groups()]
    newest = "OLD." + versions[0] if len(versions) == 1 else "greatest(%s)" % ", ".join("OLD." + v for v in versions)
    seq = "scd_resend_" + t.name
    body = "\n".join([
        "DECLARE",
        "    pv bigint;",
        "BEGIN",
        "    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = %s AND x.pk = %s;" % (lit(t.name), pk_json("OLD", t.pk)),
        "    IF %s > coalesce(pv, 0) THEN" % newest,
        "        INSERT INTO public.repl_resend (tbl, pk) VALUES (%s, %s) ON CONFLICT DO NOTHING;" % (lit(t.name), pk_json("OLD", t.pk)),
        "        PERFORM nextval('public.%s'::regclass);" % seq,
        "        RETURN NULL;",
        "    END IF;",
        "    RETURN OLD;",
        "END",
    ])
    sql = function(fn_name("purge_guard", t), body)
    sql += trigger(t, "zz_purge_guard", "BEFORE DELETE", fn_name("purge_guard", t) + "()", API + " AND " + SPLIT, replica=True)
    return "zz_purge_guard", sql, [("resend", t.name, seq)]


def register(t):
    r = q(t.register)
    cols = [c for c in t.replicated if c not in t.pk]
    body = "\n".join([
        "BEGIN",
        "    IF TG_OP = 'INSERT' THEN",
        "        UPDATE %s t SET %s WHERE %s AND (t.%s IS NULL OR t.%s < NEW.%s);" % (table(t.name), assign(cols, "NEW"), same("t", "NEW", t.pk), r, r, r),
        "        IF FOUND THEN",
        "            RETURN NULL;",
        "        END IF;",
        "        PERFORM 1 FROM %s t WHERE %s;" % (table(t.name), same("t", "NEW", t.pk)),
        "        IF FOUND THEN",
        "            RETURN NULL;",
        "        END IF;",
        "        RETURN NEW;",
        "    END IF;",
        "    IF OLD.%s IS NOT NULL AND (NEW.%s IS NULL OR NEW.%s <= OLD.%s) THEN" % (r, r, r, r),
        "        RETURN NULL;",
        "    END IF;",
        "    RETURN NEW;",
        "END",
    ])
    sql = function(fn_name("max", t), body)
    sql += trigger(t, "zz_max", "BEFORE INSERT OR UPDATE", fn_name("max", t) + "()", CORE + " AND " + SPLIT, replica=True)
    return "zz_max", sql, []


def suppress(t):
    return trigger(t, "zzz_suppress", "BEFORE UPDATE", "pg_catalog.suppress_redundant_updates_trigger()", replica=True)


def counters_view(counters):
    rows = ",\n".join("    (%s, %s, %s)" % (lit(c), lit(tbl), lit(seq)) for c, tbl, seq in counters)
    return "\n".join([
        "CREATE OR REPLACE VIEW public.scd_counters AS",
        "SELECT c.counter, c.tbl, coalesce(s.last_value, 0) AS n",
        "FROM (VALUES",
        rows,
        ") AS c (counter, tbl, seq)",
        "LEFT JOIN pg_catalog.pg_sequences s ON s.schemaname = 'public' AND s.sequencename = c.seq;",
        "",
    ])


def drop_stale(expected):
    rows = ",\n".join("            (%s, %s)" % (lit(tbl), lit(name)) for tbl, name in expected)
    names = ", ".join(lit(n) for n in GENERATED)
    return "\n".join([
        "DO $d$",
        "DECLARE",
        "    r record;",
        "BEGIN",
        "    FOR r IN",
        "        SELECT c.relname::text AS tbl, t.tgname::text AS name",
        "        FROM pg_catalog.pg_trigger t",
        "        JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid",
        "        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace",
        "        WHERE n.nspname = 'public' AND NOT t.tgisinternal AND t.tgname::text IN (%s)" % names,
        "        EXCEPT VALUES",
        rows,
        "        ORDER BY 1, 2",
        "    LOOP",
        "        EXECUTE format('DROP TRIGGER %I ON public.%I', r.name, r.tbl);",
        "    END LOOP;",
        "END",
        "$d$;",
        "",
    ])
