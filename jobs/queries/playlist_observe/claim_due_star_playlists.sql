WITH star AS MATERIALIZED (
    SELECT subscription.user_urn AS owner_sc_user_id
    FROM subscriptions AS subscription
    WHERE subscription.exp_date > extract(epoch FROM now())::bigint
),
picked AS (
    SELECT share.playlist_urn, share.next_reconcile_at
    FROM star
    CROSS JOIN LATERAL (
        SELECT state.playlist_urn, state.next_reconcile_at
        FROM playlist_membership_state AS state
        WHERE state.owner_sc_user_id = star.owner_sc_user_id
          AND state.sync_status <> 'clean'
          AND state.next_reconcile_at IS NOT NULL
          AND state.next_reconcile_at <= now()
        ORDER BY state.next_reconcile_at, state.playlist_urn
        LIMIT $1
    ) AS share
    ORDER BY share.next_reconcile_at, share.playlist_urn
    LIMIT $2
)
UPDATE playlist_membership_state AS state
SET next_reconcile_at = clock_timestamp() + $3::double precision * interval '1 second',
    updated_at = clock_timestamp()
FROM picked
WHERE state.playlist_urn = picked.playlist_urn
RETURNING state.playlist_urn AS "playlist_urn!",
          picked.next_reconcile_at AS "due_at!"
