INSERT INTO chat.media_holds (media_id, report_id, until)
VALUES ($1, $2, now() + $3::int * interval '1 day')
ON CONFLICT DO NOTHING
