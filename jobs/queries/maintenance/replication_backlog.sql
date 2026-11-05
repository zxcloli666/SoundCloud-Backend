SELECT COALESCE(max(pg_wal_lsn_diff(pg_current_wal_lsn(), COALESCE(slot.confirmed_flush_lsn, slot.restart_lsn))), 0)::bigint
FROM pg_replication_slots AS slot
WHERE (slot.slot_type = 'physical' OR slot.database = current_database())
  AND slot.slot_name !~ '_graph(_|$)'
  AND slot.active
  AND slot.wal_status IS DISTINCT FROM 'lost'
  AND COALESCE(slot.confirmed_flush_lsn, slot.restart_lsn) IS NOT NULL
