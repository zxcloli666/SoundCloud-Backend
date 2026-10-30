from sqlbits import assign, function, indent, keys_tuple, index_pred, lit, pk_json, pk_match, q, ref, refs, same, table, tup

GENERATED = (
    "zz_erase", "zz_fk", "zz_guard", "zz_guard_intent", "zz_guard_result", "zz_max", "zz_merge", "zz_orphan",
    "zz_owner", "zz_purge_guard", "zz_skip", "zz_tomb", "zz_tomb_check", "zz_tomb_guard", "zz_ver", "zzz_suppress",
)
SPLIT = "public.scd_mode() = 'split'"
API = "public.scd_role() = 'api'"
CORE = "public.scd_role() = 'core'"


def render(model):
    per, counters, expected = collect(model)
    parts = [sequences(counters), guard_function()] + [per[n] for n in sorted(per)]
    parts += [counters_view(counters), drop_stale(expected)]
    return "\n".join(parts)


def collect(model):
    per = {}
    counters = []
    expected = []
    for t in model.installed():
        if not t.exists and not t.spec.get("columns"):
            continue
        sqls = []
        for name, sql, seqs in table_triggers(model, t):
            sqls.append(sql)
            expected.append((t.name, name))
            counters += seqs
        if sqls:
            per[t.name] = "\n".join(sqls)
    return per, sorted(set(counters)), sorted(expected)


def sequences(counters):
    return "".join("CREATE SEQUENCE IF NOT EXISTS public.%s;\n" % seq for _, _, seq in counters)


