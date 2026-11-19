cs_key='{"subject_id": 900000001, "relation": 1, "user_id": 7}'
cs_rows() { only "$1" "SELECT count(*) FROM public.catalog_audience_ids WHERE subject_id IN (900000001, 900000002)"; }
cs_has() { only "$1" "SELECT count(*) FROM public.catalog_audience_ids WHERE (subject_id, relation, user_id) = ($2, 1, $3)"; }
cs_deleted() { only "$1" "SELECT n_tup_del FROM pg_catalog.pg_stat_user_tables WHERE relid = 'public.catalog_audience_ids'::regclass"; }

say "C copy: a tablesync into an empty api table skips the per-row lookups and drops the copied rows held in cmd_pending"
fresh scd_b1_cs_core scd_b1_tpl
sql -d scd_b1_cs_core -c "INSERT INTO public.catalog_audience_ids (subject_id, relation, user_id, created_at) SELECT 900000001, 1, u, now() FROM generate_series(1, 3000) u" > /dev/null
sql -d scd_b1_cs_core -c "CREATE PUBLICATION scd_b1_cs_pub FOR TABLE public.catalog_audience_ids" > /dev/null
for n in cs_a cs_b; do
  fresh "scd_b1_$n" scd_b1_tpl
  val "scd_b1_$n" "SELECT count(*) FROM public.scd_configure('main', 'api', 'split')" > /dev/null
  sql -d "scd_b1_$n" -c "INSERT INTO public.cmd_pending (tbl, pk, node, id) VALUES ('catalog_audience_ids', '$cs_key', 'main', '00000000-0000-0000-0000-0000000c5001')" > /dev/null
done
sql -d scd_b1_cs_b -c "SET scd.writer = 'migration'" -c "INSERT INTO public.catalog_audience_ids (subject_id, relation, user_id, created_at) VALUES (900000002, 1, 1, now())" > /dev/null
for n in cs_a cs_b; do
  val scd_b1_cs_core "SELECT pg_catalog.pg_create_logical_replication_slot('scd_b1_${n}_from_core', 'pgoutput')" > /dev/null
  sql -d "scd_b1_$n" -c "CREATE SUBSCRIPTION ${n}_from_core CONNECTION 'host=$socket dbname=scd_b1_cs_core user=scd' PUBLICATION scd_b1_cs_pub WITH (create_slot = false, slot_name = 'scd_b1_${n}_from_core', copy_data = true)" > /dev/null
  wait_for "scd_b1_$n" "SELECT bool_and(srsubstate = 'r') FROM pg_catalog.pg_subscription_rel" "the copy into $n is ready"
done
want "the empty table takes every copied row but the held one" "$(cs_rows cs_a) $(cs_has cs_a 900000001 7)" "2999 0"
wait_for scd_b1_cs_a "SELECT n_tup_del = 1 FROM pg_catalog.pg_stat_user_tables WHERE relid = 'public.catalog_audience_ids'::regclass" "the fast copy inserted the held row and its statement trigger deleted it"
want "the table with a local row takes the per-row path" "$(cs_rows cs_b) $(cs_has cs_b 900000001 7) $(cs_has cs_b 900000002 1)" "3000 0 1"
want "the per-row path never inserts the held row" "$(cs_deleted cs_b)" 0

say "C copy: after the copy the apply keeps holding pending keys"
sql -d scd_b1_cs_a -c "INSERT INTO public.cmd_pending (tbl, pk, node, id) VALUES ('catalog_audience_ids', '{\"subject_id\": 900000001, \"relation\": 1, \"user_id\": 5000}', 'main', '00000000-0000-0000-0000-0000000c5002')" > /dev/null
sql -d scd_b1_cs_core -c "INSERT INTO public.catalog_audience_ids (subject_id, relation, user_id, created_at) VALUES (900000001, 1, 5000, now()), (900000001, 1, 5001, now())" > /dev/null
wait_for scd_b1_cs_a "SELECT EXISTS (SELECT 1 FROM public.catalog_audience_ids WHERE (subject_id, relation, user_id) = (900000001, 1, 5001))" "the apply delivers a new row after the copy"
want "the apply holds the pending key" "$(cs_has cs_a 900000001 5000)" 0

for n in cs_a cs_b; do
  sql -d "scd_b1_$n" -c "ALTER SUBSCRIPTION ${n}_from_core DISABLE" -c "ALTER SUBSCRIPTION ${n}_from_core SET (slot_name = NONE)" -c "DROP SUBSCRIPTION ${n}_from_core" > /dev/null
  wait_for scd_b1_cs_core "SELECT NOT active FROM pg_catalog.pg_replication_slots WHERE slot_name = 'scd_b1_${n}_from_core'" "the source slot for $n is idle"
  val scd_b1_cs_core "SELECT pg_catalog.pg_drop_replication_slot('scd_b1_${n}_from_core')" > /dev/null
done
for d in scd_b1_cs_a scd_b1_cs_b scd_b1_cs_core; do
  sql -d postgres -c "DROP DATABASE $d WITH (FORCE)" > /dev/null
done
