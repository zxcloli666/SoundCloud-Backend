WITH star AS MATERIALIZED (
    SELECT subscription.user_urn AS owner_sc_user_id
    FROM subscriptions AS subscription
    WHERE subscription.exp_date > extract(epoch FROM now())::bigint
)
SELECT (
           SELECT min(state.next_reconcile_at)
           FROM star
           JOIN playlist_membership_state AS state
             ON state.owner_sc_user_id = star.owner_sc_user_id
           WHERE state.sync_status <> 'clean'
             AND state.next_reconcile_at <= now()
       ) AS star,
       (
           SELECT state.next_reconcile_at
           FROM playlist_membership_state AS state
           WHERE state.sync_status <> 'clean'
             AND state.next_reconcile_at <= now()
             AND NOT EXISTS (
                 SELECT 1
                 FROM star
                 WHERE star.owner_sc_user_id = state.owner_sc_user_id
             )
           ORDER BY state.next_reconcile_at, state.playlist_urn
           LIMIT 1
       ) AS regular
