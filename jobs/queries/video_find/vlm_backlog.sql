SELECT count(*) AS "backlog!"
FROM track_video_dispatch
WHERE stage IN ('pending_vlm', 'vlm_dispatched')
