SELECT EXISTS(
    SELECT 1
    FROM unnest($1::text[]) AS local(sc_track_id)
    JOIN tracks AS track ON track.sc_track_id = local.sc_track_id
    WHERE NOT (local.sc_track_id = ANY ($2))
      AND track.deleted_at IS NULL
      AND (track.sharing = 'public' OR track.uploader_sc_user_id = $3)
) AS "lacking!"
