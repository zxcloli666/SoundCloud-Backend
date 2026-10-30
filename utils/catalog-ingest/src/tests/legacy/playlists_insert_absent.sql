INSERT INTO playlists (
    urn, sc_playlist_id, title, title_normalized, description, genre, tags,
    artwork_url, permalink_url, owner_sc_user_id, owner_urn, owner_username,
    track_count, duration_ms, playlist_type, kind, sharing,
    release_year, release_date, label_name, likes_count_sc, reposts_count_sc,
    sc_created_at, sc_last_modified, sc_synced_at, sc_observation, sc_metadata
)
SELECT row.urn, row.sc_playlist_id, row.title, row.title_normalized, row.description,
       row.genre, row.tags, row.artwork_url, row.permalink_url, row.owner_sc_user_id,
       row.owner_urn, row.owner_username, row.track_count, row.duration_ms, row.playlist_type,
       row.kind, row.sharing, row.release_year, row.release_date, row.label_name,
       row.likes_count_sc, row.reposts_count_sc, row.sc_created_at, row.sc_last_modified,
       now(), $2, row.sc_metadata
FROM jsonb_to_recordset($1::jsonb) AS row(
    urn text, sc_playlist_id text, title text, title_normalized text, description text,
    genre text, tags text[], artwork_url text, permalink_url text, owner_sc_user_id text,
    owner_urn text, owner_username text, track_count integer, duration_ms bigint,
    playlist_type text, kind text, sharing text, release_year smallint, release_date date,
    label_name text, likes_count_sc bigint, reposts_count_sc bigint,
    sc_created_at timestamptz, sc_last_modified timestamptz, sc_metadata jsonb
)
ON CONFLICT DO NOTHING
RETURNING urn
