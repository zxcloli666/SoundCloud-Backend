SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.scd_cmd_match_sc_account(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    artist uuid := (args ->> 'artist_id')::uuid;
    account text := args ->> 'sc_user_id';
    stamp timestamptz := (args ->> 'at')::timestamptz;
    key jsonb := jsonb_build_object('tbl', 'artist_sc_accounts', 'pk', jsonb_build_object('artist_id', (args ->> 'artist_id')::uuid, 'sc_user_id', args ->> 'sc_user_id'));
    artist_key jsonb := jsonb_build_object('tbl', 'artists', 'pk', jsonb_build_object('id', (args ->> 'artist_id')::uuid));
    fresh boolean;
    claims boolean;
    touched jsonb := '[]';
BEGIN
    IF args ->> 'op' = 'delete' THEN
        DELETE FROM public.artist_sc_accounts AS a
        WHERE a.artist_id = artist AND a.sc_user_id = account;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('touched', touched, 'output', false);
        END IF;
        touched := jsonb_build_array(key);
        UPDATE public.artists AS a
        SET has_sc_account = EXISTS (SELECT 1
                                     FROM public.artist_sc_accounts AS other
                                     WHERE other.artist_id = artist
                                       AND (other.verified OR other.source = 'mb_resolve')
                                       AND other.role IN ('main', 'alt', 'demo'))
        WHERE a.id = artist;
        IF FOUND THEN
            touched := touched || jsonb_build_array(artist_key);
        END IF;
        RETURN jsonb_build_object('touched', touched, 'output', true);
    END IF;
    INSERT INTO public.artist_sc_accounts AS accounts (artist_id, sc_user_id, role, source, verified, created_at, updated_at)
    VALUES (artist, account, args ->> 'role', args ->> 'source', (args ->> 'verified')::boolean, stamp, stamp)
    ON CONFLICT (artist_id, sc_user_id) DO UPDATE
        SET role       = CASE
                             WHEN EXCLUDED.verified AND NOT accounts.verified
                                 THEN EXCLUDED.role
                             ELSE accounts.role END,
            verified   = accounts.verified OR EXCLUDED.verified,
            source     = CASE
                             WHEN accounts.source = 'manual'
                                 THEN accounts.source
                             ELSE EXCLUDED.source END,
            updated_at = stamp
    RETURNING (accounts.xmax = 0), (accounts.verified OR accounts.source = 'mb_resolve') AND accounts.role IN ('main', 'alt', 'demo')
    INTO fresh, claims;
    touched := jsonb_build_array(key);
    UPDATE public.artists AS a
    SET sc_user_id = COALESCE(a.sc_user_id, account),
        has_sc_account = CASE
                             WHEN fresh THEN a.has_sc_account OR claims
                             ELSE EXISTS (SELECT 1
                                          FROM public.artist_sc_accounts AS other
                                          WHERE other.artist_id = artist
                                            AND (other.verified OR other.source = 'mb_resolve')
                                            AND other.role IN ('main', 'alt', 'demo')) END,
        updated_at = stamp
    WHERE a.id = artist;
    IF FOUND THEN
        touched := touched || jsonb_build_array(artist_key);
    END IF;
    RETURN jsonb_build_object('touched', touched, 'output', true);
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_match_wanted_link(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    wanted uuid := (args ->> 'wanted_id')::uuid;
    linked uuid;
BEGIN
    UPDATE public.wanted_tracks AS w
    SET track_id   = target.id,
        status     = 'linked',
        updated_at = (args ->> 'at')::timestamptz
    FROM (SELECT COALESCE(winner.id, track.id) AS id
          FROM public.tracks AS track
                   LEFT JOIN public.tracks AS winner ON winner.id = track.superseded_by
          WHERE track.sc_track_id = args ->> 'sc_track_id') AS target
    WHERE w.id = wanted
    RETURNING w.track_id INTO linked;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', NULL);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'wanted_tracks', 'pk', jsonb_build_object('id', wanted))),
        'output', to_jsonb(linked));
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_match_genius_song(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    track uuid := (args ->> 'track_id')::uuid;
    attached text;
BEGIN
    UPDATE public.tracks AS t
    SET genius_song_id = (args ->> 'genius_song_id')::bigint
    WHERE t.id = track
      AND t.genius_song_id IS NULL
    RETURNING t.sc_track_id INTO attached;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('touched', '[]'::jsonb, 'output', NULL);
    END IF;
    RETURN jsonb_build_object(
        'touched', jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', track))),
        'output', to_jsonb(attached));
END
$f$;

CREATE OR REPLACE FUNCTION public.scd_cmd_match_album_track(args jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
DECLARE
    track uuid := (args ->> 'track_id')::uuid;
    album record;
    inherited boolean := false;
    touched jsonb := '[]';
BEGIN
    FOR album IN
        SELECT (a.e ->> 'album_id')::uuid AS album_id, (a.e ->> 'position')::smallint AS position
        FROM jsonb_array_elements(args -> 'albums') WITH ORDINALITY AS a(e, n)
        ORDER BY a.n
    LOOP
        UPDATE public.tracks AS t
        SET album_id       = COALESCE(t.album_id, album.album_id),
            album_position = COALESCE(t.album_position, album.position)
        WHERE t.id = track
          AND (t.album_id, t.album_position)
              IS DISTINCT FROM (COALESCE(t.album_id, album.album_id), COALESCE(t.album_position, album.position));
        inherited := inherited OR FOUND;
        INSERT INTO public.album_tracks (album_id, track_id, position)
        VALUES (album.album_id, track, album.position)
        ON CONFLICT DO NOTHING;
        IF FOUND THEN
            touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'album_tracks', 'pk', jsonb_build_object('album_id', album.album_id, 'track_id', track)));
        END IF;
    END LOOP;
    IF inherited THEN
        touched := touched || jsonb_build_array(jsonb_build_object('tbl', 'tracks', 'pk', jsonb_build_object('id', track)));
    END IF;
    RETURN jsonb_build_object('touched', touched, 'output', NULL);
END
$f$;
