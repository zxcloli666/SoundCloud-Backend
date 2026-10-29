# Replication ownership

`ownership.yaml` says who owns every table and column. `gen.py` turns it into SQL in `generated/`. The SQL is checked in, and CI fails when it drifts.

## Classes

| class | meaning | replicated |
|---|---|---|
| C | core-owned; api writes only through `scd_wt` and a kind | core -> api, column list without `local`/`private` |
| P | core-private | no |
| M | intent group from api, result group from core, each with its own version | api -> core: PK + api groups; core -> api: full |
| A | any node writes; HLC row version; delete policy `tombstone`, `none` or `core` | both ways |
| K | max register (`register:`), core keeps the highest | api -> core |
| L | node-local | no |
| X | protocol tables (cmd_*, repl_*) with explicit `publish:` | as listed |

`schema.json` is the schema of the migrated databases. `model.py` checks `ownership.yaml` against it: every table classified, every non-PK unique key on M/A/K declared as `id` (R6) or `random`, every replicated column in a group, no NOT NULL column without a default left out of a publication list, FKs from replicated to unreplicated tables listed in `unpublished_fks`, every non-generated trigger listed in `derived`.

## Commands

```
python3 ops/replication/gen.py                 # regenerate generated/
python3 ops/replication/gen.py --check         # drift check
bash scripts/check-ownership.sh                # unit tests, drift, code against ownership.yaml
bash scripts/check-migrations.sh               # append-only + R12 rules on added migrations
```

After a migration changes the schema, regenerate `schema.json` against PG 17 with `wal_level=logical`:

```
podman run -d --name scdpg -e POSTGRES_USER=scd -e POSTGRES_PASSWORD=dev \
  mirror.gcr.io/library/postgres:17-alpine -c wal_level=logical
SCHEMA_UPDATE=1 PSQL="podman exec -i scdpg psql -U scd" ops/replication/tests/pg/run.sh
python3 ops/replication/gen.py
```

`run.sh` without `SCHEMA_UPDATE` is the PG proof: migrations, schema.json diff, install of the generated SQL twice with an empty search_path, `smoke.sql` per trigger class, then core + main + star as three databases on real logical replication.

## check-ownership rules

Code is scanned with `codemap.py` (Rust and SQL). utils crates count as api code when the api uses them. `core_only_code` paths are exempt.

| rule | fails on |
|---|---|
| api-write-c, api-write-p | api or streaming writes a C or P table |
| api-delete-shared | api deletes M rows or A rows whose delete policy is not `tombstone` (R7) |
| a-insert-only-updated | an `insert_only` A table is updated |
| clock | `nextval('catalog_metadata_clock')` |
| unknown-kind | `scd_wt('<kind>'` with a kind not in `kinds:` |
| kind-* | a `scd_cmd_<kind>` function in migrations uses SKIP LOCKED, FOR UPDATE, LIMIT, writes outside `touches`, or has no kind |

`baseline.txt` holds today's violations. CI fails on a new one and on a fixed one still listed: fix the code, then delete the line. `check_ownership.py --update-baseline` rewrites the file.

## Migration rules (R12, added files in api/migrations)

- DML only inside a DO block that checks `public.scd_role()`.
- No CASCADE except FK actions.
- DROP or RENAME of a published column only after `scd_unpublish_column('tbl', 'col')` in an earlier migration. No RENAME of a published table.
- No `nextval('catalog_metadata_clock')`.

## Generated files and install order

1. `tables.sql`: planned tables, version columns, planned columns.
2. B2 functions: `scd_role()`, `scd_mode()`, `scd_node()`, `scd_hlc(bigint)`, `scd_unpublish_column`, `scd_sync_publications`. `tests/pg/b2_stub.sql` is the test stand-in.
3. `triggers.sql`, `row_hash.sql`, `purge_children.sql`, `publications.sql`.

Every public object is schema-qualified (`public.`); the apply worker runs with an empty search_path.

| trigger | fires | tables | does |
|---|---|---|---|
| zz_ver | origin | M, versioned A | bumps a group version with `scd_hlc` when the group changes; on insert only for groups this node owns |
| zz_merge | replica, split | M, versioned A | keeps the higher version per group; insert on an existing PK or R6 natural key becomes a merge (R6 conflict counted) |
| zz_skip | replica, split | insert-only A | skips an insert on an existing PK |
| zz_tomb / zz_tomb_check | origin / replica, split | tombstone A | delete writes `repl_tombstones`; a late insert not newer than the tombstone is dropped |
| zz_owner | replica, api, split | C | skips keys in `cmd_pending`; purges local rows that hold core's unique keys; insert on an existing PK becomes an update |
| zz_purge_guard | replica, api, split | M | refuses a delete whose `repl_purges` receipt is older than the local api version, queues `repl_resend` |
| zz_max | replica, core, split | K | keeps the max register |
| zzz_suppress | replica | C, M, A, K | `suppress_redundant_updates_trigger` |
| zz_guard | origin statement, api | C, P | counts a write without `scd.cmd` |
| zz_guard_intent | origin statement, core | M api columns | counts a write unless `scd.writer` is mirror, migration, repl or api |
| zz_guard_result | origin statement, api | M core columns | counts a write unless `scd.writer` is migration or repl |

Guards only count (`public.scd_counters`) unless `scd.guard = 'enforce'`, then they raise.

Functions: `scd_row_hash(tbl, pk)` and `scd_row_hash_<tbl>(pk)` hash the replicated columns with fixed TimeZone, DateStyle, IntervalStyle, extra_float_digits and bytea_output. `scd_purge(tbl, pk)` and `scd_purge_<tbl>(pk)` delete a row and its FK children explicitly (cascades do not fire in apply): CASCADE children are deleted or recursed, SET NULL/DEFAULT applied, NO ACTION/RESTRICT children get their nullable FK columns cleared, else purged. `scd_publication_lists()` returns (pubname, node, publish, tbl, cols).

PK values are `jsonb_build_object` of the PK columns by name everywhere: `cmd_pending.pk`, `repl_tombstones.pk`, `repl_purges.pk`, `repl_resend.pk`, the hash and purge functions.

GUCs: `scd.role` core|api, `scd.node` core|main|star, `scd.mode` legacy|split (per database), `scd.cmd` (set by `scd_wt`), `scd.writer` mirror|migration|repl|api, `scd.guard` log|enforce.
