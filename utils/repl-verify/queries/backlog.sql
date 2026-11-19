SELECT COALESCE(max(pg_wal_lsn_diff(pg_current_wal_lsn(), slot.confirmed_flush_lsn)), 0)::bigint AS "backlog!"
FROM pg_replication_slots AS slot
WHERE slot.slot_type = 'logical'
  AND slot.database = current_database()
  AND slot.slot_name !~ '_graph(_|$)'
  AND slot.slot_name !~ '^pg_[0-9]+_sync_[0-9]+_'