def table_triggers(model, t):
    out = []
    if t.cls in ("C", "P"):
        out.append(guard(t, "zz_guard", "BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE", API, "scd_guard_" + t.name,
                         ["cmd", "migration", "repl"], "guard"))
    if t.cls == "C":
        out.append(owner(t))
    if t.cls == "M":
        intent = t.group_cols(("api",))
        result = t.group_cols(("core",))
        if intent:
            out.append(guard(t, "zz_guard_intent", "BEFORE INSERT OR UPDATE OF " + ", ".join(q(c) for c in intent), CORE,
                             "scd_guard_intent_" + t.name, ["mirror", "migration", "repl", "api"], "guard_intent"))
        if result:
            out.append(guard(t, "zz_guard_result", "BEFORE UPDATE OF " + ", ".join(q(c) for c in result), API,
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
        if lww(t):
            out.append(tomb_guard(t))
    if t.name == "repl_tombstones":
        out.append(erase(model, t))
    if t.cls == "K":
        out.append(register(t))
    if t in model.fk_parents():
        out.append(fk_guard(model, t))
    if model.orphan_fks(t):
        out.append(orphan(model, t))
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


def guard(t, name, events, when, seq, writers, counter):
    args = ", ".join(lit(a) for a in ["public." + seq] + writers)
    return name, trigger(t, name, events, "public.scd_guard(%s)" % args, when, row=False), [(counter, t.name, seq)]


def ver(t):
    groups = t.versioned_groups()
    lines = ["BEGIN", "    IF TG_OP = 'INSERT' THEN"]
    for gname, g in groups:
        prev = "NEW." + q(g["version"])
        if lww(t) and g["version"] == row_version(t):
            prev = "greatest(%s, (SELECT x.v FROM public.repl_tombstones x WHERE x.tbl = %s AND x.pk = %s))" % (prev, lit(t.name), pk_json("NEW", t.pk))
        bump = "        NEW.%s := public.scd_hlc(%s);" % (q(g["version"]), prev)
        cond = owns(g["owner"])
        if cond:
            lines += ["        IF %s THEN" % cond, "    " + bump, "        END IF;"]
        else:
            lines.append(bump)
    lines += ["        RETURN NEW;", "    END IF;"]
    for gname, g in groups:
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
    core_cols = [c for _, g in groups if not g["version"] for c in g["cols"]]
    tombs = t.cls == "A" and t.delete == "tombstone"
    lines = ["DECLARE", "    o record;", "    core boolean := public.scd_role() = 'core';", "    n bigint;"]
    if tombs:
        lines.append("    tv bigint;")
    lines += ["BEGIN"]
    lines += ["    IF current_setting('scd.merging', true) = 'on' THEN", "        RETURN NEW;", "    END IF;"]
    lines += ["    IF TG_OP = 'UPDATE' THEN"]
    for _, g in groups:
        if not g["version"]:
            continue
        v = q(g["version"])
        lines += ["        IF NEW.%s < OLD.%s OR (core AND NEW.%s = OLD.%s) THEN" % (v, v, v, v)]
        lines += ["            NEW.%s := OLD.%s;" % (q(c), q(c)) for c in g["cols"] + [g["version"]]]
        lines += ["        END IF;"]
    if core_cols:
        lines += ["        IF core THEN"]
        lines += ["            NEW.%s := OLD.%s;" % (q(c), q(c)) for c in core_cols]
        lines += ["        END IF;"]
    lines += indent(erase_unchanged(t, "OLD"), 2)
    lines += ["        RETURN NEW;", "    END IF;"]
    body = ["SELECT t.* INTO o FROM %s t WHERE %s;" % (table(t.name), same("t", "NEW", t.pk)), "IF FOUND THEN"]
    body += indent(merge_into(t) + erase_unchanged(t, "o"))
    body += ["    RETURN NULL;", "END IF;"]
    seqs = []
    for name, how in sorted(t.unique_decl.items()):
        u = t.unique_index(name)
        if how != "id" or u is None:
            continue
        seq = "scd_conflict_" + t.name
        seqs.append(("conflict", t.name, seq))
        pred_new = index_pred(u, t.columns, "NEW")
        pred_t = index_pred(u, t.columns, "t")
        where = "%s = %s" % (keys_tuple(u, t.columns, "t"), keys_tuple(u, t.columns, "NEW"))
        if pred_t:
            where += " AND " + pred_t
        found = [
            "SELECT t.* INTO o FROM %s t WHERE %s;" % (table(t.name), where),
            "IF FOUND THEN",
            "    PERFORM nextval('public.%s'::regclass);" % seq,
        ]
        found += indent(merge_into(t))
        found += ["    RETURN NULL;", "END IF;"]
        if pred_new:
            found = ["IF %s THEN" % pred_new] + indent(found) + ["END IF;"]
        body += found
    if tombs:
        blocked, more = tomb_block(t)
        body += blocked
        seqs += more
    body += insert_self(t)
    lines += ["    FOR attempt IN 1..3 LOOP"] + indent(body, 2) + ["    END LOOP;"]
    lines += indent(give_up(t)) + ["END"]
    sql = function(fn_name("merge", t), "\n".join(lines))
    sql += trigger(t, "zz_merge", "BEFORE INSERT OR UPDATE", fn_name("merge", t) + "()", SPLIT, replica=True)
    return "zz_merge", sql, seqs


def merge_into(t):
    conds = []
    sets = []
    for _, g in sorted(t.groups.items()):
        if not g["version"]:
            cols = g["cols"]
            conds.append("(NOT core AND %s IS DISTINCT FROM %s)" % (tup("NEW", cols), tup("o", cols)))
            sets += ["%s = CASE WHEN core THEN o.%s ELSE NEW.%s END" % (q(c), q(c), q(c)) for c in cols]
            continue
        v = q(g["version"])
        newer = "NEW.%s > o.%s" % (v, v)
        conds.append(newer)
        born = "(NEW.%s & 15) <> public.scd_node_ord()" % v
        for col in g["cols"]:
            c = q(col)
            if col in t.least:
                conds.append("(%s AND NEW.%s < o.%s)" % (born, c, c))
                sets.append("%s = CASE WHEN %s THEN least(o.%s, NEW.%s) WHEN %s THEN NEW.%s ELSE o.%s END" % (c, born, c, c, newer, c, c))
            else:
                sets.append("%s = CASE WHEN %s THEN NEW.%s ELSE o.%s END" % (c, newer, c, c))
        sets.append("%s = greatest(o.%s, NEW.%s)" % (v, v, v))
    return [
        "IF %s THEN" % " OR ".join(conds),
        "    PERFORM pg_catalog.set_config('scd.merging', 'on', true);",
        "    UPDATE %s t SET" % table(t.name),
    ] + ["        %s%s" % (x, "," if i < len(sets) - 1 else "") for i, x in enumerate(sets)] + [
        "    WHERE %s;" % same("t", "o", t.pk),
        "    GET DIAGNOSTICS n = ROW_COUNT;",
        "    PERFORM pg_catalog.set_config('scd.merging', '', true);",
        "    IF n = 0 THEN",
        "        CONTINUE;",
        "    END IF;",
        "END IF;",
    ]


def insert_self(t, counted=True):
    cols = [c for c in t.columns if c not in t.generated and c not in t.added]
    lines = [
        "PERFORM pg_catalog.set_config('scd.merging', 'on', true);",
        "INSERT INTO %s (%s)" % (table(t.name), ", ".join(q(c) for c in cols)),
        "VALUES (%s)" % refs("NEW", cols),
        "ON CONFLICT DO NOTHING;",
    ]
    if counted:
        lines.append("GET DIAGNOSTICS n = ROW_COUNT;")
    lines.append("PERFORM pg_catalog.set_config('scd.merging', '', true);")
    if counted:
        lines += ["IF n > 0 THEN", "    RETURN NULL;", "END IF;"]
    return lines


def give_up(t):
    return ["RAISE EXCEPTION 'scd: insert into %s still conflicts after 3 attempts' USING ERRCODE = 'unique_violation';" % table(t.name)]


def resend(tbl, pk, seq):
    return [
        "INSERT INTO public.repl_resend (tbl, pk) VALUES (%s, %s) ON CONFLICT DO NOTHING;" % (tbl, pk),
        "PERFORM nextval('public.%s'::regclass);" % seq,
    ]


def skip(t):
    tombs = t.cls == "A" and t.delete == "tombstone"
    lines = ["DECLARE", "    tv bigint;", "BEGIN"] if tombs else ["BEGIN"]
    lines += [
        "    IF current_setting('scd.merging', true) = 'on' THEN",
        "        RETURN NEW;",
        "    END IF;",
        "    PERFORM 1 FROM %s t WHERE %s;" % (table(t.name), same("t", "NEW", t.pk)),
        "    IF FOUND THEN",
        "        RETURN NULL;",
        "    END IF;",
    ]
    seqs = []
    if tombs:
        blocked, seqs = tomb_block(t)
        lines += indent(blocked)
    lines += indent(insert_self(t, counted=False)) + ["    RETURN NULL;", "END"]
    sql = function(fn_name("skip", t), "\n".join(lines))
    sql += trigger(t, "zz_skip", "BEFORE INSERT", fn_name("skip", t) + "()", SPLIT, replica=True)
    return "zz_skip", sql, seqs


def row_version(t):
    return t.groups["row"]["version"] if "row" in t.groups else None


def lww(t):
    return t.cls == "A" and t.delete == "tombstone" and bool(row_version(t)) and not t.delete_wins()


def tomb_tables(model):
    return [x for x in model.installed() if x.cls == "A" and x.delete == "tombstone" and (x.exists or x.spec.get("columns"))]


def tomb(t):
    v = row_version(t)
    body = "\n".join([
        "BEGIN",
        "    INSERT INTO public.repl_tombstones (tbl, pk, v, at)",
        "    VALUES (%s, %s, %s, now())" % (lit(t.name), pk_json("OLD", t.pk), "OLD." + q(v) if v else "0"),
        "    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;",
        "    RETURN OLD;",
        "END",
    ])
    sql = function(fn_name("tomb", t), body)
    sql += trigger(t, "zz_tomb", "BEFORE DELETE", fn_name("tomb", t) + "()", SPLIT)
    sql += erase_rows(t)
    return "zz_tomb", sql, [("resend", t.name, "scd_resend_" + t.name)] if lww(t) else []


def erase_rows(t):
    where = pk_match("t", t)
    lines = [
        "BEGIN",
        "    PERFORM pg_catalog.set_config('scd.merging', 'on', true);",
        "    DELETE FROM %s t WHERE %s%s;" % (table(t.name), where, " AND t.%s <= upto" % q(row_version(t)) if lww(t) else ""),
        "    PERFORM pg_catalog.set_config('scd.merging', '', true);",
    ]
    if lww(t):
        lines += ["    IF EXISTS (SELECT 1 FROM %s t WHERE %s) THEN" % (table(t.name), where)]
        lines += indent(resend(lit(t.name), "k", "scd_resend_" + t.name), 2)
        lines += ["    END IF;"]
    lines.append("END")
    return function(fn_name("erase", t), "\n".join(lines), "void", "plpgsql", "k jsonb, upto bigint")


def tomb_block(t):
    key = pk_json("NEW", t.pk)
    lines = ["SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = %s AND x.pk = %s;" % (lit(t.name), key)]
    if not lww(t):
        return lines + ["IF FOUND THEN", "    RETURN NULL;", "END IF;"], []
    seq = "scd_resend_" + t.name
    lines += ["IF FOUND AND NEW.%s <= tv THEN" % q(row_version(t))]
    lines += indent(resend("'repl_tombstones'", "jsonb_build_object('tbl', %s, 'pk', %s)" % (lit(t.name), key), seq))
    lines += ["    RETURN NULL;", "END IF;"]
    return lines, [("resend", t.name, seq)]


def tomb_check(t):
    blocked, seqs = tomb_block(t)
    body = "\n".join(["DECLARE", "    tv bigint;", "BEGIN"] + indent(blocked) + ["    RETURN NEW;", "END"])
    sql = function(fn_name("tomb_check", t), body)
    sql += trigger(t, "zz_tomb_check", "BEFORE INSERT", fn_name("tomb_check", t) + "()", SPLIT, replica=True)
    return "zz_tomb_check", sql, seqs


def tomb_guard(t):
    key = pk_json("OLD", t.pk)
    seq = "scd_resend_" + t.name
    lines = [
        "DECLARE",
        "    tv bigint;",
        "BEGIN",
        "    IF current_setting('scd.merging', true) = 'on' THEN",
        "        RETURN OLD;",
        "    END IF;",
        "    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = %s AND x.pk = %s;" % (lit(t.name), key),
        "    IF NOT FOUND OR OLD.%s <= tv THEN" % q(row_version(t)),
        "        RETURN OLD;",
        "    END IF;",
    ]
    lines += indent(resend(lit(t.name), key, seq)) + ["    RETURN NULL;", "END"]
    sql = function(fn_name("tomb_guard", t), "\n".join(lines))
    sql += trigger(t, "zz_tomb_guard", "BEFORE DELETE", fn_name("tomb_guard", t) + "()", SPLIT, replica=True)
    return "zz_tomb_guard", sql, [("resend", t.name, seq)]


def erase(model, t):
    lines = ["BEGIN", "    CASE tbl"]
    for x in tomb_tables(model):
        lines += ["        WHEN %s THEN" % lit(x.name), "            PERFORM %s(k, upto);" % fn_name("erase", x)]
    lines += ["        ELSE", "            NULL;", "    END CASE;", "END"]
    sql = function("public.scd_erase", "\n".join(lines), "void", "plpgsql", "tbl text, k jsonb, upto bigint")
    body = "\n".join(["BEGIN", "    PERFORM public.scd_erase(NEW.tbl, NEW.pk, NEW.v);", "    RETURN NULL;", "END"])
    sql += function(fn_name("erase", t), body)
    sql += trigger(t, "zz_erase", "AFTER INSERT OR UPDATE", fn_name("erase", t) + "()", SPLIT, replica=True)
    return "zz_erase", sql, []


def erase_unchanged(t, stored):
    if t.name != "repl_tombstones":
        return []
    return ["IF NEW.v <= %s.v THEN" % stored, "    PERFORM public.scd_erase(NEW.tbl, NEW.pk, %s.v);" % stored, "END IF;"]


def owner_purges(t, keys, flag):
    lines = []
    for i, u in enumerate(keys):
        pred_new = index_pred(u, t.columns, "NEW")
        pred_t = index_pred(u, t.columns, "t")
        conds = (["k%d" % i] if flag else []) + ([pred_new] if pred_new else [])
        where = "%s = %s" % (keys_tuple(u, t.columns, "t"), keys_tuple(u, t.columns, "NEW"))
        if pred_t:
            where += " AND " + pred_t
        where += " AND %s IS DISTINCT FROM %s" % (tup("t", t.pk), tup("NEW", t.pk))
        loop = [
            "FOR r IN SELECT %s FROM %s t WHERE %s LOOP" % (", ".join(ref("t", c) for c in t.pk), table(t.name), where),
            "    PERFORM %s(%s);" % (fn_name("purge", t), pk_json("r", t.pk)),
            "END LOOP;",
        ]
        lines += ["IF %s THEN" % " AND ".join(conds)] + indent(loop) + ["END IF;"] if conds else loop
    return lines


def owner(t):
    keys = list(t.uniques)
    cols = [c for c in t.replicated if c not in t.pk]
    lines = ["DECLARE", "    r record;", "    n bigint;"]
    lines += ["    k%d boolean;" % i for i in range(len(keys))]
    lines += [
        "BEGIN",
        "    IF current_setting('scd.merging', true) = 'on' THEN",
        "        RETURN NEW;",
        "    END IF;",
        "    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = %s AND p.pk = %s) THEN" % (lit(t.name), pk_json("NEW", t.pk)),
        "        RETURN NULL;",
        "    END IF;",
        "    IF TG_OP = 'UPDATE' THEN",
    ]
    for i, u in enumerate(keys):
        lines.append("        k%d := %s IS DISTINCT FROM %s;" % (i, keys_tuple(u, t.columns, "NEW", True), keys_tuple(u, t.columns, "OLD", True)))
    lines += indent(owner_purges(t, keys, True), 2)
    lines += ["        RETURN NEW;", "    END IF;"]
    body = owner_purges(t, keys, False)
    if cols:
        body += ["UPDATE %s t SET %s WHERE %s;" % (table(t.name), assign(cols, "NEW"), same("t", "NEW", t.pk)), "IF FOUND THEN"]
    else:
        body += ["IF EXISTS (SELECT 1 FROM %s t WHERE %s) THEN" % (table(t.name), same("t", "NEW", t.pk))]
    body += ["    RETURN NULL;", "END IF;"] + insert_self(t)
    lines += ["    FOR attempt IN 1..3 LOOP"] + indent(body, 2) + ["    END LOOP;"]
    lines += indent(give_up(t)) + ["END"]
    sql = function(fn_name("owner", t), "\n".join(lines))
    sql += trigger(t, "zz_owner", "BEFORE INSERT OR UPDATE", fn_name("owner", t) + "()", API + " AND " + SPLIT, replica=True)
    return "zz_owner", sql, []


def purge_guard(t):
    versions = [q(g["version"]) for _, g in t.api_groups()]
    newest = "OLD." + versions[0] if len(versions) == 1 else "greatest(%s)" % ", ".join("OLD." + v for v in versions)
    seq = "scd_resend_" + t.name
    lines = [
        "DECLARE",
        "    pv bigint;",
        "BEGIN",
        "    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = %s AND x.pk = %s;" % (lit(t.name), pk_json("OLD", t.pk)),
        "    IF %s > coalesce(pv, 0) THEN" % newest,
    ]
    lines += indent(resend(lit(t.name), pk_json("OLD", t.pk), seq), 2)
    lines += ["        RETURN NULL;", "    END IF;", "    RETURN OLD;", "END"]
    sql = function(fn_name("purge_guard", t), "\n".join(lines))
    sql += trigger(t, "zz_purge_guard", "BEFORE DELETE", fn_name("purge_guard", t) + "()", API + " AND " + SPLIT, replica=True)
    return "zz_purge_guard", sql, [("resend", t.name, seq)]


def register(t):
    r = q(t.register)
    cols = [c for c in t.replicated if c not in t.pk]
    body = [
        "UPDATE %s t SET %s WHERE %s AND (t.%s IS NULL OR t.%s < NEW.%s);" % (table(t.name), assign(cols, "NEW"), same("t", "NEW", t.pk), r, r, r),
        "IF FOUND THEN",
        "    RETURN NULL;",
        "END IF;",
        "PERFORM 1 FROM %s t WHERE %s;" % (table(t.name), same("t", "NEW", t.pk)),
        "IF FOUND THEN",
        "    RETURN NULL;",
        "END IF;",
    ] + insert_self(t)
    lines = [
        "DECLARE",
        "    n bigint;",
        "BEGIN",
        "    IF TG_OP = 'INSERT' THEN",
        "        IF current_setting('scd.merging', true) = 'on' THEN",
        "            RETURN NEW;",
        "        END IF;",
        "        FOR attempt IN 1..3 LOOP",
    ] + indent(body, 3) + ["        END LOOP;"] + indent(give_up(t), 2) + [
        "    END IF;",
        "    IF OLD.%s IS NOT NULL AND (NEW.%s IS NULL OR NEW.%s <= OLD.%s) THEN" % (r, r, r, r),
        "        RETURN NULL;",
        "    END IF;",
        "    RETURN NEW;",
        "END",
    ]
    sql = function(fn_name("max", t), "\n".join(lines))
    sql += trigger(t, "zz_max", "BEFORE INSERT OR UPDATE", fn_name("max", t) + "()", CORE + " AND " + SPLIT, replica=True)
    return "zz_max", sql, []


def suppress(t):
    return trigger(t, "zzz_suppress", "BEFORE UPDATE", "pg_catalog.suppress_redundant_updates_trigger()", SPLIT, replica=True)


def fk_guard(model, t):
    refuse = []
    actions = []
    for child, fk in model.children(t.name):
        where = "%s = %s" % (tup("c", fk["cols"]), tup("OLD", fk["ref_cols"]))
        action = fk["on_delete"]
        if action in ("a", "r"):
            refuse.append("EXISTS (SELECT 1 FROM %s c WHERE %s)" % (table(child.name), where))
        elif action == "c" and model.children(child.name):
            actions += [
                "    FOR r IN SELECT %s FROM %s c WHERE %s LOOP" % (", ".join(ref("c", c) for c in child.pk), table(child.name), where),
                "        PERFORM %s(%s);" % (fn_name("purge", child), pk_json("r", child.pk)),
                "    END LOOP;",
            ]
        elif action == "c":
            actions.append("    DELETE FROM %s c WHERE %s;" % (table(child.name), where))
        else:
            value = "NULL" if action == "n" else "DEFAULT"
            sets = ", ".join("%s = %s" % (q(c), value) for c in fk["cols"])
            actions.append("    UPDATE %s c SET %s WHERE %s;" % (table(child.name), sets, where))
    seq = "scd_restrict_" + t.name
    lines = ["DECLARE", "    r record;", "BEGIN"] if any("FOR r IN" in a for a in actions) else ["BEGIN"]
    if refuse:
        lines += ["    IF %s THEN" % " OR ".join(refuse)]
        lines += indent(resend(lit(t.name), pk_json("OLD", t.pk), seq), 2)
        lines += ["        RETURN NULL;", "    END IF;"]
    lines += actions + ["    RETURN OLD;", "END"]
    sql = function(fn_name("fk", t), "\n".join(lines))
    sql += trigger(t, "zz_fk", "BEFORE DELETE", fn_name("fk", t) + "()", CORE + " AND " + SPLIT, replica=True)
    return "zz_fk", sql, [("restrict", t.name, seq)] if refuse else []


def orphan(model, t):
    fks = model.orphan_fks(t)
    seq = "scd_orphan_" + t.name
    lines = ["BEGIN"]
    for fk in fks:
        present = " AND ".join("NEW.%s IS NOT NULL" % q(c) for c in fk["cols"])
        where = "%s = %s" % (tup("p", fk["ref_cols"]), tup("NEW", fk["cols"]))
        lines += [
            "    IF %s AND NOT EXISTS (SELECT 1 FROM %s p WHERE %s) THEN" % (present, table(fk["ref_table"]), where),
            "        PERFORM nextval('public.%s'::regclass);" % seq,
            "    END IF;",
        ]
    lines += ["    RETURN NEW;", "END"]
    cols = sorted({c for fk in fks for c in fk["cols"]})
    sql = function(fn_name("orphan", t), "\n".join(lines))
    sql += trigger(t, "zz_orphan", "BEFORE INSERT OR UPDATE OF " + ", ".join(q(c) for c in cols), fn_name("orphan", t) + "()",
                   CORE + " AND " + SPLIT, replica=True)
    return "zz_orphan", sql, [("orphan", t.name, seq)]


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
