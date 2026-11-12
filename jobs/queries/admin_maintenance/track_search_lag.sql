SELECT COALESCE((
           SELECT max(EXTRACT(EPOCH FROM sender.replay_lag))
           FROM pg_stat_replication AS sender
           JOIN pg_replication_slots AS slot ON slot.active_pid = sender.pid
           WHERE slot.slot_type = 'logical'
             AND slot.slot_name !~ '_graph(_|$)'
       ), 0)::float8 AS lag_seconds,
       COALESCE((
           SELECT max(pg_wal_lsn_diff(pg_current_wal_lsn(), slot.confirmed_flush_lsn))
           FROM pg_replication_slots AS slot
           WHERE slot.slot_type = 'logical'
             AND slot.slot_name !~ '_graph(_|$)'
       ), 0)::bigint AS backlog_bytes
