WITH event AS (
    SELECT *
    FROM UNNEST(
        $1::uuid[],
        $2::text[],
        $3::text[],
        $4::real[],
        $5::timestamptz[]
    ) AS event(event_id, sc_user_id, sc_track_id, position_pct, detected_at)
), inserted AS (
    INSERT INTO rec_hard_negatives (
        event_id,
        sc_user_id,
        sc_track_id,
        predicted_score,
        position_pct,
        detected_at
    )
    SELECT event.event_id,
           event.sc_user_id,
           event.sc_track_id,
           impression.score,
           event.position_pct,
           event.detected_at
    FROM event
    CROSS JOIN LATERAL (
        SELECT candidate.score
        FROM rec_impressions AS candidate
        WHERE candidate.sc_user_id = event.sc_user_id
          AND candidate.sc_track_id = event.sc_track_id
          AND candidate.shown_at <= event.detected_at
        ORDER BY candidate.shown_at DESC, candidate.id DESC
        LIMIT 1
    ) AS impression
    ON CONFLICT (event_id) WHERE event_id IS NOT NULL DO NOTHING
    RETURNING event_id
), advanced AS (
    INSERT INTO rec_hard_negative_sweep AS sweep (
        singleton,
        scanned_through,
        scanned_event_id,
        updated_at
    )
    VALUES (true, $6, $7, now())
    ON CONFLICT (singleton) DO UPDATE
    SET scanned_through = EXCLUDED.scanned_through,
        scanned_event_id = EXCLUDED.scanned_event_id,
        updated_at = now()
    WHERE (sweep.scanned_through, sweep.scanned_event_id)
        < (EXCLUDED.scanned_through, EXCLUDED.scanned_event_id)
    RETURNING singleton
)
SELECT count(*)::bigint AS "recorded!"
FROM inserted
