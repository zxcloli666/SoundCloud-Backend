import glob
import os

import gen_funcs
import gen_triggers

HEAD = "-- no-transaction\n"
LOCK = "SET LOCAL lock_timeout = '5s';\n"


def render(m):
    per, counters, expected = gen_triggers.collect(m)
    hashed = {t.name: t for t in gen_funcs.hashed(m)}
    purged = {t.name: t for t in gen_funcs.purge_tables(m)}
    receipts = {t.name: t for t in gen_funcs.receipt_tables(m)}
    blocks = [
        gen_triggers.sequences(counters) + "\n" + gen_triggers.guard_function() + "\n" + gen_funcs.registry(m)
        + "\n" + gen_funcs.publications(m)
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
    return HEAD + "\n".join("BEGIN;\n" + LOCK + b.rstrip("\n") + "\nCOMMIT;\n" for b in blocks)


def latest(migrations):
    found = sorted(glob.glob(os.path.join(migrations, "*_scd_install.sql")))
    return found[-1] if found else None
