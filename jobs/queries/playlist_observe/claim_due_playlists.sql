WITH due_owners AS MATERIALIZED (
    SELECT DISTINCT state.owner_sc_user_id
    FROM playlist_membership_state AS state
    WHERE state.sync_status <> 'clean'
      AND state.next_reconcile_at IS NOT NULL
      AND state.next_reconcile_at <= now()
),
ahead AS (
    SELECT due.owner_sc_user_id
    FROM due_owners AS due
    WHERE due.owner_sc_user_id > (SELECT cursor.last_owner_sc_user_id FROM playlist_sweep_cursor AS cursor)
    ORDER BY due.owner_sc_user_id
    LIMIT $1
),
owners AS (
    SELECT ahead.owner_sc_user_id
    FROM ahead
    UNION ALL
    (
        SELECT due.owner_sc_user_id
        FROM due_owners AS due
        WHERE NOT EXISTS (SELECT 1 FROM ahead)
        ORDER BY due.owner_sc_user_id
        LIMIT $1
    )
),
picked AS (
    SELECT share.playlist_urn, owners.owner_sc_user_id, share.next_reconcile_at
    FROM owners
    CROSS JOIN LATERAL (
        SELECT state.playlist_urn, state.next_reconcile_at
        FROM playlist_membership_state AS state
        WHERE state.owner_sc_user_id = owners.owner_sc_user_id
          AND state.sync_status <> 'clean'
          AND state.next_reconcile_at IS NOT NULL
          AND state.next_reconcile_at <= now()
        ORDER BY state.next_reconcile_at, state.playlist_urn
        LIMIT $2
    ) AS share
    ORDER BY owners.owner_sc_user_id, share.next_reconcile_at, share.playlist_urn
    LIMIT $3
),
advance AS (
    UPDATE playlist_sweep_cursor
    SET last_owner_sc_user_id = (SELECT max(owner_sc_user_id) FROM picked),
        updated_at = now()
    WHERE EXISTS (SELECT 1 FROM picked)
)
UPDATE playlist_membership_state AS state
SET next_reconcile_at = clock_timestamp() + make_interval(secs => $4::double precision),
    updated_at = clock_timestamp()
FROM picked
WHERE state.playlist_urn = picked.playlist_urn
RETURNING state.playlist_urn AS "playlist_urn!"
