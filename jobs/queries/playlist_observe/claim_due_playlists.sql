WITH RECURSIVE resume AS (
    SELECT CASE
        WHEN cursor.last_owner_sc_user_id IS NULL THEN NULL
        WHEN EXISTS (
            SELECT 1 FROM playlist_membership_state AS state
            WHERE state.sync_status <> 'clean'
              AND state.next_reconcile_at IS NOT NULL
              AND state.next_reconcile_at <= now()
              AND state.owner_sc_user_id > cursor.last_owner_sc_user_id
        ) THEN cursor.last_owner_sc_user_id
        ELSE NULL
    END AS after_owner
    FROM playlist_sweep_cursor AS cursor
),
owners AS (
    (
        SELECT state.owner_sc_user_id, 1 AS depth
        FROM playlist_membership_state AS state, resume
        WHERE state.sync_status <> 'clean'
          AND state.next_reconcile_at IS NOT NULL
          AND state.next_reconcile_at <= now()
          AND (resume.after_owner IS NULL OR state.owner_sc_user_id > resume.after_owner)
        ORDER BY state.owner_sc_user_id
        LIMIT 1
    )
    UNION ALL
    SELECT next.owner_sc_user_id, owners.depth + 1
    FROM owners
    CROSS JOIN LATERAL (
        SELECT state.owner_sc_user_id
        FROM playlist_membership_state AS state
        WHERE state.sync_status <> 'clean'
          AND state.next_reconcile_at IS NOT NULL
          AND state.next_reconcile_at <= now()
          AND state.owner_sc_user_id > owners.owner_sc_user_id
        ORDER BY state.owner_sc_user_id
        LIMIT 1
    ) AS next
    WHERE owners.depth < $1
),
picked AS (
    SELECT share.playlist_urn, owners.owner_sc_user_id
    FROM owners
    CROSS JOIN LATERAL (
        SELECT state.playlist_urn
        FROM playlist_membership_state AS state
        WHERE state.owner_sc_user_id = owners.owner_sc_user_id
          AND state.sync_status <> 'clean'
          AND state.next_reconcile_at IS NOT NULL
          AND state.next_reconcile_at <= now()
        ORDER BY state.next_reconcile_at, state.playlist_urn
        LIMIT $2
    ) AS share
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
