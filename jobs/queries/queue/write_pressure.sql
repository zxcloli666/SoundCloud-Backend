SELECT COALESCE((
           SELECT max(pg_wal_lsn_diff(pg_current_wal_lsn(), slot.confirmed_flush_lsn))
           FROM pg_replication_slots AS slot
           WHERE slot.slot_type = 'logical'
       ), 0)::bigint AS replication_backlog_bytes,
       (
           SELECT count(*)
           FROM pg_stat_activity AS backend
           WHERE backend.state = 'active'
             AND backend.backend_type = 'client backend'
       ) AS active_backends,
       pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0')::bigint AS wal_position
