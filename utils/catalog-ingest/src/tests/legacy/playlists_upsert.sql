INSERT INTO playlists (
   urn, sc_playlist_id, title, title_normalized, description, genre, tags,
   artwork_url, permalink_url, owner_sc_user_id, owner_urn, owner_username,
   track_count, duration_ms, playlist_type, kind, sharing,
   release_year, release_date, label_name, likes_count_sc, reposts_count_sc,
   sc_created_at, sc_last_modified, sc_synced_at, sc_observation, sc_metadata
) VALUES (
   $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24, now(), $25, $26
)
ON CONFLICT (urn) DO UPDATE SET
   sc_playlist_id = EXCLUDED.sc_playlist_id,
   title = EXCLUDED.title,
   title_normalized = EXCLUDED.title_normalized,
   description = EXCLUDED.description,
   genre = EXCLUDED.genre,
   tags = EXCLUDED.tags,
   artwork_url = EXCLUDED.artwork_url,
   permalink_url = EXCLUDED.permalink_url,
   owner_sc_user_id = COALESCE(EXCLUDED.owner_sc_user_id, playlists.owner_sc_user_id),
   owner_urn = COALESCE(EXCLUDED.owner_urn, playlists.owner_urn),
   owner_username = COALESCE(EXCLUDED.owner_username, playlists.owner_username),
   track_count = EXCLUDED.track_count,
   duration_ms = COALESCE(EXCLUDED.duration_ms, playlists.duration_ms),
   playlist_type = COALESCE(EXCLUDED.playlist_type, playlists.playlist_type),
   kind = COALESCE(EXCLUDED.kind, playlists.kind),
   sharing = EXCLUDED.sharing,
   sc_metadata = EXCLUDED.sc_metadata,
   release_year = COALESCE(EXCLUDED.release_year, playlists.release_year),
   release_date = COALESCE(EXCLUDED.release_date, playlists.release_date),
   label_name = COALESCE(EXCLUDED.label_name, playlists.label_name),
   likes_count_sc = COALESCE(EXCLUDED.likes_count_sc, playlists.likes_count_sc),
   reposts_count_sc = COALESCE(EXCLUDED.reposts_count_sc, playlists.reposts_count_sc),
   sc_created_at = COALESCE(EXCLUDED.sc_created_at, playlists.sc_created_at),
   sc_last_modified = COALESCE(EXCLUDED.sc_last_modified, playlists.sc_last_modified),
   sc_synced_at = now(),
   sc_observation = GREATEST(playlists.sc_observation, EXCLUDED.sc_observation),
   sc_desired = '{}',
   sc_write_confirmed = false,
   updated_at = now()
WHERE playlists.deleted_at IS NULL AND catalog_observation_is_current(
   playlists.sc_observation, playlists.sc_mutation_observation, EXCLUDED.sc_observation,
   playlists.sc_last_modified, COALESCE(EXCLUDED.sc_last_modified, playlists.sc_last_modified),
   playlists.sc_desired, playlists.sc_write_confirmed, to_jsonb(EXCLUDED)
)
RETURNING (xmax = 0) AS "was_new!"
