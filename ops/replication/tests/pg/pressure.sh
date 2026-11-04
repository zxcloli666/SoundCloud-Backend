wp_query="$(cat "$root/jobs/queries/queue/write_pressure.sql")"
wp_backlog() { val postgres "$wp_query" | cut -d'|' -f1; }
wp_lag() { val postgres "SELECT pg_catalog.pg_wal_lsn_diff(pg_catalog.pg_current_wal_lsn(), confirmed_flush_lsn)::bigint FROM pg_catalog.pg_replication_slots WHERE slot_name = '$1'"; }
wp_slot() { val postgres "SELECT pg_catalog.pg_create_logical_replication_slot('$1', 'pgoutput')" > /dev/null; }
wp_wal() { val postgres "SELECT pg_catalog.pg_logical_emit_message(false, 'scd_b1_pressure', repeat('x', $1))" > /dev/null; }

say "write pressure: the bulk governor counts the scd slots and skips the graph slots"
wp_slot scd_b1_pressure_graph_from_core
wp_slot scd_b1_from_pressure_graph
wp_wal 4194304
want "graph slots of both name styles are no replication backlog" "$(wp_backlog)" 0
wp_slot scd_b1_pressure_from_core
wp_wal 65536
wp_counted="$(wp_backlog)"
wp_scd="$(wp_lag scd_b1_pressure_from_core)"
wp_graph="$(wp_lag scd_b1_pressure_graph_from_core)"
[ "$wp_counted" -gt 0 ] && [ "$wp_counted" -le "$wp_scd" ] && [ "$wp_counted" -lt "$wp_graph" ] \
  || { echo "FAIL write pressure counted $wp_counted bytes, the scd slot lags $wp_scd, the graph slots $wp_graph"; exit 1; }
echo "ok the scd slot is the backlog ($wp_counted bytes), the graph slots behind it are not ($wp_graph bytes)"
for s in scd_b1_pressure_graph_from_core scd_b1_from_pressure_graph scd_b1_pressure_from_core; do
  val postgres "SELECT pg_catalog.pg_drop_replication_slot('$s')" > /dev/null
done
