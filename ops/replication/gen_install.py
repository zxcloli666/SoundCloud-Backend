import glob
import os
import re

import gen_funcs
import gen_triggers
from sqlbits import function, lit

HEAD = "-- no-transaction\n"
LOCK = "SET LOCAL lock_timeout = '5s';\n"
UNIT = re.compile(r"^CREATE OR REPLACE TRIGGER (\S+) .* ON (\S+) FOR EACH .*;\n(?:ALTER TABLE \2 ENABLE REPLICA TRIGGER \1;\n)?", re.M)
KEPT = re.compile(r"^SELECT public\.scd_keep_trigger\('([^']*)', '([^']*)', (true|false), \$t\$(.*)\$t\$\);\n", re.M)


def render(m, previous=""):
    per, counters, expected = gen_triggers.collect(m)
    hashed = {t.name: t for t in gen_funcs.hashed(m)}
    purged = {t.name: t for t in gen_funcs.purge_tables(m)}
    receipts = {t.name: t for t in gen_funcs.receipt_tables(m)}
    blocks = [
        gen_triggers.sequences(counters) + "\n" + gen_triggers.guard_function() + "\n" + keep_function() + "\n"
        + gen_funcs.registry(m) + "\n" + gen_funcs.publications(m)
    ]
    for name in sorted(set(per) | set(hashed) | set(purged) | set(receipts)):
        parts = []
        if name in hashed:
            parts += [gen_funcs.row_hash_function(hashed[name]), gen_funcs.row_json_function(hashed[name])]
        if name in purged:
            parts.append(gen_funcs.purge_function(m, purged[name]))
        if name in receipts:
            parts.append(gen_funcs.receipt_function(receipts[name]))
        if name in per:
            parts.append(per[name])
        blocks.append("\n".join(parts))
    tables = list(hashed.values())
    blocks.append("\n".join([
        gen_funcs.row_hash_dispatcher(tables),
        gen_funcs.row_json_dispatcher(tables),
        gen_funcs.purge_dispatcher(list(purged.values())),
        gen_funcs.receipt_dispatcher(list(receipts.values())),
        gen_triggers.counters_view(counters),
    ]))
    blocks.append(gen_triggers.drop_stale(expected))
    text = HEAD + "\n".join("BEGIN;\n" + LOCK + b.rstrip("\n") + "\nCOMMIT;\n" for b in blocks)
    return keep_unchanged(text, previous)


def keep_function():
    body = "\n".join([
        "BEGIN",
        "    IF EXISTS (",
        "        SELECT 1",
        "        FROM pg_catalog.pg_trigger AS t",
        "        WHERE t.tgrelid = pg_catalog.to_regclass(rel)",
        "          AND t.tgname = trig",
        "          AND NOT t.tgisinternal",
        "          AND t.tgenabled = CASE WHEN replica THEN 'R' ELSE 'O' END",
        "    ) THEN",
        "        RETURN;",
        "    END IF;",
        "    EXECUTE ddl;",
        "    IF replica THEN",
        "        EXECUTE pg_catalog.format('ALTER TABLE %s ENABLE REPLICA TRIGGER %s', rel, trig);",
        "    END IF;",
        "END",
    ])
    return function("public.scd_keep_trigger", body, "void", "plpgsql", "rel text, trig text, replica boolean, ddl text")


def keep_unchanged(text, previous):
    known = {u.group(0) for u in UNIT.finditer(previous)}
    known |= {unit(rel, trig, replica == "true", create) for rel, trig, replica, create in KEPT.findall(previous)}

    def swap(u):
        if u.group(0) not in known:
            return u.group(0)
        create, _, alter = u.group(0).partition("\n")
        return "SELECT public.scd_keep_trigger(%s, %s, %s, $t$%s$t$);\n" % (
            lit(u.group(2)), lit(u.group(1)), "true" if alter else "false", create)

    return UNIT.sub(swap, text)


def unit(rel, trig, replica, create):
    text = create + "\n"
    if replica:
        text += "ALTER TABLE %s ENABLE REPLICA TRIGGER %s;\n" % (rel, trig)
    return text


def latest(migrations):
    found = sorted(glob.glob(os.path.join(migrations, "*_scd_install.sql")))
    return found[-1] if found else None


def previous(migrations, target):
    if target is None:
        return ""
    name = os.path.basename(target)
    older = [p for p in sorted(glob.glob(os.path.join(migrations, "*_scd_install.sql"))) if os.path.basename(p) < name]
    if not older:
        return ""
    with open(older[-1], encoding="utf-8", newline="") as f:
        return f.read()
