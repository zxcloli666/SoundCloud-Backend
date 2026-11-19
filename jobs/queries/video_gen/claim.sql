WITH due AS (
    SELECT gen.sc_track_id
    FROM track_video_gen AS gen
    WHERE gen.sent_at IS NULL
      AND gen.next_at <= now()
      AND gen.stage = ANY ($2::text[])
    ORDER BY gen.next_at, gen.sc_track_id
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
UPDATE track_video_gen AS gen
SET next_at = now() + $3::float8 * interval '1 second',
    updated_at = now()
FROM due
WHERE gen.sc_track_id = due.sc_track_id
RETURNING gen.sc_track_id, gen.stage, gen.attempt, gen.step, gen.shot, gen.redo, gen.faults,
          gen.plan, gen.state, gen.rejections
