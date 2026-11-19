SELECT count(*) FILTER (WHERE sent_at IS NOT NULL) AS "sent!",
       count(*) FILTER (WHERE stage NOT IN ('done', 'rejected', 'failed')) AS "active!"
FROM track_video_gen
