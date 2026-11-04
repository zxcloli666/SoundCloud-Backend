WITH batch AS MATERIALIZED (
    SELECT a, b, co, weight
    FROM colike_fresh
    WHERE a BETWEEN $1 AND $2
), dropped AS (
    DELETE FROM artist_colike AS edge
    WHERE edge.a_id BETWEEN $1 AND $2
      AND NOT EXISTS (
          SELECT 1
          FROM batch
          WHERE batch.a = edge.a_id
            AND batch.b = edge.b_id
      )
    RETURNING 1
), upserted AS (
    INSERT INTO artist_colike (a_id, b_id, co, w, updated_at)
    SELECT a, b, co, weight, $3
    FROM batch
    ON CONFLICT (a_id, b_id) DO UPDATE
    SET co = excluded.co,
        w = excluded.w,
        updated_at = excluded.updated_at
    WHERE artist_colike.co IS DISTINCT FROM excluded.co
       OR artist_colike.w IS DISTINCT FROM excluded.w
    RETURNING 1
)
SELECT (SELECT count(*) FROM upserted) AS upserted,
       (SELECT count(*) FROM dropped) AS removed
