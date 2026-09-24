WITH candidate AS MATERIALIZED (
    SELECT impression.score
    FROM rec_impressions AS impression
    WHERE impression.sc_user_id = $2
      AND impression.sc_track_id = $3
      AND impression.shown_at <= $5
    ORDER BY impression.shown_at DESC, impression.id DESC
    LIMIT 1
), inserted AS (
    INSERT INTO rec_hard_negatives (
        event_id,
        sc_user_id,
        sc_track_id,
        predicted_score,
        position_pct,
        detected_at
    )
    SELECT $1,
           $2,
           $3,
           candidate.score,
           $4,
           $5
    FROM candidate
    ON CONFLICT (event_id) WHERE event_id IS NOT NULL DO NOTHING
    RETURNING event_id
)
SELECT EXISTS (
    SELECT 1
    FROM inserted
    UNION ALL
    SELECT 1
    FROM rec_hard_negatives
    WHERE event_id = $1
) AS "recorded!"
