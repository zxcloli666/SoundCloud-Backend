WITH live AS (
    SELECT media.id,
           media.status,
           media.restorable_until
    FROM chat.media AS media
    WHERE media.id = ANY($1::uuid[])
      AND media.kind <> 'thumb'
      AND (
          media.status IN ('uploaded', 'attached', 'held')
          OR (media.status = 'deleting' AND media.lease_until IS NULL)
      )
),
held AS (
    INSERT INTO chat.media_holds AS hold (media_id, report_id, until)
    SELECT live.id, $2::uuid, now() + $3::int * interval '1 day'
    FROM live
    ON CONFLICT (media_id, report_id) DO UPDATE
    SET until = greatest(hold.until, excluded.until)
    RETURNING media_id
),
kept AS (
    UPDATE chat.media AS media
    SET status = 'held',
        restorable_until = NULL
    FROM live
    WHERE (media.id = live.id OR media.parent_id = live.id)
      AND live.status = 'deleting'
      AND (live.restorable_until IS NULL OR live.restorable_until <= now())
      AND media.status = 'deleting'
      AND NOT EXISTS (
          SELECT 1
          FROM chat.media_refs AS ref
          WHERE ref.media_id = live.id
      )
)
SELECT media_id AS "media_id!"
FROM held
ORDER BY 1
