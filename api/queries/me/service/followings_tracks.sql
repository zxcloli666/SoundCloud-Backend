WITH followed AS MATERIALIZED (
    SELECT DISTINCT ON (target_user_urn) target_user_urn, wanted_state
    FROM user_followings
    WHERE user_id = ANY($1)
    ORDER BY target_user_urn, (user_id = $2) DESC
)
SELECT t.sc_track_id
FROM followed f
CROSS JOIN LATERAL (
    SELECT sc_track_id, release_date, sc_created_at
    FROM tracks
    WHERE uploader_sc_user_id = ANY(ARRAY[
        split_part(f.target_user_urn, ':', 3), f.target_user_urn
    ])
      AND sharing = 'public' AND deleted_at IS NULL AND superseded_by IS NULL
    OFFSET 0
) t
WHERE f.wanted_state
ORDER BY t.release_date DESC NULLS LAST, t.sc_created_at DESC NULLS LAST, t.sc_track_id DESC
LIMIT $3 OFFSET $4
