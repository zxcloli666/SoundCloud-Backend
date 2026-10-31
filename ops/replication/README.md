# Replication ownership

`ownership.yaml` says who owns every table and column. `gen.py` turns it into SQL in `generated/`. The SQL is checked in, and CI fails when it drifts.

## Classes

| class | meaning | replicated |
|---|---|---|
| C | core-owned; api writes only through `scd_wt` and a kind | core -> api, column list without `local`/`private` |
| P | core-private | no |
| M | intent groups from api, each with a version; result groups from core, no version (one writer) | api -> core: PK + api groups; core -> api: full |
| A | any node writes; HLC row version; delete policy `tombstone`, `none` or `core` | both ways |
| K | max register (`register:`), core keeps the highest | api -> core |
| L | node-local | no |
| X | protocol tables (cmd_*, repl_*) with explicit `publish:` | as listed |

`schema.json` is the schema of the migrated databases. `model.py` checks `ownership.yaml` against it: every table classified, every non-PK unique key on M/A/K declared as `id` (R6) or `random`, every replicated column in a group, no NOT NULL column without a default left out of a publication list, FKs from replicated to unreplicated tables listed in `unpublished_fks`, every non-generated trigger listed in `derived` with what it writes, ENABLE ALWAYS, gated by `WHEN (public.scd_derived())` and with a pinned search_path. `least:` lists immutable columns merged with `least()` when two nodes insert the same key (default `created_at` in a versioned group; an insert-only table lists them, e.g. `user_events.created_at`). A C table an optimistic kind touches declares each non-PK unique index as `key` (the value belongs to one primary key for good) or sets `reassert: always` (see Hold).

## Commands

