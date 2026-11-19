SELECT count(*) AS "waiting!"
FROM track_video_dispatch
WHERE stage = 'pending_vlm'
