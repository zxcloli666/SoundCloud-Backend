SELECT COALESCE(
    (SELECT wanted_state FROM user_followings
     WHERE user_id = ANY($2) AND target_user_urn = $3
     ORDER BY (user_id = $1) DESC LIMIT 1),
    (SELECT false FROM catalog_collection_sync
     WHERE subject_id = $1 AND collection = 'followings' AND scope = $4 AND synced_at IS NOT NULL)
) AS "following?"