```
python3 ops/replication/gen.py                 # regenerate generated/
python3 ops/replication/gen.py --check         # drift check, generated/ and the newest *_scd_install.sql
python3 ops/replication/gen.py --install api/migrations/NNNN_scd_install.sql
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

When generated SQL changes, ship it as a new `NNNN_scd_install.sql` (`--install`); an applied install migration is never edited.

`run.sh` without `SCHEMA_UPDATE` is the PG proof: migrations, schema.json diff, install of the generated SQL twice with an empty search_path, `smoke.sql` per trigger class, then core + main + star as three databases on real logical replication, and `split.sh`: dispatch, dead commands, purges, R6 and FK cases with row diffs per node pair.

## check-ownership rules

Code is scanned with `codemap.py` (Rust and SQL). utils crates count as api code when the api uses them. `core_only_code` paths are exempt.

| rule | fails on |
|---|---|
| api-write-c, api-write-p | api or streaming writes a C or P table |
| api-delete-shared | api deletes M rows or A rows whose delete policy is not `tombstone` (R7) |
| core-delete-m | jobs or other core code deletes an M row directly instead of `scd_purge_intent` (R7) |
| a-insert-only-updated | an `insert_only` A table is updated |
| clock | `nextval('catalog_metadata_clock')` |
| unknown-kind | `scd_wt('<kind>'` or a `const NAME` in `utils/write-through/src` with a kind not in `kinds:` |
| kind-* | a `scd_cmd_<kind>` function in migrations uses SKIP LOCKED, FOR UPDATE, LIMIT, writes outside `touches`, or has no kind |
| c-key-update | an UPDATE (or upsert DO UPDATE) sets a primary key column of a C table an optimistic kind touches |

`baseline.txt` holds today's violations. CI fails on a new one and on a fixed one still listed: fix the code, then delete the line. `check_ownership.py --update-baseline` rewrites the file.

## Migration rules (R12, added files in api/migrations)

- DML only inside a DO block that checks `public.scd_role()`.
- No CASCADE except FK actions.
- DROP or RENAME of a published column only after `scd_unpublish_column('tbl', 'col')` in an earlier migration. No RENAME of a published table.
- No `nextval('catalog_metadata_clock')`.

## Migrations and install order

1. `0174_scd_core_functions`: `scd_node()`, `scd_role()`, `scd_mode()` (unset = core, core, legacy), `scd_node_ord()`, `scd_hlc(bigint)`, `scd_clock()`, `scd_derived()`.
2. `0175_scd_tables`: cmd_* and repl_* tables, `scd_unpublished_columns`.
3. `0176`-`0199`: one version column add per table, `lock_timeout` 5s.
4. `0200_scd_install`: the generated SQL, one short transaction per table (`gen.py --install`).
5. `0201_scd_derived_always`: derived triggers ENABLE ALWAYS, gated, search_path pinned.
6. `0202_scd_dispatch`: `scd_wt`, `scd_dispatch`, the appliers, `scd_drain_resend()`.
7. `0203_scd_admin`: `scd_sync_publications()`, `scd_unpublish_column`, `scd_configure`, `scd_grant_repl`, `scd_repl_problems`.
8. `0204_scd_uuid_v5`: `uuid-ossp`; `scd_uuid(ns, VARIADIC key)` is UUID v5 of `ns:key1\x1fkey2...` under namespace `e5f6011f-72fa-46e1-ad97-55586f6f1f83`, equal to Rust `Uuid::new_v5` of the same bytes; NULL when a part is NULL.
9. `0205_scd_wt_output_race`: `scd_wt` returns `(id, output)`, kinds return `{touched, output}`; `scd_unique_race`; dispatch leaves a raced 23505 to the apply retry.
10. `0206_scd_install`: the generated SQL again: replica inserts merge inside the trigger, tombstones are versioned deletes (lab T3h).
11. `0207_scd_install`: the generated SQL again: `scd_purge_intent` takes the version the caller saw, zz_owner locks the row before it checks `cmd_pending`.
12. `0208_scd_cmd_purge_lock`: `scd_lock_row(tbl, pk)`; the cmd_purge applier locks the row before it checks `cmd_pending`.
13. `0209_scd_tombstones_version_index`: unique index on `repl_tombstones (tbl, pk, v)`, concurrently.
14. `0210_scd_tombstones_version_key`: that index becomes the primary key; one immutable row per delete.
15. `0211_scd_install`: the generated SQL again: a tombstone that arrives locks the live row, is written, then deletes what it covers (lab T3h deadlock, T9 lost slot); core columns (derived on core) stay out of the hash; `scd_resend_all()`; zz_owner applies a core row equal to the local one instead of raising 23505.
16. `0212_scd_install`: the generated SQL again: an M row at or below its `repl_purges` version is refused on every node and counted (`purge_block`); a local M write starts above it.
17. `0213_scd_resend_live_key_index`: index on `repl_resend` by live table and live key, concurrently.
18. `0214_scd_drain_resend_lock`: `scd_try_lock_row(tbl, pk)`; `scd_drain_resend()` locks the live row before it reads the image and skips a locked one, and drains in live-key order on that index, a tombstone next to its row (lab T3h divergence and core deadlocks).
19. `0220_scd_sweep`: `scd_drain_resend_counts(batch)`, `scd_sweep(batch)` (one runner per node); `scd_drain_resend()` keeps its contract.
20. `0221_scd_verify`: `scd_verify_probe`, `scd_verify_repair` and the probe, drop and resend ops in the `repl_repair` applier (B13).
21. `0222_scd_install`: the generated SQL again: a `least` column also merges down on an incoming row of the same version, so a resent image fixes a row this node re-inserted while it missed the first insert (lab T18).
22. `0256_scd_install`: the generated SQL again: zz_owner holds deletes and key moves of a pending key (Hold), `scd_reassert_always(tbl)`, `least` on insert-only tables (`user_events.created_at`).
23. `0257_scd_dispatch_reassert_always`: `scd_dispatch` reasserts every touched key of a `reassert: always` table that core has, equal hash or not.
24. `0264_kinds_catalog_misc`: kinds `track.too_long`, `track.release_held`, `track.s3_state` (ok before missing per track, in `sc_track_id` order, each key touched once; non-optimistic, so an api node in split only ships it and never locks the tracks rows core's batches write).
25. `0265_entity_reads_wake`: entity_reads_playlist_wake on `entity_reads` (below); api read stamps of tracks, users and playlists go to `entity_reads` instead of the catalog rows.
26. `0267_scd_install`: the generated SQL again: `playlists.last_read_at` is a core column (written only by entity_reads_playlist_wake), published by core and kept out of the hash; derived triggers may sit on K tables.

`jobs-migrate core` calls `scd_configure` and `scd_grant_repl` when `SCD_NODE`, `SCD_ROLE` and `SCD_MODE` are set, and refuses `SCD_MODE=split` while `scd_repl_problems()` returns rows. With the env unset nothing changes and the node stays legacy.

In legacy mode only zz_ver and the guards fire: versions are stamped and guards count. Every replica trigger and the dispatch need `scd.mode = split`.

Every public object is schema-qualified (`public.`); the apply worker runs with an empty search_path.

| trigger | fires | tables | does |
|---|---|---|---|
| zz_ver | origin | M, versioned A, repl_tombstones | bumps a group version with `scd_hlc` when the group changes; on insert only for groups this node owns; an insert into an LWW tombstone table starts above the newest local tombstone; an M api group starts or moves above the key's newest `repl_purges` version; a tombstone gets a fresh HLC above the key's newest, an UPDATE of `repl_tombstones` raises 0A000 |
| zz_merge | replica, split | M, versioned A | refuses an M row (insert or update) whose api version is at or below the key's newest `repl_purges` version and counts it, on every apply path (apply, `scd_apply_row` in repair and reassert, resend); keeps the higher version per versioned group, ties go to the incoming row on api and the stored row on core; core groups are taken from core as is; insert on an existing PK or R6 natural key becomes a merge (R6 conflict counted), `least` columns merge with least() for a row born on another node or of the same version (an image of this node's own row that another node merged down, lab T18) |
| zz_skip | replica, split | insert-only A | skips an insert on an existing PK; `least` columns merge down with least() on that conflict only, and an incoming update never raises them |
| zz_tomb | origin, split | tombstone A | BEFORE DELETE: writes `repl_tombstones` first, so the tombstone reaches a peer ahead of the key-only DELETE; its `v` is the delete's HLC |
| zz_tomb_check | replica, split | tombstone A | drops an insert the tombstone covers; LWW tables queue a resend of the tombstone to the sender |
| zz_tomb_guard | replica, split | LWW tombstone A | refuses a replicated DELETE of a row newer than the local tombstone and queues a resend of the row |
| zz_erase | replica, split | repl_tombstones | BEFORE an arriving tombstone: locks the live row (`scd_tomb_lock`), writes the tombstone (an echo is a no-op), deletes the rows it covers (`scd_erase_<t>`), so core forwards the tombstone ahead of its DELETE; a newer local row stays and is queued for resend |
| zz_core_col | origin, api, split | C with core columns | an api write of a core column keeps the stored value (`scd_core_col_<t>` counts it); the value only comes from core |
| zz_owner | replica, api, split | C | locks the key, then holds every incoming change of it while it is in `cmd_pending` (a local optimistic write in flight is waited for): insert, update (core columns still apply), a delete straight from the apply (trigger depth 1), a move onto it (the old row goes) and a move off it (the held row stays, the new key goes in); purges local rows that hold core's unique keys, held ones included; insert on an existing PK becomes an update; the row lock, not the update count, says the row exists, so a row equal to the local one (zzz_suppress skips the update) is a no-op |
| zz_purge_guard | replica, api, split | M | refuses a delete whose `repl_purges` receipt is older than the local api version, queues `repl_resend` |
| zz_max | replica, core, split | K | keeps the max register |
| entity_reads_playlist_wake | always, derived | entity_reads | AFTER a playlist read stamp: wakes a public `auth_required` playlist whose `playlists.last_read_at` is older than the read minus 6 hours (state row first), then moves that stamp to the read; on core in split (also for the max merge of zz_max), on every node in legacy (local writes only); an api node gets the wake and the stamp from core |
| zz_fk | replica, core, split | M/A parents of FKs | applies FK delete actions that apply skips; a RESTRICT child refuses the delete and queues `repl_resend` (`scd_restrict_<t>`) |
| zz_orphan | replica, core, split | M/A children of M/A parents | counts a row whose parent is missing (`scd_orphan_<t>`) |
| zzz_suppress | replica, split | C, M, A, K | `suppress_redundant_updates_trigger` |
| zz_guard | origin statement, api | C, P | counts a write without `scd.cmd` |
| zz_guard_intent | origin statement, core | M api columns, insert or update | counts a write unless `scd.writer` is mirror, migration, repl or api |
| zz_guard_result | origin statement, api | M core columns, update | counts a write unless `scd.writer` is migration or repl; on an M table an optimistic kind touches, also unless `scd.cmd` is set |

Guards only count (`public.scd_counters`) unless `scd.guard = 'enforce'`, then they raise.

Replica inserts (zz_merge, zz_skip, zz_max, zz_owner) never return the row to the apply. They look the key up, merge into a found row, else insert it themselves with `INSERT ... ON CONFLICT DO NOTHING` under `scd.merging = on` and return NULL. A concurrent apply or local writer of the same key makes the insert wait and do nothing; the next of 3 attempts finds the row and merges. A merge whose row is deleted under it updates nothing and retries, so the tombstone decides. After 3 attempts it raises 23505. Two apply workers on core and an apply against a local writer no longer kill the worker (lab T3h: 100 restarts in 600 s).

Tombstones (lab T3h). A replicated DELETE carries only the key, an UPDATE of a missing row is dropped, and a DELETE that finds no row on core writes nothing to forward. So a tombstone is a versioned delete:
- LWW (natural or R6 keys): a row lives when its version is above every tombstone of its key. The tombstone travels before the DELETE; zz_erase deletes covered rows and keeps newer ones, zz_tomb_guard refuses a DELETE that would kill a newer row, and both queue the newer row for resend. A row a node still sends below the tombstone is dropped by zz_tomb_check, which resends the tombstone back to that node. Every disagreement ends in a resend toward the higher version, so `scd_drain_resend()` must run on every node, core included.
- Delete-wins (a single uuid PK without an R6 `id:`, never reused: sessions, login_requests, link_requests, featured_items, listening_history): a tombstone deletes every row of its key and blocks every insert. A logout beats any concurrent touch.
- Rows are immutable, PK `(tbl, pk, v)`: every delete adds a row with a fresh HLC, the newest per key decides, core purges after 7 days. No tombstone UPDATE travels, so applying one never waits on a tombstone row a local delete holds.
- Lock order, the same on every path: the live row first, then `repl_tombstones`. A local DELETE locks the row in the executor and zz_tomb adds a new tombstone row. An arriving tombstone (zz_erase) locks the row with `scd_tomb_lock`, then writes the tombstone and erases. zz_tomb_guard and zz_merge run after the apply locked the row and only read tombstones. `scd_purge_intent` locks the M row before it writes `repl_purges`. No path holds a tombstone row while it waits for a live row (lab T3h: 11 deadlocks in 600 s before).
- Lock order for playlist and track rows (B5b, 0255): a transaction writes the `playlist_membership_state` row of a playlist before its other rows (a path that only needs the lock writes it as a no-op, because logical replication carries writes and not locks, so the apply worker on another node would take the rows in write order), then `playlists`, then `tracks` in key order, and `sync_queue` and owned rows last, while core finalize locks the state, playlist or track row before the queue row; operations and projection rows are written only under the state row.
- An arriving tombstone deletes the live row it covers whichever arrived first (LWW: `row_v <= v`; delete-wins: always). The delete runs in the apply under `scd.merging`, so zz_tomb (origin only) mints no new tombstone and nothing echoes beyond the tombstone itself. After a lost slot: new slot, then `SELECT public.scd_resend_all()` on the node whose changes were lost; the sweepers drain it (by hand: `scd_drain_resend()` on every node until it returns 0). `scd_resend_all()` queues every key this node publishes (api: M, A, K; core: C, M, A) and every tombstone (lab T9). It cannot resend a delete: a lost core->api slot leaves core's C deletes, M purges and deletes of untombstoned A rows on the api until verify removes them (below).
- REPLICA IDENTITY FULL was not used: a newer local row would turn every incoming UPDATE into update_missing (dropped), the WAL of big tables grows with full old tuples, and it gives no resend path.

Core columns: a column that a derived trigger computes on core only (`derived: {trigger: {writes: [tbl.col]}}` in `ownership.yaml`; today `artists.has_sc_account`, `lyrics_cache.fts`, `playlists.last_read_at`; `scd_core_columns()`; the trigger sits on a C or K table). An api kind cannot compute it, so the row hash leaves it out, zz_core_col keeps api writes off it, and core's value reaches the api nodes as a normal core write. A trigger that only nulls a column or drops a row is a veto (`vetoes:`, e.g. `tracks.primary_artist_id`, `track_artists`): core's answer differs from the api's only when the api tried a vetoed value, so the reassert is right. A trigger that writes rows of another table lists it under `rows:` (`lyrics_lookup_state`); no kind may touch both tables. `derived.py` reads each derived trigger's source from `schema.json` and fails the check when it writes a column or rows that are not declared.

Functions: `scd_row_hash(tbl, pk)` and `scd_row_hash_<tbl>(pk)` hash the replicated columns except core columns with fixed TimeZone, DateStyle, IntervalStyle, extra_float_digits and bytea_output. `scd_purge(tbl, pk)` and `scd_purge_<tbl>(pk)` delete a row and its FK children explicitly (cascades do not fire in apply): CASCADE children are deleted or recursed, SET NULL/DEFAULT applied, NO ACTION/RESTRICT children get their nullable FK columns cleared, else purged. `scd_purge_intent(tbl, pk, seen)` and `scd_purge_intent_<tbl>(pk, seen)` are the only way core purges an M row (R7): in one tx they lock the row, return false if its api version is not `seen` (the version the caller checked, e.g. `wanted_v`; a re-like in between changes it), else write `repl_purges` with that version and delete; on an api node they raise. `scd_row_json(tbl, pk)` returns the replicated columns as jsonb (cmd_reassert, repl_repair). `scd_resend(tbl)` queues every key of a table in `repl_resend`, `scd_resend_all()` every table this node publishes. `scd_kinds()` and `scd_table_class(tbl)` expose the registry. `scd_publication_lists()` returns (pubname, node, publish, tbl, cols); `scd_sync_publications()` applies it, intersected with existing columns and minus `scd_unpublished_columns`.

Kinds: `scd_cmd_<kind>(args jsonb) RETURNS jsonb` returns `{"touched": [{"tbl", "pk"}], "output": ...}`. A kind is deterministic: timestamps come from args, not `now()`, or core's row differs and gets reasserted. Anything other than an object is refused with 22023.

Optimistic kinds on M tables (today `connection.login` on `soundcloud_connections`). The api node runs the kind, core runs it again and its answer wins. On such a table the api zz_merge drops every incoming image of a row while the node has it in `cmd_pending`, so core rows written before the command cannot undo the local write; `cmd_done` comes first in core's transaction and releases the hold before core's image of the kind. So the kind must write every row it touches on core, also when it decides to change nothing. zz_guard_result lets the kind write core columns, and zz_ver keeps a group version that the kind raised above the stored one, so a kind can stamp a version it computed under the row lock and carries in args. Core compares that version with its own (`login_v` for logins), never the row image that came in the same transaction. There is no reassert for M rows: a dead kind leaves the api core columns until core writes the row again.

Hold (B5b). While a key is in `cmd_pending` the api row stays as the kind left it, so core's hash compare at dispatch is sound where every unique key is the PK. A row core writes at a key the api did not touch is never dropped: it purges a held row that holds its unique value, and a move off a held key inserts the new key. A purged held row comes back only through the dispatch:
- `key` uniques (`tracks`, `users`, `playlists`, `lyrics_lookup_state`): no writer deletes the row or moves the value to another key, so core's row at the held key can never carry it again, its hash differs and the dispatch reasserts or purges the key.
- `reassert: always` (`playlist_track_projection`, `playlist_membership_operations`, `artists`, `artist_sc_accounts`, `wanted_tracks`, `track_comments`): values move between keys (positions, sequences, merges), so core reasserts every touched key it has.
- A move off a key the api deleted or purged is an UPDATE of a missing row, dropped by the apply before any trigger; c-key-update keeps PK moves out of the code of these tables, writers delete and insert.

Dispatch: `SELECT id, output FROM scd_wt(kind, args)`. On core and in legacy mode it runs the kind in place. On an api node in split mode it runs an optimistic kind, then writes `cmd_pending` and `cmd_outbox`; a non-optimistic kind only ships (output NULL). On core the replica trigger on `cmd_outbox` runs the kind once per (node, id), writes `cmd_done` first, records a non-transient failure in `cmd_dead`, then compares touched C keys and answers with `cmd_reassert` or `cmd_purge`. `scd_drain_resend()` turns `repl_resend` rows into `repl_repair`. It locks each live row with SKIP LOCKED before it reads the image, so a local write waits for the image or the key waits for the next drain; a write that commits between the read and the commit would reach a peer first, meet no row there and be dropped, and the older image would win. Keys go in live table and live key order, tombstone keys by the key they delete, the same on every node, so two repair batches applied on core lock rows in one order. The receiver applies the image with `scd_apply_row`: insert if missing, the merge if present. It returns the keys it went through, skipped ones included, so 0 means the queue is empty.

Re-push and the dedup window (`0261`, on the `0257` body): a command core already has in `cmd_done` is acked again without running the kind: the `cmd_done` row is deleted and inserted with its old status and time, its `cmd_reassert` and `cmd_purge` rows are deleted and the touched check runs again against core's current rows. So re-pushing a pending row after a lost core->api slot releases its `cmd_pending` keys in the same transaction that carries the lost reasserts and purges. A command older than `scd_dedup_window()` (7 days) minus 1 hour is never run: it may have run before and lost its `cmd_done` row to retention, so it goes to `cmd_dead` and is replayed by hand only if core never ran it. A re-push re-inserts the api rows with their `created_at` (delete then insert on the api node, the publication is insert-only).

Retention (`0234`, `0261`). Core, jobs every 10 minutes, `scd_retention_batch`: `cmd_reassert` and `cmd_purge` go with their `cmd_done` row, `cmd_done` when it has no children left, `repl_purges` and `repl_tombstones` after 7 days, these five only while `scd_retention_guard()` is clear; `cmd_dead` and `storage_event_repairs` (older than the 2-day upload lookback anyway) after 7 days without the guard. `scd_dead_redact()` drops the token pair of a dead `connection.login` that no replay can install (the connection is gone or has that `login_v` or newer). Every node, the sweeper loop, `scd_node_retention_batch`: `cmd_outbox` rows done more than 2 days ago (clamped to 1-6 days, inside core's 7-day dedup), never a pending row; the token pair of a `connection.login` row 1 hour after its ack, or of a pending one past the dedup window; expired `oauth_app_tokens_local` rows after 1 hour. On an api node `scd_node_retention_guard()` holds it while the subscription from core is missing, disabled, without a worker, silent for 3 minutes or behind, or the slot core reads is missing, inactive or behind. Pending rows older than 2 days are reported as held (`pending`). One runner per node: the advisory lock `scd.retention.node`. Metrics: `scd_repl_retention_deleted_total{table}`, `scd_repl_retention_redacted_total{table}`, `scd_repl_retention_held{table, reason}`.

Race (lab 3b): `INSERT ... ON CONFLICT` checks only its arbiter; a non-arbiter unique index that meets a row inserted by a concurrent apply or a concurrent local backend (two first inserts of one track race on `tracks_pkey` or `tracks_urn_uq`) raises 23505. `scd_wt` finds the conflicting row from the error (schema, table, index, `Key (...)=(...)` detail), and when its xmin is not in the snapshot taken before the kind it re-raises 23505 with HINT `scd_wt: retry the transaction, a concurrent transaction inserted this key`; the retry takes the update path. Any other 23505 keeps no hint. Needs English `lc_messages`, else nothing is retried. Under REPEATABLE READ the check cannot see the new row, so nothing is retried. In dispatch the same raced 23505 rolls back the apply tx, the worker retries it, a real one goes to `cmd_dead`.

Client: `utils/write-through` (`wt` in api, jobs with `sqlx`, streaming with `tokio-postgres`). `wt::call(conn, &args)` is one `scd_wt` call and decodes the output into `Kind::Output`. `wt::retry(|| async { begin; call; commit })` runs the whole transaction again once when the error carries the race hint. Each `scd_wt` call is one subtransaction: batch rows into one call. `tests/real_pg` runs every kind on an api and a core database and compares outputs and row hashes; adding a kind there is a struct, an `impl Kind` and one `Pair::run`.

Sweeper: `scd_sweep(batch)` takes `pg_try_advisory_xact_lock(hashtextextended('scd.sweep', 0))` (else `busy`), drains up to `batch` keys and returns `state` (legacy, busy, ran), `sent`, `skipped` (row locked), `gone` (no row) and `queued` (capped at 100000); on an api node it also drops its verify answers older than a day and answers up to 10 held probes. `utils/repl-verify` `Sweeper` calls it in the api process on main and star and in jobs on core: every 2 s, at once again after a full batch that sent something, every 60 s in legacy mode, backoff on errors. Metrics: `scd_repl_resend_queued`, `scd_repl_resend_sent_total`, `scd_repl_resend_skipped_locked_total`, `scd_repl_resend_gone_total`.

Verify (core, jobs, `REPL_VERIFY=off|dry|repair`, default repair), for drift replication cannot heal:
- `scd_verify_probe(budget, size)` walks the C, M and A tables of `scd_core_pub` (not `repl_tombstones`) in PK order, `budget` chunks of `size` keys per call, cursor in `repl_checksums ('core', '*', 'cursor')`. Each chunk goes out as a `repl_repair` op: `pk = {lo, hi}`, `row = {scd_op: probe, n, h, attempt, cols}`, `h` is md5 over the md5 of each row's `to_jsonb` limited to `cols` (what core publishes, minus core columns), with fixed TimeZone and output styles.
- An api node runs the op in its apply worker, so it hashes its copy at the same point of core's stream. A different chunk is stored in its own `repl_checksums (node, tbl, probe id)` as `{lo, hi, attempt, n}` and queued in `repl_resend`; the sweeper ships it to core as a K row. While `repl_resend` holds keys of that table the node holds the probe (`repl_checksums (node, tbl, 'wait <probe id>')`, never replicated) and its sweeper answers it once those keys are shipped; that answer replicates to core by itself, behind the resent rows.
- Core answers such a chunk (`recheck`) with an exact probe, attempt 1: the same range plus core's `{pk: md5}` at that point of its stream. The api answers it with only the keys that differ there, `{pk: its md5 or null}`. So a row written on any node after a probe is never taken for drift.
- `scd_verify_repair(dry, answers)` takes each exact answer, skips a key whose current core row now equals the api's, and acts by class on the rest. Core has the key: its image, plus a `resend` request for M and A so a newer api version comes back. Only the api has it: C `drop` (skipped while the key is in `cmd_pending`); M `drop` with core's `repl_purges` receipts (zz_purge_guard keeps a newer re-like and resends it), `resend` when there is no receipt; tombstone A the newest tombstone, else `resend`; other A `drop` (only core deletes them). After a repair the chunk gets an exact probe again, up to attempt 3; a key that still differs then is unresolved. Dry mode sends probes but no repair and reports what it would do.
- Under lag: an answer travels behind every api write it hashed (one subscription, commit order), so core has those rows when it settles.
- Every repair is logged. Metrics: `scd_repl_verify_probes_total`, `scd_repl_verify_cycles_total`, `scd_repl_verify_rechecks_total{peer,tbl}`, `scd_repl_verify_repairs_total{peer,tbl,action,dry}`, `scd_repl_verify_unresolved_total{peer,tbl}`.
- Cost: default 10 chunks of 500 rows every 5 s, an index range scan per chunk on core and on each api node; about 30 h per cycle for main's ~110M rows.
- CLI (`cargo build --features cli`, `DATABASE_URL` of the node): `repl-verify verify [--dry-run] --once` probes every table once and prints the report; `repl-verify sweep --once` drains a node.

PK values are `jsonb_build_object` of the PK columns by name everywhere: `cmd_pending.pk`, `repl_tombstones.pk`, `repl_purges.pk`, `repl_resend.pk`, the hash and purge functions.

GUCs: `scd.role` core|api, `scd.node` core|main|star, `scd.mode` legacy|split (per database), `scd.cmd` (set by `scd_wt`), `scd.writer` mirror|migration|repl|api, `scd.guard` log|enforce.
