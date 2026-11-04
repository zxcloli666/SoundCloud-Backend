INSERT INTO chat.media_holds (media_id, report_id, until)
SELECT media_id, $2::uuid, now() + $3::int * interval '1 day'
FROM unnest($1::uuid[]) AS media_id
ON CONFLICT DO NOTHING
