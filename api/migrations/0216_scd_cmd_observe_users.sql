SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_observe_users(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    stamp timestamptz := (args ->> 'observed_at')::timestamptz;
    seen bigint := (args ->> 'observation')::bigint;
    r record;
    fresh boolean;
    output jsonb := '[]';
    touched jsonb := '[]';
BEGIN
    IF args ->> 'mode' = 'absent' THEN
        WITH inserted AS (
            INSERT INTO public.users (
                sc_user_id, urn, username, username_normalized, full_name, first_name, last_name,
                permalink, permalink_url, avatar_url, country, city, description, verified,
                followers_count, followings_count, tracks_count, playlists_count,
                reposts_count, comments_count, kind, sc_created_at, sc_last_modified, sc_synced_at, sc_observation,
                created_at, updated_at
            )
            SELECT row.sc_user_id, row.urn, row.username, row.username_normalized, row.full_name,
                   row.first_name, row.last_name, row.permalink, row.permalink_url, row.avatar_url,
                   row.country, row.city, row.description, row.verified, row.followers_count,
                   row.followings_count, row.tracks_count, row.playlists_count, row.reposts_count,
                   row.comments_count, row.kind, row.sc_created_at, row.sc_last_modified, stamp, seen,
                   stamp, stamp
            FROM jsonb_to_recordset(args -> 'rows') AS row(
                sc_user_id text, urn text, username text, username_normalized text, full_name text,
                first_name text, last_name text, permalink text, permalink_url text, avatar_url text,
                country text, city text, description text, verified boolean, followers_count bigint,
                followings_count bigint, tracks_count bigint, playlists_count bigint, reposts_count bigint,
                comments_count bigint, kind text, sc_created_at timestamptz, sc_last_modified timestamptz
            )
            ON CONFLICT DO NOTHING
            RETURNING sc_user_id
        )
        SELECT coalesce(jsonb_agg(jsonb_build_object('sc_user_id', inserted.sc_user_id, 'was_new', true)), '[]'),
               coalesce(jsonb_agg(jsonb_build_object('tbl', 'users', 'pk', jsonb_build_object('sc_user_id', inserted.sc_user_id))), '[]')
        INTO output, touched
        FROM inserted;
        RETURN jsonb_build_object('touched', touched, 'output', output);
    END IF;

    FOR r IN
        SELECT *
        FROM jsonb_to_recordset(args -> 'rows') AS row(
            sc_user_id text, urn text, username text, username_normalized text, full_name text,
            first_name text, last_name text, permalink text, permalink_url text, avatar_url text,
            country text, city text, description text, verified boolean, followers_count bigint,
            followings_count bigint, tracks_count bigint, playlists_count bigint, reposts_count bigint,
            comments_count bigint, kind text, sc_created_at timestamptz, sc_last_modified timestamptz,
            keys jsonb
        )
    LOOP
        INSERT INTO public.users AS users (
           sc_user_id, urn, username, username_normalized, full_name, first_name, last_name,
           permalink, permalink_url, avatar_url, country, city, description, verified,
           followers_count, followings_count, tracks_count, playlists_count,
           reposts_count, comments_count, kind, sc_created_at, sc_last_modified, sc_synced_at, sc_observation,
           created_at, updated_at
        ) VALUES (
           r.sc_user_id, r.urn, r.username, r.username_normalized, r.full_name, r.first_name, r.last_name,
           r.permalink, r.permalink_url, r.avatar_url, r.country, r.city, r.description, r.verified,
           r.followers_count, r.followings_count, r.tracks_count, r.playlists_count,
           r.reposts_count, r.comments_count, r.kind, r.sc_created_at, r.sc_last_modified, stamp, seen,
           stamp, stamp
        )
        ON CONFLICT (sc_user_id) DO UPDATE SET
           urn = EXCLUDED.urn,
           username = EXCLUDED.username,
           username_normalized = EXCLUDED.username_normalized,
           full_name = CASE WHEN r.keys ? 'full_name' THEN EXCLUDED.full_name ELSE users.full_name END,
           first_name = CASE WHEN r.keys ? 'first_name' THEN EXCLUDED.first_name ELSE users.first_name END,
           last_name = CASE WHEN r.keys ? 'last_name' THEN EXCLUDED.last_name ELSE users.last_name END,
           permalink = CASE WHEN r.keys ? 'permalink' THEN EXCLUDED.permalink ELSE users.permalink END,
           permalink_url = CASE WHEN r.keys ? 'permalink_url' THEN EXCLUDED.permalink_url ELSE users.permalink_url END,
           avatar_url = CASE WHEN r.keys ? 'avatar_url' THEN EXCLUDED.avatar_url ELSE users.avatar_url END,
           country = CASE WHEN r.keys ?| ARRAY['country', 'country_code'] THEN EXCLUDED.country ELSE users.country END,
           city = CASE WHEN r.keys ? 'city' THEN EXCLUDED.city ELSE users.city END,
           description = CASE WHEN r.keys ? 'description' THEN EXCLUDED.description ELSE users.description END,
           verified = CASE WHEN r.keys ? 'verified' THEN EXCLUDED.verified ELSE users.verified END,
           followers_count = COALESCE(EXCLUDED.followers_count, users.followers_count),
           followings_count = COALESCE(EXCLUDED.followings_count, users.followings_count),
           tracks_count = COALESCE(EXCLUDED.tracks_count, users.tracks_count),
           playlists_count = COALESCE(EXCLUDED.playlists_count, users.playlists_count),
           reposts_count = COALESCE(EXCLUDED.reposts_count, users.reposts_count),
           comments_count = COALESCE(EXCLUDED.comments_count, users.comments_count),
           kind = CASE WHEN r.keys ? 'kind' THEN EXCLUDED.kind ELSE users.kind END,
           sc_created_at = COALESCE(EXCLUDED.sc_created_at, users.sc_created_at),
           sc_last_modified = COALESCE(EXCLUDED.sc_last_modified, users.sc_last_modified),
           sc_synced_at = stamp,
           sc_observation = GREATEST(users.sc_observation, EXCLUDED.sc_observation),
           updated_at = stamp
        WHERE EXCLUDED.sc_observation > 0
          AND (
              EXCLUDED.sc_last_modified IS NULL
              OR users.sc_synced_at < stamp - interval '7 days'
              OR users.sc_last_modified IS DISTINCT FROM EXCLUDED.sc_last_modified
              OR users.username IS DISTINCT FROM EXCLUDED.username
              OR (r.keys ? 'avatar_url' AND users.avatar_url IS DISTINCT FROM EXCLUDED.avatar_url)
              OR (r.keys ? 'verified' AND users.verified IS DISTINCT FROM EXCLUDED.verified)
          )
          AND (EXCLUDED.sc_observation > users.sc_observation
               OR EXCLUDED.sc_last_modified > users.sc_last_modified)
          AND (users.sc_last_modified IS NULL
               OR COALESCE(EXCLUDED.sc_last_modified, users.sc_last_modified) >= users.sc_last_modified)
        RETURNING (users.xmax = 0) INTO fresh;
        IF FOUND THEN
            output := output || jsonb_build_array(jsonb_build_object('sc_user_id', r.sc_user_id, 'was_new', fresh));
            touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'users', 'pk', jsonb_build_object('sc_user_id', r.sc_user_id)));
        END IF;
    END LOOP;
    RETURN jsonb_build_object('touched', touched, 'output', output);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_observe_profile(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    user_id text := args ->> 'user_id';
BEGIN
    INSERT INTO public.user_profiles AS profiles (soundcloud_user_id, profile_json, synced_at, sc_observation)
    VALUES (user_id, args -> 'profile', (args ->> 'observed_at')::timestamptz, (args ->> 'observation')::bigint)
    ON CONFLICT (soundcloud_user_id) DO UPDATE
    SET profile_json = EXCLUDED.profile_json,
        synced_at = EXCLUDED.synced_at,
        sc_observation = EXCLUDED.sc_observation
    WHERE EXCLUDED.sc_observation > profiles.sc_observation;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', NULL);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'user_profiles', 'pk', jsonb_build_object('soundcloud_user_id', user_id))),
        'output', NULL);
END
$f$;
