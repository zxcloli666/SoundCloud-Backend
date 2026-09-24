DO
$$
DECLARE
    target       text;
    default_rows constant int := 50000;
    overrides    constant jsonb := coalesce(
        nullif(current_setting('planfixture.overrides', true), '')::jsonb, '{}'::jsonb
    );
    track_uuid   constant text := $u$('00000000-0000-4000-8000-' || lpad(to_hex(%s), 12, '0'))::uuid$u$;
    artist_uuid  constant text := $u$('00000000-0000-4000-8001-' || lpad(to_hex(%s), 12, '0'))::uuid$u$;
    event_uuid   constant text := $u$('00000000-0000-4000-8002-' || lpad(to_hex(%s), 12, '0'))::uuid$u$;
    album_uuid   constant text := $u$('00000000-0000-4000-8003-' || lpad(to_hex(%s), 12, '0'))::uuid$u$;
    wanted_uuid  constant text := $u$('00000000-0000-4000-8004-' || lpad(to_hex(%s), 12, '0'))::uuid$u$;
    artist_pick  constant text := '1 + floor(power(random(), 2) * %s)::int';
    word_a       constant text := $p$(ARRAY['midnight', 'ocean', 'velvet', 'neon', 'crystal', 'shadow', 'golden', 'silver', 'crimson', 'frozen', 'electric', 'lunar', 'solar', 'silent', 'wild', 'broken', 'endless', 'hidden', 'distant', 'burning', 'quiet', 'rising', 'falling', 'sacred'])$p$;
    word_b       constant text := $p$(ARRAY['dreams', 'echoes', 'waves', 'lights', 'roads', 'hearts', 'storms', 'skies', 'rivers', 'fires', 'ghosts', 'mirrors', 'angels', 'machines', 'gardens', 'winters', 'summers', 'nights', 'mornings', 'shadows', 'voices', 'signals', 'horizons', 'tides'])$p$;
    genre_pool   constant text := $p$(ARRAY['house', 'techno', 'hip-hop', 'trap', 'ambient', 'drum and bass', 'dubstep', 'lo-fi', 'pop', 'rock', 'jazz', 'soul', 'funk', 'metal', 'indie', 'phonk', 'trance', 'garage', 'rnb', 'experimental'])$p$;
    title_expr   constant text := word_a || '[1 + (n * 7) % 24] || '' '' || ' || word_b || '[1 + (n * 13) % 24] || '' '' || n';
    person_expr  constant text := word_a || '[1 + (n * 11) % 24] || '' '' || ' || word_b || '[1 + (n * 17) % 24] || '' '' || n';
    genre_expr   constant text := genre_pool || '[1 + floor(power(random(), 2) * 20)::int]';
    tags_expr    constant text := 'ARRAY(SELECT ' || genre_pool
                                  || '[1 + floor(power(random(), 2) * 20)::int] FROM generate_series(1, 1 + (n % 3)))';
    language_expr constant text := $p$(ARRAY['en', 'en', 'en', 'en', 'ru', 'es', 'de', 'fr', 'pt', 'ja'])[1 + (n * 3) % 10]$p$;
    storage_expr constant text := $p$(ARRAY['ok', 'ok', 'ok', 'pending', 'pending', 'failed', 'too_long'])[1 + (n * 3) % 7]$p$;
    index_expr   constant text := $p$(ARRAY['indexed', 'indexed', 'indexed', 'pending', 'pending', 'failed', 'too_long'])[1 + (n * 5) % 7]$p$;
    sharing_expr constant text := $p$CASE WHEN n % 25 = 0 THEN 'private' ELSE 'public' END$p$;
    plays_expr   constant text := 'floor(power(random(), 6) * 10000000)::bigint';
    rows_wanted  int;
    users_rows   int := default_rows;
    tracks_rows  int := default_rows;
    artists_rows int := default_rows;
    albums_rows  int := default_rows;
    wanted_rows  int := default_rows;
    column_list  text;
    value_list   text;
    seeded       int;
    failed       text := '';
