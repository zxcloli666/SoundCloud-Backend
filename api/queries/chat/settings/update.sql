INSERT INTO chat.users AS u (user_id, dm_policy, read_receipts, typing, presence_share)
VALUES ($1, COALESCE($2, 'following'), COALESCE($3, true), COALESCE($4, true), COALESCE($5, false))
ON CONFLICT (user_id) DO UPDATE
    SET dm_policy = COALESCE($2, u.dm_policy),
        read_receipts = COALESCE($3, u.read_receipts),
        typing = COALESCE($4, u.typing),
        presence_share = COALESCE($5, u.presence_share),
        updated_at = now()
RETURNING dm_policy, read_receipts, typing, presence_share
