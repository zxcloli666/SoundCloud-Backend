INSERT INTO users (
    sc_user_id, urn, username, username_normalized, full_name, first_name, last_name,
    permalink, permalink_url, avatar_url, country, city, description, verified,
    followers_count, followings_count, tracks_count, playlists_count,
    reposts_count, comments_count, kind, sc_created_at, sc_last_modified, sc_synced_at, sc_observation
)
SELECT row.sc_user_id, row.urn, row.username, row.username_normalized, row.full_name,
       row.first_name, row.last_name, row.permalink, row.permalink_url, row.avatar_url,
       row.country, row.city, row.description, row.verified, row.followers_count,
       row.followings_count, row.tracks_count, row.playlists_count, row.reposts_count,
       row.comments_count, row.kind, row.sc_created_at, row.sc_last_modified, now(), $2
FROM jsonb_to_recordset($1::jsonb) AS row(
    sc_user_id text, urn text, username text, username_normalized text, full_name text,
    first_name text, last_name text, permalink text, permalink_url text, avatar_url text,
    country text, city text, description text, verified boolean, followers_count bigint,
    followings_count bigint, tracks_count bigint, playlists_count bigint, reposts_count bigint,
    comments_count bigint, kind text, sc_created_at timestamptz, sc_last_modified timestamptz
)
ON CONFLICT DO NOTHING
RETURNING sc_user_id