BEGIN
    FOREACH target IN ARRAY ARRAY[
        'users', 'artists', 'albums', 'tracks', 'track_artists', 'album_tracks', 'album_artists',
        'artist_colike', 'sc_track_counters', 'wanted_tracks', 'wanted_track_albums',
        'user_likes_tracks', 'user_events',
        'listening_history', 'lyrics_cache', 'lyrics_lookup_state', 'background_jobs',
        'playlists', 'playlist_track_projection', 'playlist_membership_state',
        'playlist_membership_operations', 'playlist_remote_snapshot_tracks',
        'catalog_audience', 'track_comments', 'user_followings'
    ]
    LOOP
        IF to_regclass(target) IS NULL THEN
            CONTINUE;
        END IF;

        rows_wanted := coalesce((overrides ->> target)::int, default_rows);
        IF target = 'users' THEN users_rows := rows_wanted; END IF;
        IF target = 'tracks' THEN tracks_rows := rows_wanted; END IF;
        IF target = 'artists' THEN artists_rows := rows_wanted; END IF;
        IF target = 'albums' THEN albums_rows := rows_wanted; END IF;
        IF target = 'wanted_tracks' THEN wanted_rows := rows_wanted; END IF;

        SELECT string_agg(quote_ident(attname), ', ' ORDER BY attnum),
               string_agg(
                   CASE
                       WHEN target || '.' || attname IN ('tracks.title', 'tracks.title_normalized')
                           THEN title_expr
                       WHEN target || '.' || attname IN ('users.username', 'users.username_normalized',
                                                         'artists.name', 'artists.normalized_name')
                           THEN person_expr
                       WHEN target || '.' || attname = 'tracks.genre' THEN genre_expr
                       WHEN target || '.' || attname = 'artists.tags' THEN tags_expr
                       WHEN target || '.' || attname = 'tracks.language' THEN language_expr
                       WHEN target || '.' || attname = 'tracks.storage_state' THEN storage_expr
                       WHEN target || '.' || attname = 'tracks.index_state' THEN index_expr
                       WHEN target || '.' || attname = 'tracks.sharing' THEN sharing_expr
                       WHEN target || '.' || attname = 'tracks.play_count_sc' THEN plays_expr
                       WHEN target || '.' || attname IN ('tracks.storage_priority', 'tracks.index_priority')
                           THEN '1 + (n % 5)'
                       WHEN target || '.' || attname = 'tracks.primary_artist_id'
                           THEN format(artist_uuid, format(artist_pick, artists_rows))
                       WHEN target || '.' || attname = 'tracks.uploader_sc_user_id'
                           THEN '(' || format(artist_pick, users_rows) || ')::text'
                       WHEN target || '.' || attname = 'tracks.id' THEN format(track_uuid, 'n')
                       WHEN target || '.' || attname = 'artists.id' THEN format(artist_uuid, 'n')
                       WHEN target || '.' || attname = 'user_events.id' THEN format(event_uuid, 'n')
                       WHEN target || '.' || attname = 'albums.id' THEN format(album_uuid, 'n')
                       WHEN target || '.' || attname = 'albums.primary_artist_id'
                           THEN format(artist_uuid, format(artist_pick, artists_rows))
                       WHEN target || '.' || attname = 'album_artists.album_id'
                           THEN format(album_uuid, '1 + (n - 1) % ' || albums_rows)
                       WHEN target || '.' || attname = 'album_artists.artist_id'
                           THEN format(artist_uuid, format(artist_pick, artists_rows))
                       WHEN target || '.' || attname = 'album_artists.role' THEN $v$'featured'$v$
                       WHEN target || '.' || attname = 'wanted_tracks.id' THEN format(wanted_uuid, 'n')
                       WHEN target || '.' || attname = 'wanted_tracks.primary_artist_id'
                           THEN format(artist_uuid, format(artist_pick, artists_rows))
                       WHEN target || '.' || attname = 'wanted_track_albums.wanted_track_id'
                           THEN format(wanted_uuid, '1 + (n - 1) % ' || wanted_rows)
                       WHEN target || '.' || attname IN ('album_tracks.track_id', 'lyrics_cache.track_id',
                                                         'lyrics_lookup_state.track_id')
                           THEN format(track_uuid, '1 + (n - 1) % ' || tracks_rows)
                       WHEN target || '.' || attname = 'wanted_tracks.track_id'
                           THEN 'CASE WHEN n % 10 < 3 THEN '
                                || format(track_uuid, '1 + (n - 1) % ' || tracks_rows) || ' END'
                       WHEN target || '.' || attname = 'album_tracks.album_id'
                           THEN format(album_uuid, '1 + ((n - 1) / 10) % ' || albums_rows)
                       WHEN target || '.' || attname = 'wanted_track_albums.album_id'
                           THEN format(album_uuid, '1 + (n - 1) % ' || albums_rows)
                       WHEN target || '.' || attname = 'track_artists.track_id'
                           THEN format(track_uuid, '1 + (n - 1) % ' || tracks_rows)
                       WHEN target || '.' || attname = 'track_artists.artist_id'
                           THEN format(artist_uuid, format(artist_pick, artists_rows))
                       WHEN target || '.' || attname = 'track_artists.role'
                           THEN $v$CASE WHEN n % 10 = 0 THEN 'featured' ELSE 'primary' END$v$
                       WHEN target || '.' || attname = 'artist_colike.a_id'
                           THEN format(artist_uuid, '1 + n % ' || (artists_rows - 1))
                       WHEN target || '.' || attname = 'artist_colike.b_id'
                           THEN format(artist_uuid,
                                       '2 + n % ' || (artists_rows - 1)
                                       || ' + ((n::bigint * 7919) % (' || artists_rows
                                       || ' - 1 - n % ' || (artists_rows - 1) || '))::int')
                       WHEN target || '.' || attname = 'artist_colike.w' THEN 'random()::real'
                       WHEN target || '.' || attname = 'sc_track_counters.play_count'
                           THEN 'floor(power(random(), 6) * 10000000)::bigint'
                       WHEN target || '.' || attname = 'user_likes_tracks.user_id'
                           THEN '(1 + floor(power(random(), 2) * ' || users_rows || ')::int)::text'
                       WHEN target || '.' || attname = 'user_likes_tracks.sc_track_id'
                           THEN '(1 + floor(random() * ' || tracks_rows || ')::int)::text'
                       WHEN target || '.' || attname = 'user_events.sc_user_id'
                           THEN $v$CASE WHEN n % 3 = 0 THEN 'soundcloud:users:' ELSE '' END || $v$
                                || '(1 + floor(power(random(), 2) * ' || users_rows || ')::int)::text'
                       WHEN target || '.' || attname = 'user_events.sc_track_id'
                           THEN '(1 + floor(random() * ' || tracks_rows || ')::int)::text'
                       WHEN target || '.' || attname = 'user_events.event_type'
                           THEN $v$(ARRAY['full_play', 'full_play', 'full_play', 'full_play', 'skip', 'skip', 'skip', 'like', 'playlist_add', 'dislike'])[1 + n % 10]$v$
                       WHEN target || '.' || attname = 'user_events.created_at'
                           THEN format($t$now() - interval '400 days' * ((%s - n)::float8 / %s)$t$,
                                       rows_wanted, rows_wanted)
                       WHEN target || '.' || attname = 'lyrics_cache.source' THEN $v$'lrclib'$v$
                       WHEN attname = 'lane' THEN $v$'core_bulk'$v$
                       WHEN attname = 'expected_projection_revision' THEN 'n'
                       WHEN attname = 'accepted_projection_revision' THEN 'n + 1'
                       WHEN target || '.' || attname = 'lyrics_lookup_state.priority' THEN '0'
                       WHEN target || '.' || attname = 'playlist_membership_operations.kind'
                           THEN $v$'remove'$v$
                       WHEN target || '.' || attname = 'catalog_audience.relation'
                           THEN $v$'track-favoriters'$v$
                       WHEN attname = 'sc_track_id' OR attname = 'sc_user_id'
                            OR attname = 'sc_playlist_id' OR attname = 'sc_comment_id'
                           THEN 'n::text'
                       WHEN atttypid = 'uuid'::regtype THEN 'gen_random_uuid()'
                       WHEN atttypid IN ('int2'::regtype, 'int4'::regtype, 'int8'::regtype)
                           THEN 'n'
                       WHEN atttypid IN ('float4'::regtype, 'float8'::regtype, 'numeric'::regtype)
                           THEN 'n::numeric'
                       WHEN atttypid = 'bool'::regtype THEN 'false'
                       WHEN atttypid IN ('timestamptz'::regtype, 'timestamp'::regtype)
                           THEN $t$now() - ((n % 400) || ' days')::interval$t$
                       WHEN atttypid = 'date'::regtype THEN $d$current_date - (n % 400)$d$
                       WHEN atttypid = 'jsonb'::regtype THEN $j$'{}'::jsonb$j$
                       WHEN atttypid = 'json'::regtype THEN $j$'{}'::json$j$
                       WHEN atttypid = 'bytea'::regtype THEN $b$sha256(n::text::bytea)$b$
                       WHEN atttypid = '_text'::regtype THEN $a$ARRAY[]::text[]$a$
                       WHEN atttypid = 'varchar'::regtype AND atttypmod BETWEEN 5 AND 23
                           THEN $s$'p' || n$s$
                       ELSE $s$'planfixture' || n$s$
                   END,
                   ', ' ORDER BY attnum)
          INTO column_list, value_list
          FROM pg_attribute
         WHERE attrelid = to_regclass(target)
           AND attnum > 0
           AND NOT attisdropped
           AND (
               (attnotnull AND (
                   atttypid IN ('timestamptz'::regtype, 'timestamp'::regtype)
                   OR NOT EXISTS (
                       SELECT 1 FROM pg_attrdef d
                        WHERE d.adrelid = attrelid AND d.adnum = attnum
                   )
               ))
               OR target || '.' || attname IN (
                   'tracks.id', 'artists.id', 'albums.id', 'albums.primary_artist_id',
                   'wanted_tracks.id', 'wanted_tracks.primary_artist_id',
                   'user_events.id', 'sc_track_counters.play_count',
                   'tracks.genre', 'tracks.language', 'tracks.primary_artist_id',
                   'tracks.uploader_sc_user_id', 'tracks.sharing', 'tracks.storage_state',
                   'tracks.index_state', 'tracks.storage_priority', 'tracks.index_priority',
                   'tracks.play_count_sc', 'tracks.sc_created_at', 'artists.tags',
                   'album_tracks.album_id', 'album_tracks.track_id', 'lyrics_cache.track_id',
                   'lyrics_lookup_state.track_id', 'wanted_tracks.track_id'
               )
           );

        IF column_list IS NULL THEN
            CONTINUE;
        END IF;

        IF target = 'playlist_membership_operations' THEN
            column_list := column_list || ', track_id';
            value_list := value_list || ', n::text';
        END IF;

        IF target = 'lyrics_cache' THEN
            column_list := column_list || ', plain_text';
            value_list := value_list || $v$, 'planfixture lyrics ' || n$v$;
        END IF;

        EXECUTE format('ALTER TABLE %I DISABLE TRIGGER ALL', target);
        BEGIN
            EXECUTE format(
                'INSERT INTO %I (%s) SELECT %s FROM generate_series(1, %s) AS n ON CONFLICT DO NOTHING',
                target, column_list, value_list, rows_wanted
            );
            GET DIAGNOSTICS seeded = ROW_COUNT;
            RAISE NOTICE 'seeded % of % rows into %', seeded, rows_wanted, target;
        EXCEPTION WHEN OTHERS THEN
            failed := failed || target || ' (' || SQLERRM || '); ';
        END;
        EXECUTE format('ALTER TABLE %I ENABLE TRIGGER ALL', target);
    END LOOP;

    IF failed <> '' THEN
        RAISE NOTICE 'not seeded: %', failed;
    END IF;
END
$$;

UPDATE artists AS a
SET track_count_primary = counts.primary_tracks,
    track_count_featured = counts.featured_tracks
FROM (
    SELECT artist_id,
           count(*) FILTER (WHERE role = 'primary')  AS primary_tracks,
           count(*) FILTER (WHERE role = 'featured') AS featured_tracks
    FROM track_artists
    GROUP BY artist_id
) AS counts
WHERE a.id = counts.artist_id
  AND (a.track_count_primary, a.track_count_featured)
      IS DISTINCT FROM (counts.primary_tracks::int, counts.featured_tracks::int);

ANALYZE;
