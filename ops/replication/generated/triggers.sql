CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_cdn_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_discover_promoted;
CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_disliked_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_oauth_apps;
CREATE SEQUENCE IF NOT EXISTS public.scd_conflict_sync_queue;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_admin_maintenance_runs;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_ai_resolver_budget;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_ai_resolver_cache;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_album_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_album_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_albums;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_attribution_revalidation;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_colike;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_coplay;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_sc_accounts;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artist_socials;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_audio_index_wire_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_background_job_enqueues;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_background_job_failures;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_background_jobs;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_background_schedules;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_audience;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_collection_cursors;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_collection_seen;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_collection_sync;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_merge_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_catalog_work_links;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_cmd_dead;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_discover_tag_counts;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_enrich_calibration;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_job_status;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_lyrics_cache;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_lyrics_embedding_wire_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_lyrics_lookup_backfill_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_lyrics_lookup_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_oauth_app_token_issuance_reservations;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_oauth_app_token_refresh_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_oauth_app_tokens;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_pipeline_event_receipts;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_legacy_membership_intents;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_membership_operations;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_membership_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_reconcile_runs;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_remote_observations;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_remote_snapshot_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_remote_snapshots;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_sweep_cursor;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlist_track_projection;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_recommendation_quality_models;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_search_terms;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_star_track_tiers;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_storage_event_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_subscription_snapshot_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_sync_queue_action_pauses;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_taste_model_versions;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_taste_schedule;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_track_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_track_comments;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_track_work_aliases;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_transcription_sync_versions;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_transcription_wire_state;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_collection_sync;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_followers;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_owned_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_owned_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_profiles;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_taste_vectors;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_user_web_profiles;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_users;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_wanted_track_albums;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_wanted_track_artists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_wanted_track_work_aliases;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_wanted_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_intent_sync_queue;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_intent_user_followings;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_intent_user_likes_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_intent_user_likes_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_soundcloud_connections;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_sync_queue;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_user_followings;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_user_likes_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_guard_result_user_likes_tracks;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_soundcloud_connections;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_sync_queue;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_followings;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_likes_playlists;
CREATE SEQUENCE IF NOT EXISTS public.scd_resend_user_likes_tracks;

CREATE OR REPLACE FUNCTION public.scd_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF 'cmd' = ANY (TG_ARGV) AND coalesce(current_setting('scd.cmd', true), '') <> '' THEN
        RETURN NULL;
    END IF;
    IF coalesce(current_setting('scd.writer', true), '') = ANY (TG_ARGV[1:]) THEN
        RETURN NULL;
    END IF;
    PERFORM nextval(TG_ARGV[0]::regclass);
    IF current_setting('scd.guard', true) = 'enforce' THEN
        RAISE EXCEPTION 'scd guard: % on public.% by writer %', TG_OP, TG_TABLE_NAME,
            coalesce(nullif(current_setting('scd.writer', true), ''), 'none')
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NULL;
END
$f$;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.admin_maintenance_runs FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_admin_maintenance_runs', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_admin_maintenance_runs()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'admin_maintenance_runs' AND p.pk = jsonb_build_object('kind', NEW.kind)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.admin_maintenance_runs t SET changed = NEW.changed, completed_at = NEW.completed_at, cursor_text = NEW.cursor_text, cursor_uuid = NEW.cursor_uuid, merged = NEW.merged, phase = NEW.phase, run_id = NEW.run_id, scanned = NEW.scanned, skipped = NEW.skipped, started_at = NEW.started_at, status = NEW.status, updated_at = NEW.updated_at WHERE t.kind = NEW.kind;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.admin_maintenance_runs FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_admin_maintenance_runs();
ALTER TABLE public.admin_maintenance_runs ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.admin_maintenance_runs FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.admin_maintenance_runs ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.ai_resolver_budget FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_ai_resolver_budget', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.ai_resolver_cache FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_ai_resolver_cache', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.album_artists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_album_artists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_album_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_artists' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        IF EXISTS (SELECT 1 FROM public.album_artists t WHERE (t.album_id, t.artist_id, t.role) = (NEW.album_id, NEW.artist_id, NEW.role)) THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.album_artists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_album_artists();
ALTER TABLE public.album_artists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.album_artists FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.album_artists ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.album_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_album_tracks', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_album_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'album_tracks' AND p.pk = jsonb_build_object('album_id', NEW.album_id, 'track_id', NEW.track_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.album_tracks t SET disc_number = NEW.disc_number, "position" = NEW."position" WHERE (t.album_id, t.track_id) = (NEW.album_id, NEW.track_id);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.album_tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_album_tracks();
ALTER TABLE public.album_tracks ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.album_tracks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.album_tracks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.albums FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_albums', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_albums()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
    k1 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'albums' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := (NEW.mb_release_id, ((NEW.mb_release_id IS NOT NULL))) IS DISTINCT FROM (OLD.mb_release_id, ((OLD.mb_release_id IS NOT NULL)));
        k1 := (NEW.spotify_album_id, ((NEW.spotify_album_id IS NOT NULL))) IS DISTINCT FROM (OLD.spotify_album_id, ((OLD.spotify_album_id IS NOT NULL)));
    END IF;
    IF k0 AND ((NEW.mb_release_id IS NOT NULL)) THEN
        FOR r IN SELECT t.id FROM public.albums t WHERE t.mb_release_id = NEW.mb_release_id AND ((t.mb_release_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_albums(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF k1 AND ((NEW.spotify_album_id IS NOT NULL)) THEN
        FOR r IN SELECT t.id FROM public.albums t WHERE t.spotify_album_id = NEW.spotify_album_id AND ((t.spotify_album_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_albums(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.albums t SET aggregates_updated_at = NEW.aggregates_updated_at, confidence = NEW.confidence, cover_url = NEW.cover_url, created_at = NEW.created_at, genius_album_id = NEW.genius_album_id, is_star_artist = NEW.is_star_artist, mb_release_id = NEW.mb_release_id, normalized_title = NEW.normalized_title, popularity_score = NEW.popularity_score, primary_artist_id = NEW.primary_artist_id, release_date = NEW.release_date, release_year = NEW.release_year, source = NEW.source, spotify_album_id = NEW.spotify_album_id, title = NEW.title, total_duration_ms = NEW.total_duration_ms, track_count = NEW.track_count, type = NEW.type, updated_at = NEW.updated_at WHERE t.id = NEW.id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.albums FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_albums();
ALTER TABLE public.albums ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.albums FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.albums ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_attribution_revalidation FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_attribution_revalidation', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_attribution_revalidation()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_attribution_revalidation' AND p.pk = jsonb_build_object('singleton', NEW.singleton)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.artist_attribution_revalidation t SET credits_removed = NEW.credits_removed, identity_completed_at = NEW.identity_completed_at, identity_cursor_account = NEW.identity_cursor_account, identity_cursor_artist = NEW.identity_cursor_artist, tracks_reset = NEW.tracks_reset, updated_at = NEW.updated_at, walker_completed_at = NEW.walker_completed_at WHERE t.singleton = NEW.singleton;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.artist_attribution_revalidation FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_attribution_revalidation();
ALTER TABLE public.artist_attribution_revalidation ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_attribution_revalidation FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_attribution_revalidation ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_colike FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_colike', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_colike()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_colike' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.artist_colike t SET co = NEW.co, updated_at = NEW.updated_at, w = NEW.w WHERE (t.a_id, t.b_id) = (NEW.a_id, NEW.b_id);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.artist_colike FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_colike();
ALTER TABLE public.artist_colike ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_colike FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_colike ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_coplay FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_coplay', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_coplay()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_coplay' AND p.pk = jsonb_build_object('a_id', NEW.a_id, 'b_id', NEW.b_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.artist_coplay t SET last_seen = NEW.last_seen, weight = NEW.weight WHERE (t.a_id, t.b_id) = (NEW.a_id, NEW.b_id);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.artist_coplay FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_coplay();
ALTER TABLE public.artist_coplay ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_coplay FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_coplay ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_sc_accounts FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_sc_accounts', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_sc_accounts()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_sc_accounts' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'sc_user_id', NEW.sc_user_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := (NEW.sc_user_id, (NEW.verified)) IS DISTINCT FROM (OLD.sc_user_id, (OLD.verified));
    END IF;
    IF k0 AND (NEW.verified) THEN
        FOR r IN SELECT t.artist_id, t.sc_user_id FROM public.artist_sc_accounts t WHERE t.sc_user_id = NEW.sc_user_id AND (t.verified) AND (t.artist_id, t.sc_user_id) IS DISTINCT FROM (NEW.artist_id, NEW.sc_user_id) LOOP
            PERFORM public.scd_purge_artist_sc_accounts(jsonb_build_object('artist_id', r.artist_id, 'sc_user_id', r.sc_user_id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.artist_sc_accounts t SET created_at = NEW.created_at, notes = NEW.notes, role = NEW.role, source = NEW.source, updated_at = NEW.updated_at, verified = NEW.verified WHERE (t.artist_id, t.sc_user_id) = (NEW.artist_id, NEW.sc_user_id);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.artist_sc_accounts FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_sc_accounts();
ALTER TABLE public.artist_sc_accounts ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_sc_accounts FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_sc_accounts ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artist_socials FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artist_socials', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artist_socials()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artist_socials' AND p.pk = jsonb_build_object('artist_id', NEW.artist_id, 'url', NEW.url)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.artist_socials t SET created_at = NEW.created_at, kind = NEW.kind, source = NEW.source, updated_at = NEW.updated_at, verified = NEW.verified WHERE (t.artist_id, t.url) = (NEW.artist_id, NEW.url);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.artist_socials FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artist_socials();
ALTER TABLE public.artist_socials ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artist_socials FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artist_socials ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.artists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_artists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
    k1 boolean := true;
    k2 boolean := true;
    k3 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'artists' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := (NEW.genius_artist_id, ((NEW.genius_artist_id IS NOT NULL))) IS DISTINCT FROM (OLD.genius_artist_id, ((OLD.genius_artist_id IS NOT NULL)));
        k1 := (NEW.mb_artist_id, ((NEW.mb_artist_id IS NOT NULL))) IS DISTINCT FROM (OLD.mb_artist_id, ((OLD.mb_artist_id IS NOT NULL)));
        k2 := (NEW.normalized_name, ((NEW.merged_into IS NULL))) IS DISTINCT FROM (OLD.normalized_name, ((OLD.merged_into IS NULL)));
        k3 := (NEW.spotify_artist_id, ((NEW.spotify_artist_id IS NOT NULL))) IS DISTINCT FROM (OLD.spotify_artist_id, ((OLD.spotify_artist_id IS NOT NULL)));
    END IF;
    IF k0 AND ((NEW.genius_artist_id IS NOT NULL)) THEN
        FOR r IN SELECT t.id FROM public.artists t WHERE t.genius_artist_id = NEW.genius_artist_id AND ((t.genius_artist_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF k1 AND ((NEW.mb_artist_id IS NOT NULL)) THEN
        FOR r IN SELECT t.id FROM public.artists t WHERE t.mb_artist_id = NEW.mb_artist_id AND ((t.mb_artist_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF k2 AND ((NEW.merged_into IS NULL)) THEN
        FOR r IN SELECT t.id FROM public.artists t WHERE t.normalized_name = NEW.normalized_name AND ((t.merged_into IS NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF k3 AND ((NEW.spotify_artist_id IS NOT NULL)) THEN
        FOR r IN SELECT t.id FROM public.artists t WHERE t.spotify_artist_id = NEW.spotify_artist_id AND ((t.spotify_artist_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_artists(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.artists t SET account_walk_locked_at = NEW.account_walk_locked_at, aggregates_updated_at = NEW.aggregates_updated_at, album_count_denorm = NEW.album_count_denorm, avatar_url = NEW.avatar_url, bio = NEW.bio, confidence = NEW.confidence, country = NEW.country, crawl_attempts = NEW.crawl_attempts, crawl_dead = NEW.crawl_dead, crawl_fail_count = NEW.crawl_fail_count, created_at = NEW.created_at, genius_artist_id = NEW.genius_artist_id, genius_crawl_offset = NEW.genius_crawl_offset, genius_crawled_at = NEW.genius_crawled_at, genius_locked_at = NEW.genius_locked_at, genius_next_run_at = NEW.genius_next_run_at, has_sc_account = NEW.has_sc_account, interest_score = NEW.interest_score, is_star = NEW.is_star, isni = NEW.isni, last_account_walk_at = NEW.last_account_walk_at, last_crawled_at = NEW.last_crawled_at, mb_artist_id = NEW.mb_artist_id, mb_crawl_offset = NEW.mb_crawl_offset, mb_crawled_at = NEW.mb_crawled_at, mb_locked_at = NEW.mb_locked_at, mb_next_run_at = NEW.mb_next_run_at, merged_into = NEW.merged_into, monthly_listeners = NEW.monthly_listeners, name = NEW.name, normalized_name = NEW.normalized_name, popularity_score = NEW.popularity_score, sc_user_id = NEW.sc_user_id, source = NEW.source, spotify_artist_id = NEW.spotify_artist_id, star_aura_id = NEW.star_aura_id, star_custom_hex = NEW.star_custom_hex, tags = NEW.tags, track_count_featured = NEW.track_count_featured, track_count_primary = NEW.track_count_primary, trending_score = NEW.trending_score, updated_at = NEW.updated_at WHERE t.id = NEW.id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.artists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_artists();
ALTER TABLE public.artists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.artists FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.artists ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.audio_index_wire_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_audio_index_wire_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.background_job_enqueues FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_background_job_enqueues', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.background_job_failures FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_background_job_failures', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_background_job_failures()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'background_job_failures' AND p.pk = jsonb_build_object('id', NEW.id, 'generation', NEW.generation)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.background_job_failures t SET attempts = NEW.attempts, created_at = NEW.created_at, dedup_key = NEW.dedup_key, failed_at = NEW.failed_at, kind = NEW.kind, lane = NEW.lane, last_error = NEW.last_error, max_attempts = NEW.max_attempts, payload = NEW.payload, priority = NEW.priority WHERE (t.id, t.generation) = (NEW.id, NEW.generation);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.background_job_failures FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_background_job_failures();
ALTER TABLE public.background_job_failures ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.background_job_failures FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.background_job_failures ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.background_jobs FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_background_jobs', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.background_schedules FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_background_schedules', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_audience FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_audience', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_catalog_audience()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_audience' AND p.pk = jsonb_build_object('subject_urn', NEW.subject_urn, 'relation', NEW.relation, 'user_urn', NEW.user_urn)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.catalog_audience t SET created_at = NEW.created_at, synced_at = NEW.synced_at WHERE (t.subject_urn, t.relation, t.user_urn) = (NEW.subject_urn, NEW.relation, NEW.user_urn);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.catalog_audience FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_catalog_audience();
ALTER TABLE public.catalog_audience ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.catalog_audience FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.catalog_audience ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_collection_cursors FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_collection_cursors', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_collection_seen FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_collection_seen', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_collection_sync FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_collection_sync', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_catalog_collection_sync()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'catalog_collection_sync' AND p.pk = jsonb_build_object('subject_id', NEW.subject_id, 'collection', NEW.collection, 'scope', NEW.scope)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.catalog_collection_sync t SET complete = NEW.complete, generation = NEW.generation, item_count = NEW.item_count, job_id = NEW.job_id, next_cursor = NEW.next_cursor, page_count = NEW.page_count, snapshot_id = NEW.snapshot_id, started_at = NEW.started_at, synced_at = NEW.synced_at, updated_at = NEW.updated_at, verified_at = NEW.verified_at WHERE (t.subject_id, t.collection, t.scope) = (NEW.subject_id, NEW.collection, NEW.scope);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.catalog_collection_sync FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_catalog_collection_sync();
ALTER TABLE public.catalog_collection_sync ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.catalog_collection_sync FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.catalog_collection_sync ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_merge_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_merge_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.catalog_work_links FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_catalog_work_links', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_cdn_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.cdn_path := OLD.cdn_path;
            NEW.created_at := OLD.created_at;
            NEW.quality := OLD.quality;
            NEW.status := OLD.status;
            NEW.track_urn := OLD.track_urn;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.cdn_tracks t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.cdn_tracks t SET cdn_path = NEW.cdn_path, created_at = NEW.created_at, quality = NEW.quality, status = NEW.status, track_urn = NEW.track_urn, updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    SELECT t.id INTO o FROM public.cdn_tracks t WHERE (t.track_urn, t.quality) = (NEW.track_urn, NEW.quality);
    IF FOUND THEN
        PERFORM nextval('public.scd_conflict_cdn_tracks'::regclass);
        UPDATE public.cdn_tracks t SET cdn_path = NEW.cdn_path, created_at = NEW.created_at, quality = NEW.quality, status = NEW.status, track_urn = NEW.track_urn, updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.id = o.id;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.cdn_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_cdn_tracks();
ALTER TABLE public.cdn_tracks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_cdn_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.cdn_path, NEW.created_at, NEW.quality, NEW.status, NEW.track_urn, NEW.updated_at) IS DISTINCT FROM (OLD.cdn_path, OLD.created_at, OLD.quality, OLD.status, OLD.track_urn, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.cdn_tracks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_cdn_tracks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.cdn_tracks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.cdn_tracks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.cmd_dead FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_cmd_dead', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_skip_diag_bundles()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM 1 FROM public.diag_bundles t WHERE t.code = NEW.code;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.diag_bundles FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_diag_bundles();
ALTER TABLE public.diag_bundles ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.diag_bundles FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.diag_bundles ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_skip_diag_events()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM 1 FROM public.diag_events t WHERE t.id = NEW.id;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.diag_events FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_diag_events();
ALTER TABLE public.diag_events ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.diag_events FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.diag_events ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_diag_issue_days()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.events := OLD.events;
            NEW.first_at := OLD.first_at;
            NEW.kind := OLD.kind;
            NEW.last_at := OLD.last_at;
            NEW.title := OLD.title;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.diag_issue_days t WHERE (t.fingerprint, t.day, t.host, t.app_version) = (NEW.fingerprint, NEW.day, NEW.host, NEW.app_version);
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.diag_issue_days t SET events = NEW.events, first_at = NEW.first_at, kind = NEW.kind, last_at = NEW.last_at, title = NEW.title, row_v = NEW.row_v WHERE (t.fingerprint, t.day, t.host, t.app_version) = (NEW.fingerprint, NEW.day, NEW.host, NEW.app_version);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.diag_issue_days FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_diag_issue_days();
ALTER TABLE public.diag_issue_days ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_diag_issue_days()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.events, NEW.first_at, NEW.kind, NEW.last_at, NEW.title) IS DISTINCT FROM (OLD.events, OLD.first_at, OLD.kind, OLD.last_at, OLD.title) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.diag_issue_days FOR EACH ROW EXECUTE FUNCTION public.scd_ver_diag_issue_days();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.diag_issue_days FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.diag_issue_days ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_skip_diag_issue_origins()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM 1 FROM public.diag_issue_origins t WHERE (t.fingerprint, t.day, t.host, t.origin) = (NEW.fingerprint, NEW.day, NEW.host, NEW.origin);
    IF FOUND THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.diag_issue_origins FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_diag_issue_origins();
ALTER TABLE public.diag_issue_origins ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.diag_issue_origins FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.diag_issue_origins ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_discover_promoted()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.active := OLD.active;
            NEW.created_at := OLD.created_at;
            NEW.entity_id := OLD.entity_id;
            NEW.entity_type := OLD.entity_type;
            NEW.note := OLD.note;
            NEW."position" := OLD."position";
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.discover_promoted t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.discover_promoted t SET active = NEW.active, created_at = NEW.created_at, entity_id = NEW.entity_id, entity_type = NEW.entity_type, note = NEW.note, "position" = NEW."position", updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    SELECT t.id INTO o FROM public.discover_promoted t WHERE (t.entity_type, t.entity_id) = (NEW.entity_type, NEW.entity_id);
    IF FOUND THEN
        PERFORM nextval('public.scd_conflict_discover_promoted'::regclass);
        UPDATE public.discover_promoted t SET active = NEW.active, created_at = NEW.created_at, entity_id = NEW.entity_id, entity_type = NEW.entity_type, note = NEW.note, "position" = NEW."position", updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.id = o.id;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.discover_promoted FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_discover_promoted();
ALTER TABLE public.discover_promoted ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_discover_promoted()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('discover_promoted', jsonb_build_object('id', OLD.id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.discover_promoted FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_discover_promoted();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_discover_promoted()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'discover_promoted' AND x.pk = jsonb_build_object('id', NEW.id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.discover_promoted FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_discover_promoted();
ALTER TABLE public.discover_promoted ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_discover_promoted()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.active, NEW.created_at, NEW.entity_id, NEW.entity_type, NEW.note, NEW."position", NEW.updated_at) IS DISTINCT FROM (OLD.active, OLD.created_at, OLD.entity_id, OLD.entity_type, OLD.note, OLD."position", OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.discover_promoted FOR EACH ROW EXECUTE FUNCTION public.scd_ver_discover_promoted();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.discover_promoted FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.discover_promoted ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_discover_settings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.show_star := OLD.show_star;
            NEW.star_limit := OLD.star_limit;
            NEW.star_strategy := OLD.star_strategy;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.discover_settings t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.discover_settings t SET show_star = NEW.show_star, star_limit = NEW.star_limit, star_strategy = NEW.star_strategy, updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.discover_settings FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_discover_settings();
ALTER TABLE public.discover_settings ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_discover_settings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.show_star, NEW.star_limit, NEW.star_strategy, NEW.updated_at) IS DISTINCT FROM (OLD.show_star, OLD.star_limit, OLD.star_strategy, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.discover_settings FOR EACH ROW EXECUTE FUNCTION public.scd_ver_discover_settings();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.discover_settings FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.discover_settings ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.discover_tag_counts FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_discover_tag_counts', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_discover_tag_counts()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'discover_tag_counts' AND p.pk = jsonb_build_object('tag', NEW.tag)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.discover_tag_counts t SET artist_count = NEW.artist_count, refreshed_at = NEW.refreshed_at WHERE t.tag = NEW.tag;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.discover_tag_counts FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_discover_tag_counts();
ALTER TABLE public.discover_tag_counts ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.discover_tag_counts FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.discover_tag_counts ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_disliked_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.created_at := OLD.created_at;
            NEW.sc_track_id := OLD.sc_track_id;
            NEW.sc_user_id := OLD.sc_user_id;
            NEW.track_data := OLD.track_data;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.disliked_tracks t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.disliked_tracks t SET created_at = NEW.created_at, sc_track_id = NEW.sc_track_id, sc_user_id = NEW.sc_user_id, track_data = NEW.track_data, row_v = NEW.row_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    SELECT t.id INTO o FROM public.disliked_tracks t WHERE (t.sc_user_id, t.sc_track_id) = (NEW.sc_user_id, NEW.sc_track_id);
    IF FOUND THEN
        PERFORM nextval('public.scd_conflict_disliked_tracks'::regclass);
        UPDATE public.disliked_tracks t SET created_at = NEW.created_at, sc_track_id = NEW.sc_track_id, sc_user_id = NEW.sc_user_id, track_data = NEW.track_data, row_v = NEW.row_v WHERE t.id = o.id;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.disliked_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_disliked_tracks();
ALTER TABLE public.disliked_tracks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_disliked_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('disliked_tracks', jsonb_build_object('id', OLD.id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.disliked_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_disliked_tracks();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_disliked_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'disliked_tracks' AND x.pk = jsonb_build_object('id', NEW.id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.disliked_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_disliked_tracks();
ALTER TABLE public.disliked_tracks ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_disliked_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.created_at, NEW.sc_track_id, NEW.sc_user_id, NEW.track_data) IS DISTINCT FROM (OLD.created_at, OLD.sc_track_id, OLD.sc_user_id, OLD.track_data) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.disliked_tracks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_disliked_tracks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.disliked_tracks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.disliked_tracks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.enrich_calibration FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_enrich_calibration', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_max_entity_reads()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        UPDATE public.entity_reads t SET last_read_at = NEW.last_read_at WHERE t.entity_urn = NEW.entity_urn AND (t.last_read_at IS NULL OR t.last_read_at < NEW.last_read_at);
        IF FOUND THEN
            RETURN NULL;
        END IF;
        PERFORM 1 FROM public.entity_reads t WHERE t.entity_urn = NEW.entity_urn;
        IF FOUND THEN
            RETURN NULL;
        END IF;
        RETURN NEW;
    END IF;
    IF OLD.last_read_at IS NOT NULL AND (NEW.last_read_at IS NULL OR NEW.last_read_at <= OLD.last_read_at) THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_max BEFORE INSERT OR UPDATE ON public.entity_reads FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_max_entity_reads();
ALTER TABLE public.entity_reads ENABLE REPLICA TRIGGER zz_max;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.entity_reads FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.entity_reads ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_featured_items()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.active := OLD.active;
            NEW.created_at := OLD.created_at;
            NEW.sc_urn := OLD.sc_urn;
            NEW.type := OLD.type;
            NEW.weight := OLD.weight;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.featured_items t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.featured_items t SET active = NEW.active, created_at = NEW.created_at, sc_urn = NEW.sc_urn, type = NEW.type, weight = NEW.weight, row_v = NEW.row_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.featured_items FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_featured_items();
ALTER TABLE public.featured_items ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_featured_items()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('featured_items', jsonb_build_object('id', OLD.id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.featured_items FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_featured_items();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_featured_items()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'featured_items' AND x.pk = jsonb_build_object('id', NEW.id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.featured_items FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_featured_items();
ALTER TABLE public.featured_items ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_featured_items()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.active, NEW.created_at, NEW.sc_urn, NEW.type, NEW.weight) IS DISTINCT FROM (OLD.active, OLD.created_at, OLD.sc_urn, OLD.type, OLD.weight) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.featured_items FOR EACH ROW EXECUTE FUNCTION public.scd_ver_featured_items();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.featured_items FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.featured_items ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.job_status FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_job_status', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_job_status()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'job_status' AND p.pk = jsonb_build_object('kind', NEW.kind, 'dedup_key', NEW.dedup_key)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.job_status t SET available_at = NEW.available_at, state = NEW.state, updated_at = NEW.updated_at WHERE (t.kind, t.dedup_key) = (NEW.kind, NEW.dedup_key);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.job_status FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_job_status();
ALTER TABLE public.job_status ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.job_status FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.job_status ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_link_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.claim_token := OLD.claim_token;
            NEW.created_at := OLD.created_at;
            NEW.error := OLD.error;
            NEW.expires_at := OLD.expires_at;
            NEW.mode := OLD.mode;
            NEW.source_session_id := OLD.source_session_id;
            NEW.status := OLD.status;
            NEW.target_session_id := OLD.target_session_id;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.link_requests t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.link_requests t SET claim_token = NEW.claim_token, created_at = NEW.created_at, error = NEW.error, expires_at = NEW.expires_at, mode = NEW.mode, source_session_id = NEW.source_session_id, status = NEW.status, target_session_id = NEW.target_session_id, row_v = NEW.row_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.link_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_link_requests();
ALTER TABLE public.link_requests ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_link_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('link_requests', jsonb_build_object('id', OLD.id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.link_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_link_requests();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_link_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'link_requests' AND x.pk = jsonb_build_object('id', NEW.id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.link_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_link_requests();
ALTER TABLE public.link_requests ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_link_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.claim_token, NEW.created_at, NEW.error, NEW.expires_at, NEW.mode, NEW.source_session_id, NEW.status, NEW.target_session_id) IS DISTINCT FROM (OLD.claim_token, OLD.created_at, OLD.error, OLD.expires_at, OLD.mode, OLD.source_session_id, OLD.status, OLD.target_session_id) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.link_requests FOR EACH ROW EXECUTE FUNCTION public.scd_ver_link_requests();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.link_requests FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.link_requests ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_skip_listening_history()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM 1 FROM public.listening_history t WHERE t.id = NEW.id;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.listening_history FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_listening_history();
ALTER TABLE public.listening_history ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE FUNCTION public.scd_tomb_listening_history()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('listening_history', jsonb_build_object('id', OLD.id), 0, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.listening_history FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_listening_history();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_listening_history()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'listening_history' AND x.pk = jsonb_build_object('id', NEW.id);
    IF FOUND THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.listening_history FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_listening_history();
ALTER TABLE public.listening_history ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.listening_history FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.listening_history ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_login_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.code_verifier := OLD.code_verifier;
            NEW.created_at := OLD.created_at;
            NEW.error := OLD.error;
            NEW.expires_at := OLD.expires_at;
            NEW.oauth_app_id := OLD.oauth_app_id;
            NEW.profile_ok := OLD.profile_ok;
            NEW.redirect_url := OLD.redirect_url;
            NEW.result_session_id := OLD.result_session_id;
            NEW.retry_count := OLD.retry_count;
            NEW.state := OLD.state;
            NEW.status := OLD.status;
            NEW.step := OLD.step;
            NEW.target_session_id := OLD.target_session_id;
            NEW.username := OLD.username;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.login_requests t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.login_requests t SET code_verifier = NEW.code_verifier, created_at = NEW.created_at, error = NEW.error, expires_at = NEW.expires_at, oauth_app_id = NEW.oauth_app_id, profile_ok = NEW.profile_ok, redirect_url = NEW.redirect_url, result_session_id = NEW.result_session_id, retry_count = NEW.retry_count, state = NEW.state, status = NEW.status, step = NEW.step, target_session_id = NEW.target_session_id, username = NEW.username, row_v = NEW.row_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.login_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_login_requests();
ALTER TABLE public.login_requests ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_login_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('login_requests', jsonb_build_object('id', OLD.id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.login_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_login_requests();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_login_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'login_requests' AND x.pk = jsonb_build_object('id', NEW.id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.login_requests FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_login_requests();
ALTER TABLE public.login_requests ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_login_requests()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.code_verifier, NEW.created_at, NEW.error, NEW.expires_at, NEW.oauth_app_id, NEW.profile_ok, NEW.redirect_url, NEW.result_session_id, NEW.retry_count, NEW.state, NEW.status, NEW.step, NEW.target_session_id, NEW.username) IS DISTINCT FROM (OLD.code_verifier, OLD.created_at, OLD.error, OLD.expires_at, OLD.oauth_app_id, OLD.profile_ok, OLD.redirect_url, OLD.result_session_id, OLD.retry_count, OLD.state, OLD.status, OLD.step, OLD.target_session_id, OLD.username) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.login_requests FOR EACH ROW EXECUTE FUNCTION public.scd_ver_login_requests();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.login_requests FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.login_requests ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.lyrics_cache FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_lyrics_cache', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_lyrics_cache()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_cache' AND p.pk = jsonb_build_object('sc_track_id', NEW.sc_track_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.lyrics_cache t SET content_generation = NEW.content_generation, content_hash = NEW.content_hash, created_at = NEW.created_at, embedded_at = NEW.embedded_at, embedding_state = NEW.embedding_state, fts = NEW.fts, language = NEW.language, language_confidence = NEW.language_confidence, plain_source = NEW.plain_source, plain_text = NEW.plain_text, source = NEW.source, synced_lrc = NEW.synced_lrc, synced_source = NEW.synced_source, synced_version = NEW.synced_version, track_id = NEW.track_id, updated_at = NEW.updated_at WHERE t.sc_track_id = NEW.sc_track_id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.lyrics_cache FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_lyrics_cache();
ALTER TABLE public.lyrics_cache ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.lyrics_cache FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.lyrics_cache ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.lyrics_embedding_wire_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_lyrics_embedding_wire_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.lyrics_lookup_backfill_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_lyrics_lookup_backfill_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.lyrics_lookup_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_lyrics_lookup_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_lyrics_lookup_state()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'lyrics_lookup_state' AND p.pk = jsonb_build_object('track_id', NEW.track_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := NEW.sc_track_id IS DISTINCT FROM OLD.sc_track_id;
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.track_id FROM public.lyrics_lookup_state t WHERE t.sc_track_id = NEW.sc_track_id AND t.track_id IS DISTINCT FROM NEW.track_id LOOP
            PERFORM public.scd_purge_lyrics_lookup_state(jsonb_build_object('track_id', r.track_id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.lyrics_lookup_state t SET algorithm_version = NEW.algorithm_version, attempts = NEW.attempts, claim_expires_at = NEW.claim_expires_at, claim_job_generation = NEW.claim_job_generation, claim_job_id = NEW.claim_job_id, claim_job_lease_id = NEW.claim_job_lease_id, claim_state_generation = NEW.claim_state_generation, created_at = NEW.created_at, failure_streak = NEW.failure_streak, generation = NEW.generation, input_artist = NEW.input_artist, input_duration_ms = NEW.input_duration_ms, input_genius_song_id = NEW.input_genius_song_id, input_genius_url = NEW.input_genius_url, input_release_date = NEW.input_release_date, input_sc_created_at = NEW.input_sc_created_at, input_title = NEW.input_title, last_attempt_at = NEW.last_attempt_at, last_error = NEW.last_error, last_outcome = NEW.last_outcome, miss_streak = NEW.miss_streak, next_run_at = NEW.next_run_at, priority = NEW.priority, retry_after_at = NEW.retry_after_at, sc_track_id = NEW.sc_track_id, source_set_version = NEW.source_set_version, status = NEW.status, updated_at = NEW.updated_at, wake_durable_at = NEW.wake_durable_at, wake_generation = NEW.wake_generation, wake_message_id = NEW.wake_message_id WHERE t.track_id = NEW.track_id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.lyrics_lookup_state FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_lyrics_lookup_state();
ALTER TABLE public.lyrics_lookup_state ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.lyrics_lookup_state FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.lyrics_lookup_state ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_oauth_app_request_cooldowns()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.failure_count := OLD.failure_count;
            NEW.retry_at := OLD.retry_at;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.oauth_app_request_cooldowns t WHERE t.oauth_app_id = NEW.oauth_app_id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.oauth_app_request_cooldowns t SET failure_count = NEW.failure_count, retry_at = NEW.retry_at, updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.oauth_app_id = NEW.oauth_app_id;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.oauth_app_request_cooldowns FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_oauth_app_request_cooldowns();
ALTER TABLE public.oauth_app_request_cooldowns ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_oauth_app_request_cooldowns()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.failure_count, NEW.retry_at, NEW.updated_at) IS DISTINCT FROM (OLD.failure_count, OLD.retry_at, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.oauth_app_request_cooldowns FOR EACH ROW EXECUTE FUNCTION public.scd_ver_oauth_app_request_cooldowns();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.oauth_app_request_cooldowns FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.oauth_app_request_cooldowns ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.oauth_app_token_issuance_reservations FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_oauth_app_token_issuance_reservations', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.oauth_app_token_refresh_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_oauth_app_token_refresh_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.oauth_app_tokens FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_oauth_app_tokens', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_oauth_app_tokens()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'oauth_app_tokens' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.oauth_app_tokens t SET access_token = NEW.access_token, expires_at = NEW.expires_at, generation = NEW.generation, last_refresh_error = NEW.last_refresh_error, last_used_at = NEW.last_used_at, refresh_attempts = NEW.refresh_attempts, refresh_token = NEW.refresh_token, refreshed_at = NEW.refreshed_at, scope = NEW.scope WHERE t.oauth_app_id = NEW.oauth_app_id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.oauth_app_tokens FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_oauth_app_tokens();
ALTER TABLE public.oauth_app_tokens ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.oauth_app_tokens FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.oauth_app_tokens ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_oauth_apps()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.active := OLD.active;
            NEW.client_id := OLD.client_id;
            NEW.client_secret := OLD.client_secret;
            NEW.created_at := OLD.created_at;
            NEW.name := OLD.name;
            NEW.redirect_uri := OLD.redirect_uri;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.oauth_apps t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.oauth_apps t SET active = NEW.active, client_id = NEW.client_id, client_secret = NEW.client_secret, created_at = NEW.created_at, name = NEW.name, redirect_uri = NEW.redirect_uri, updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    SELECT t.id INTO o FROM public.oauth_apps t WHERE (btrim(t.client_id, E'\u0009\u000A\u000B\u000C\u000D \u0085\u00A0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200A\u2028\u2029\u202F\u205F\u3000'::text)) = (btrim(NEW.client_id, E'\u0009\u000A\u000B\u000C\u000D \u0085\u00A0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200A\u2028\u2029\u202F\u205F\u3000'::text));
    IF FOUND THEN
        PERFORM nextval('public.scd_conflict_oauth_apps'::regclass);
        UPDATE public.oauth_apps t SET active = NEW.active, client_id = NEW.client_id, client_secret = NEW.client_secret, created_at = NEW.created_at, name = NEW.name, redirect_uri = NEW.redirect_uri, updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.id = o.id;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.oauth_apps FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_oauth_apps();
ALTER TABLE public.oauth_apps ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_oauth_apps()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('oauth_apps', jsonb_build_object('id', OLD.id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.oauth_apps FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_oauth_apps();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_oauth_apps()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'oauth_apps' AND x.pk = jsonb_build_object('id', NEW.id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.oauth_apps FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_oauth_apps();
ALTER TABLE public.oauth_apps ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_oauth_apps()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.active, NEW.client_id, NEW.client_secret, NEW.created_at, NEW.name, NEW.redirect_uri, NEW.updated_at) IS DISTINCT FROM (OLD.active, OLD.client_id, OLD.client_secret, OLD.created_at, OLD.name, OLD.redirect_uri, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.oauth_apps FOR EACH ROW EXECUTE FUNCTION public.scd_ver_oauth_apps();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.oauth_apps FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.oauth_apps ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.pipeline_event_receipts FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_pipeline_event_receipts', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_legacy_membership_intents FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_legacy_membership_intents', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_legacy_membership_intents()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_legacy_membership_intents' AND p.pk = jsonb_build_object('archive_id', NEW.archive_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := NEW.queue_id IS DISTINCT FROM OLD.queue_id;
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.archive_id FROM public.playlist_legacy_membership_intents t WHERE t.queue_id = NEW.queue_id AND t.archive_id IS DISTINCT FROM NEW.archive_id LOOP
            PERFORM public.scd_purge_playlist_legacy_membership_intents(jsonb_build_object('archive_id', r.archive_id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.playlist_legacy_membership_intents t SET archived_at = NEW.archived_at, classification = NEW.classification, legacy_desired_revision = NEW.legacy_desired_revision, legacy_synced_revision = NEW.legacy_synced_revision, legacy_user_id = NEW.legacy_user_id, local_snapshot_id = NEW.local_snapshot_id, playlist_urn = NEW.playlist_urn, prior_classification = NEW.prior_classification, queue_created_at = NEW.queue_created_at, queue_failed_at = NEW.queue_failed_at, queue_generation = NEW.queue_generation, queue_id = NEW.queue_id, queue_last_error = NEW.queue_last_error, queue_next_run_at = NEW.queue_next_run_at, queue_payload = NEW.queue_payload, queue_retry_count = NEW.queue_retry_count, remote_attempted_generation = NEW.remote_attempted_generation, remote_completed_generation = NEW.remote_completed_generation, remote_result = NEW.remote_result, resolved_at = NEW.resolved_at, source = NEW.source WHERE t.archive_id = NEW.archive_id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.playlist_legacy_membership_intents FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_legacy_membership_intents();
ALTER TABLE public.playlist_legacy_membership_intents ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_legacy_membership_intents FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_legacy_membership_intents ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_membership_operations FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_membership_operations', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_membership_operations()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
    k1 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_operations' AND p.pk = jsonb_build_object('operation_id', NEW.operation_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := (NEW.playlist_urn, NEW.idempotency_key) IS DISTINCT FROM (OLD.playlist_urn, OLD.idempotency_key);
        k1 := (NEW.playlist_urn, NEW.sequence) IS DISTINCT FROM (OLD.playlist_urn, OLD.sequence);
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.operation_id FROM public.playlist_membership_operations t WHERE (t.playlist_urn, t.idempotency_key) = (NEW.playlist_urn, NEW.idempotency_key) AND t.operation_id IS DISTINCT FROM NEW.operation_id LOOP
            PERFORM public.scd_purge_playlist_membership_operations(jsonb_build_object('operation_id', r.operation_id));
        END LOOP;
    END IF;
    IF k1 THEN
        FOR r IN SELECT t.operation_id FROM public.playlist_membership_operations t WHERE (t.playlist_urn, t.sequence) = (NEW.playlist_urn, NEW.sequence) AND t.operation_id IS DISTINCT FROM NEW.operation_id LOOP
            PERFORM public.scd_purge_playlist_membership_operations(jsonb_build_object('operation_id', r.operation_id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.playlist_membership_operations t SET accepted_projection_revision = NEW.accepted_projection_revision, actor_sc_user_id = NEW.actor_sc_user_id, base_baseline_generation = NEW.base_baseline_generation, base_observation_id = NEW.base_observation_id, boundary = NEW.boundary, conflict_code = NEW.conflict_code, created_at = NEW.created_at, expected_projection_revision = NEW.expected_projection_revision, idempotency_key = NEW.idempotency_key, kind = NEW.kind, left_anchor_track_id = NEW.left_anchor_track_id, ordered_track_ids = NEW.ordered_track_ids, outcome = NEW.outcome, playlist_urn = NEW.playlist_urn, request_fingerprint = NEW.request_fingerprint, resolved_at = NEW.resolved_at, right_anchor_track_id = NEW.right_anchor_track_id, sequence = NEW.sequence, track_id = NEW.track_id WHERE t.operation_id = NEW.operation_id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.playlist_membership_operations FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_membership_operations();
ALTER TABLE public.playlist_membership_operations ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_membership_operations FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_membership_operations ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_membership_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_membership_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_membership_state()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_membership_state' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.playlist_membership_state t SET baseline_generation = NEW.baseline_generation, baseline_observation_id = NEW.baseline_observation_id, candidate_fingerprint = NEW.candidate_fingerprint, committed_operation_sequence = NEW.committed_operation_sequence, conflict_code = NEW.conflict_code, last_error = NEW.last_error, last_operation_sequence = NEW.last_operation_sequence, latest_observation_id = NEW.latest_observation_id, next_reconcile_at = NEW.next_reconcile_at, owner_sc_user_id = NEW.owner_sc_user_id, projection_revision = NEW.projection_revision, projection_track_count = NEW.projection_track_count, reconcile_failure_streak = NEW.reconcile_failure_streak, reconcile_generation = NEW.reconcile_generation, remote_applied_at = NEW.remote_applied_at, remote_apply_fingerprint = NEW.remote_apply_fingerprint, remote_apply_generation = NEW.remote_apply_generation, sync_status = NEW.sync_status, updated_at = NEW.updated_at WHERE t.playlist_urn = NEW.playlist_urn;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.playlist_membership_state FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_membership_state();
ALTER TABLE public.playlist_membership_state ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_membership_state FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_membership_state ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_reconcile_runs FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_reconcile_runs', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_remote_observations FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_remote_observations', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_remote_observations()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_remote_observations' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := (NEW.playlist_urn, NEW.id) IS DISTINCT FROM (OLD.playlist_urn, OLD.id);
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.id FROM public.playlist_remote_observations t WHERE (t.playlist_urn, t.id) = (NEW.playlist_urn, NEW.id) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_playlist_remote_observations(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.playlist_remote_observations t SET all_items_identified = NEW.all_items_identified, authority = NEW.authority, declared_track_count = NEW.declared_track_count, error_kind = NEW.error_kind, observed_at = NEW.observed_at, observed_track_count = NEW.observed_track_count, outcome = NEW.outcome, pagination_complete = NEW.pagination_complete, playlist_urn = NEW.playlist_urn, retry_at = NEW.retry_at, sc_last_modified = NEW.sc_last_modified, snapshot_id = NEW.snapshot_id WHERE t.id = NEW.id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.playlist_remote_observations FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_remote_observations();
ALTER TABLE public.playlist_remote_observations ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_remote_observations FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_remote_observations ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_remote_snapshot_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_remote_snapshot_tracks', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_remote_snapshots FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_remote_snapshots', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_sweep_cursor FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_sweep_cursor', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlist_track_projection FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlist_track_projection', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlist_track_projection()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlist_track_projection' AND p.pk = jsonb_build_object('playlist_urn', NEW.playlist_urn, 'position', NEW."position")) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := (NEW.playlist_urn, NEW.sc_track_id) IS DISTINCT FROM (OLD.playlist_urn, OLD.sc_track_id);
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.playlist_urn, t."position" FROM public.playlist_track_projection t WHERE (t.playlist_urn, t.sc_track_id) = (NEW.playlist_urn, NEW.sc_track_id) AND (t.playlist_urn, t."position") IS DISTINCT FROM (NEW.playlist_urn, NEW."position") LOOP
            PERFORM public.scd_purge_playlist_track_projection(jsonb_build_object('playlist_urn', r.playlist_urn, 'position', r."position"));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.playlist_track_projection t SET sc_track_id = NEW.sc_track_id WHERE (t.playlist_urn, t."position") = (NEW.playlist_urn, NEW."position");
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.playlist_track_projection FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlist_track_projection();
ALTER TABLE public.playlist_track_projection ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlist_track_projection FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlist_track_projection ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.playlists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_playlists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'playlists' AND p.pk = jsonb_build_object('urn', NEW.urn)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := NEW.sc_playlist_id IS DISTINCT FROM OLD.sc_playlist_id;
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.urn FROM public.playlists t WHERE t.sc_playlist_id = NEW.sc_playlist_id AND t.urn IS DISTINCT FROM NEW.urn LOOP
            PERFORM public.scd_purge_playlists(jsonb_build_object('urn', r.urn));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.playlists t SET artwork_url = NEW.artwork_url, created_at = NEW.created_at, deleted_at = NEW.deleted_at, description = NEW.description, duration_ms = NEW.duration_ms, genre = NEW.genre, kind = NEW.kind, label_name = NEW.label_name, likes_count_sc = NEW.likes_count_sc, owner_sc_user_id = NEW.owner_sc_user_id, owner_urn = NEW.owner_urn, owner_username = NEW.owner_username, permalink_url = NEW.permalink_url, playlist_type = NEW.playlist_type, release_date = NEW.release_date, release_year = NEW.release_year, reposts_count_sc = NEW.reposts_count_sc, sc_created_at = NEW.sc_created_at, sc_desired = NEW.sc_desired, sc_last_modified = NEW.sc_last_modified, sc_metadata = NEW.sc_metadata, sc_mutation_observation = NEW.sc_mutation_observation, sc_observation = NEW.sc_observation, sc_playlist_id = NEW.sc_playlist_id, sc_synced_at = NEW.sc_synced_at, sc_write_confirmed = NEW.sc_write_confirmed, sharing = NEW.sharing, tags = NEW.tags, title = NEW.title, title_normalized = NEW.title_normalized, track_count = NEW.track_count, updated_at = NEW.updated_at WHERE t.urn = NEW.urn;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.playlists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_playlists();
ALTER TABLE public.playlists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.playlists FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.playlists ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.recommendation_quality_models FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_recommendation_quality_models', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_max_repl_checksums()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        UPDATE public.repl_checksums t SET at = NEW.at, hash = NEW.hash WHERE (t.node, t.tbl, t.range) = (NEW.node, NEW.tbl, NEW.range) AND (t.at IS NULL OR t.at < NEW.at);
        IF FOUND THEN
            RETURN NULL;
        END IF;
        PERFORM 1 FROM public.repl_checksums t WHERE (t.node, t.tbl, t.range) = (NEW.node, NEW.tbl, NEW.range);
        IF FOUND THEN
            RETURN NULL;
        END IF;
        RETURN NEW;
    END IF;
    IF OLD.at IS NOT NULL AND (NEW.at IS NULL OR NEW.at <= OLD.at) THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_max BEFORE INSERT OR UPDATE ON public.repl_checksums FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_max_repl_checksums();
ALTER TABLE public.repl_checksums ENABLE REPLICA TRIGGER zz_max;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.repl_checksums FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.repl_checksums ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_repl_tombstones()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.v <= OLD.v THEN
            NEW.at := OLD.at;
            NEW.v := OLD.v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.v INTO o FROM public.repl_tombstones t WHERE (t.tbl, t.pk) = (NEW.tbl, NEW.pk);
    IF FOUND THEN
        IF NEW.v > o.v THEN
            UPDATE public.repl_tombstones t SET at = NEW.at, v = NEW.v WHERE (t.tbl, t.pk) = (NEW.tbl, NEW.pk);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.repl_tombstones FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_repl_tombstones();
ALTER TABLE public.repl_tombstones ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_repl_tombstones()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.v := public.scd_hlc(NEW.v);
        RETURN NEW;
    END IF;
    IF NEW.at IS DISTINCT FROM OLD.at THEN
        NEW.v := public.scd_hlc(greatest(OLD.v, NEW.v));
    ELSE
        NEW.v := greatest(OLD.v, NEW.v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.repl_tombstones FOR EACH ROW EXECUTE FUNCTION public.scd_ver_repl_tombstones();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.repl_tombstones FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.repl_tombstones ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_sc_egress_health()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.open_until := OLD.open_until;
            NEW.opened_at := OLD.opened_at;
            NEW.opened_by := OLD.opened_by;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.sc_egress_health t WHERE t.channel = NEW.channel;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.sc_egress_health t SET open_until = NEW.open_until, opened_at = NEW.opened_at, opened_by = NEW.opened_by, row_v = NEW.row_v WHERE t.channel = NEW.channel;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.sc_egress_health FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_sc_egress_health();
ALTER TABLE public.sc_egress_health ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_sc_egress_health()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('sc_egress_health', jsonb_build_object('channel', OLD.channel), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.sc_egress_health FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_sc_egress_health();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_sc_egress_health()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'sc_egress_health' AND x.pk = jsonb_build_object('channel', NEW.channel);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.sc_egress_health FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_sc_egress_health();
ALTER TABLE public.sc_egress_health ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_sc_egress_health()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.open_until, NEW.opened_at, NEW.opened_by) IS DISTINCT FROM (OLD.open_until, OLD.opened_at, OLD.opened_by) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.sc_egress_health FOR EACH ROW EXECUTE FUNCTION public.scd_ver_sc_egress_health();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sc_egress_health FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sc_egress_health ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_max_sc_track_counters()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        UPDATE public.sc_track_counters t SET comment_count = NEW.comment_count, fetched_at = NEW.fetched_at, likes_count = NEW.likes_count, play_count = NEW.play_count, reposts_count = NEW.reposts_count WHERE t.sc_track_id = NEW.sc_track_id AND (t.fetched_at IS NULL OR t.fetched_at < NEW.fetched_at);
        IF FOUND THEN
            RETURN NULL;
        END IF;
        PERFORM 1 FROM public.sc_track_counters t WHERE t.sc_track_id = NEW.sc_track_id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
        RETURN NEW;
    END IF;
    IF OLD.fetched_at IS NOT NULL AND (NEW.fetched_at IS NULL OR NEW.fetched_at <= OLD.fetched_at) THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_max BEFORE INSERT OR UPDATE ON public.sc_track_counters FOR EACH ROW WHEN (public.scd_role() = 'core' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_max_sc_track_counters();
ALTER TABLE public.sc_track_counters ENABLE REPLICA TRIGGER zz_max;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sc_track_counters FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sc_track_counters ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.search_terms FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_search_terms', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_search_terms()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'search_terms' AND p.pk = jsonb_build_object('word', NEW.word)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.search_terms t SET latin = NEW.latin, ndoc = NEW.ndoc WHERE t.word = NEW.word;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.search_terms FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_search_terms();
ALTER TABLE public.search_terms ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.search_terms FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.search_terms ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_sessions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.created_at := OLD.created_at;
            NEW.soundcloud_connection_id := OLD.soundcloud_connection_id;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.sessions t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.sessions t SET created_at = NEW.created_at, soundcloud_connection_id = NEW.soundcloud_connection_id, updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.sessions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_sessions();
ALTER TABLE public.sessions ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_sessions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('sessions', jsonb_build_object('id', OLD.id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.sessions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_sessions();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_sessions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'sessions' AND x.pk = jsonb_build_object('id', NEW.id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.sessions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_sessions();
ALTER TABLE public.sessions ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_sessions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.created_at, NEW.soundcloud_connection_id, NEW.updated_at) IS DISTINCT FROM (OLD.created_at, OLD.soundcloud_connection_id, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.sessions FOR EACH ROW EXECUTE FUNCTION public.scd_ver_sessions();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sessions FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sessions ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF first_refresh_rejection_at, last_refresh_attempt_at, last_refresh_error, last_refresh_error_kind, last_refresh_success_at, refresh_failure_count, refresh_generation, refresh_lease_expires_at, refresh_lease_id, refresh_rejection_count, refresh_v, retry_at, updated_at ON public.soundcloud_connections FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_soundcloud_connections', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_soundcloud_connections()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.refresh_v <= OLD.refresh_v THEN
            NEW.refresh_generation := OLD.refresh_generation;
            NEW.refresh_failure_count := OLD.refresh_failure_count;
            NEW.refresh_lease_id := OLD.refresh_lease_id;
            NEW.refresh_lease_expires_at := OLD.refresh_lease_expires_at;
            NEW.last_refresh_attempt_at := OLD.last_refresh_attempt_at;
            NEW.last_refresh_success_at := OLD.last_refresh_success_at;
            NEW.last_refresh_error_kind := OLD.last_refresh_error_kind;
            NEW.last_refresh_error := OLD.last_refresh_error;
            NEW.retry_at := OLD.retry_at;
            NEW.refresh_rejection_count := OLD.refresh_rejection_count;
            NEW.first_refresh_rejection_at := OLD.first_refresh_rejection_at;
            NEW.updated_at := OLD.updated_at;
            NEW.refresh_v := OLD.refresh_v;
        END IF;
        IF NEW.token_v <= OLD.token_v THEN
            NEW.soundcloud_user_id := OLD.soundcloud_user_id;
            NEW.username := OLD.username;
            NEW.created_at := OLD.created_at;
            NEW.oauth_app_id := OLD.oauth_app_id;
            NEW.access_token := OLD.access_token;
            NEW.refresh_token := OLD.refresh_token;
            NEW.expires_at := OLD.expires_at;
            NEW.scope := OLD.scope;
            NEW.token_v := OLD.token_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.refresh_v, t.token_v INTO o FROM public.soundcloud_connections t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.refresh_v > o.refresh_v OR NEW.token_v > o.token_v THEN
            UPDATE public.soundcloud_connections t SET refresh_generation = NEW.refresh_generation, refresh_failure_count = NEW.refresh_failure_count, refresh_lease_id = NEW.refresh_lease_id, refresh_lease_expires_at = NEW.refresh_lease_expires_at, last_refresh_attempt_at = NEW.last_refresh_attempt_at, last_refresh_success_at = NEW.last_refresh_success_at, last_refresh_error_kind = NEW.last_refresh_error_kind, last_refresh_error = NEW.last_refresh_error, retry_at = NEW.retry_at, refresh_rejection_count = NEW.refresh_rejection_count, first_refresh_rejection_at = NEW.first_refresh_rejection_at, updated_at = NEW.updated_at, refresh_v = NEW.refresh_v, soundcloud_user_id = NEW.soundcloud_user_id, username = NEW.username, created_at = NEW.created_at, oauth_app_id = NEW.oauth_app_id, access_token = NEW.access_token, refresh_token = NEW.refresh_token, expires_at = NEW.expires_at, scope = NEW.scope, token_v = NEW.token_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.soundcloud_connections FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_soundcloud_connections();
ALTER TABLE public.soundcloud_connections ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_soundcloud_connections()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'soundcloud_connections' AND x.pk = jsonb_build_object('id', OLD.id);
    IF OLD.token_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('soundcloud_connections', jsonb_build_object('id', OLD.id)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_soundcloud_connections'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.soundcloud_connections FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_soundcloud_connections();
ALTER TABLE public.soundcloud_connections ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_soundcloud_connections()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF public.scd_role() = 'core' AND coalesce(current_setting('scd.writer', true), '') <> 'api' THEN
            NEW.refresh_v := public.scd_hlc(NEW.refresh_v);
        END IF;
        NEW.token_v := public.scd_hlc(NEW.token_v);
        RETURN NEW;
    END IF;
    IF (NEW.refresh_generation, NEW.refresh_failure_count, NEW.refresh_lease_id, NEW.refresh_lease_expires_at, NEW.last_refresh_attempt_at, NEW.last_refresh_success_at, NEW.last_refresh_error_kind, NEW.last_refresh_error, NEW.retry_at, NEW.refresh_rejection_count, NEW.first_refresh_rejection_at, NEW.updated_at) IS DISTINCT FROM (OLD.refresh_generation, OLD.refresh_failure_count, OLD.refresh_lease_id, OLD.refresh_lease_expires_at, OLD.last_refresh_attempt_at, OLD.last_refresh_success_at, OLD.last_refresh_error_kind, OLD.last_refresh_error, OLD.retry_at, OLD.refresh_rejection_count, OLD.first_refresh_rejection_at, OLD.updated_at) THEN
        NEW.refresh_v := public.scd_hlc(greatest(OLD.refresh_v, NEW.refresh_v));
    ELSE
        NEW.refresh_v := greatest(OLD.refresh_v, NEW.refresh_v);
    END IF;
    IF (NEW.soundcloud_user_id, NEW.username, NEW.created_at, NEW.oauth_app_id, NEW.access_token, NEW.refresh_token, NEW.expires_at, NEW.scope) IS DISTINCT FROM (OLD.soundcloud_user_id, OLD.username, OLD.created_at, OLD.oauth_app_id, OLD.access_token, OLD.refresh_token, OLD.expires_at, OLD.scope) THEN
        NEW.token_v := public.scd_hlc(greatest(OLD.token_v, NEW.token_v));
    ELSE
        NEW.token_v := greatest(OLD.token_v, NEW.token_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.soundcloud_connections FOR EACH ROW EXECUTE FUNCTION public.scd_ver_soundcloud_connections();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.soundcloud_connections FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.soundcloud_connections ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.star_track_tiers FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_star_track_tiers', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.storage_event_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_storage_event_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.subscription_snapshot_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_subscription_snapshot_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_subscriptions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.exp_date := OLD.exp_date;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.subscriptions t WHERE t.user_urn = NEW.user_urn;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.subscriptions t SET exp_date = NEW.exp_date, row_v = NEW.row_v WHERE t.user_urn = NEW.user_urn;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.subscriptions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_subscriptions();
ALTER TABLE public.subscriptions ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_subscriptions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('subscriptions', jsonb_build_object('user_urn', OLD.user_urn), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.subscriptions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_subscriptions();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_subscriptions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'subscriptions' AND x.pk = jsonb_build_object('user_urn', NEW.user_urn);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.subscriptions FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_subscriptions();
ALTER TABLE public.subscriptions ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_subscriptions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF NEW.exp_date IS DISTINCT FROM OLD.exp_date THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.subscriptions FOR EACH ROW EXECUTE FUNCTION public.scd_ver_subscriptions();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.subscriptions FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.subscriptions ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard_intent BEFORE UPDATE OF action_type, cancelled, created_at, generation, intent_v, payload, target_urn, user_id ON public.sync_queue FOR EACH STATEMENT WHEN (public.scd_role() = 'core') EXECUTE FUNCTION public.scd_guard('public.scd_guard_intent_sync_queue', 'mirror', 'migration', 'repl', 'api');

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF dead, exec_generation, failed_at, last_error, lease_generation, lease_id, locked_at, next_run_at, remote_attempted_generation, remote_completed_generation, remote_result, result_v, retry_count ON public.sync_queue FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_sync_queue', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_sync_queue()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.intent_v <= OLD.intent_v THEN
            NEW.user_id := OLD.user_id;
            NEW.action_type := OLD.action_type;
            NEW.target_urn := OLD.target_urn;
            NEW.created_at := OLD.created_at;
            NEW.payload := OLD.payload;
            NEW.generation := OLD.generation;
            NEW.cancelled := OLD.cancelled;
            NEW.intent_v := OLD.intent_v;
        END IF;
        IF NEW.result_v <= OLD.result_v THEN
            NEW.locked_at := OLD.locked_at;
            NEW.lease_id := OLD.lease_id;
            NEW.lease_generation := OLD.lease_generation;
            NEW.retry_count := OLD.retry_count;
            NEW.last_error := OLD.last_error;
            NEW.next_run_at := OLD.next_run_at;
            NEW.dead := OLD.dead;
            NEW.failed_at := OLD.failed_at;
            NEW.remote_attempted_generation := OLD.remote_attempted_generation;
            NEW.remote_completed_generation := OLD.remote_completed_generation;
            NEW.remote_result := OLD.remote_result;
            NEW.exec_generation := OLD.exec_generation;
            NEW.result_v := OLD.result_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.intent_v, t.result_v INTO o FROM public.sync_queue t WHERE t.id = NEW.id;
    IF FOUND THEN
        IF NEW.intent_v > o.intent_v OR NEW.result_v > o.result_v THEN
            UPDATE public.sync_queue t SET user_id = NEW.user_id, action_type = NEW.action_type, target_urn = NEW.target_urn, created_at = NEW.created_at, payload = NEW.payload, generation = NEW.generation, cancelled = NEW.cancelled, intent_v = NEW.intent_v, locked_at = NEW.locked_at, lease_id = NEW.lease_id, lease_generation = NEW.lease_generation, retry_count = NEW.retry_count, last_error = NEW.last_error, next_run_at = NEW.next_run_at, dead = NEW.dead, failed_at = NEW.failed_at, remote_attempted_generation = NEW.remote_attempted_generation, remote_completed_generation = NEW.remote_completed_generation, remote_result = NEW.remote_result, exec_generation = NEW.exec_generation, result_v = NEW.result_v WHERE t.id = NEW.id;
        END IF;
        RETURN NULL;
    END IF;
    IF (((NEW.action_type)::text <> 'comment'::text)) THEN
        SELECT t.id INTO o FROM public.sync_queue t WHERE (t.user_id, t.action_type, t.target_urn) = (NEW.user_id, NEW.action_type, NEW.target_urn) AND (((t.action_type)::text <> 'comment'::text));
        IF FOUND THEN
            PERFORM nextval('public.scd_conflict_sync_queue'::regclass);
            UPDATE public.sync_queue t SET user_id = NEW.user_id, action_type = NEW.action_type, target_urn = NEW.target_urn, created_at = NEW.created_at, payload = NEW.payload, generation = NEW.generation, cancelled = NEW.cancelled, intent_v = NEW.intent_v, locked_at = NEW.locked_at, lease_id = NEW.lease_id, lease_generation = NEW.lease_generation, retry_count = NEW.retry_count, last_error = NEW.last_error, next_run_at = NEW.next_run_at, dead = NEW.dead, failed_at = NEW.failed_at, remote_attempted_generation = NEW.remote_attempted_generation, remote_completed_generation = NEW.remote_completed_generation, remote_result = NEW.remote_result, exec_generation = NEW.exec_generation, result_v = NEW.result_v WHERE t.id = o.id;
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.sync_queue FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_sync_queue();
ALTER TABLE public.sync_queue ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_sync_queue()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'sync_queue' AND x.pk = jsonb_build_object('id', OLD.id);
    IF OLD.intent_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('sync_queue', jsonb_build_object('id', OLD.id)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_sync_queue'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.sync_queue FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_sync_queue();
ALTER TABLE public.sync_queue ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_sync_queue()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF public.scd_role() = 'api' OR current_setting('scd.writer', true) = 'api' THEN
            NEW.intent_v := public.scd_hlc(NEW.intent_v);
        END IF;
        IF public.scd_role() = 'core' AND coalesce(current_setting('scd.writer', true), '') <> 'api' THEN
            NEW.result_v := public.scd_hlc(NEW.result_v);
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.user_id, NEW.action_type, NEW.target_urn, NEW.created_at, NEW.payload, NEW.generation, NEW.cancelled) IS DISTINCT FROM (OLD.user_id, OLD.action_type, OLD.target_urn, OLD.created_at, OLD.payload, OLD.generation, OLD.cancelled) THEN
        NEW.intent_v := public.scd_hlc(greatest(OLD.intent_v, NEW.intent_v));
    ELSE
        NEW.intent_v := greatest(OLD.intent_v, NEW.intent_v);
    END IF;
    IF (NEW.locked_at, NEW.lease_id, NEW.lease_generation, NEW.retry_count, NEW.last_error, NEW.next_run_at, NEW.dead, NEW.failed_at, NEW.remote_attempted_generation, NEW.remote_completed_generation, NEW.remote_result, NEW.exec_generation) IS DISTINCT FROM (OLD.locked_at, OLD.lease_id, OLD.lease_generation, OLD.retry_count, OLD.last_error, OLD.next_run_at, OLD.dead, OLD.failed_at, OLD.remote_attempted_generation, OLD.remote_completed_generation, OLD.remote_result, OLD.exec_generation) THEN
        NEW.result_v := public.scd_hlc(greatest(OLD.result_v, NEW.result_v));
    ELSE
        NEW.result_v := greatest(OLD.result_v, NEW.result_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.sync_queue FOR EACH ROW EXECUTE FUNCTION public.scd_ver_sync_queue();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sync_queue FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sync_queue ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.sync_queue_action_pauses FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_sync_queue_action_pauses', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_sync_queue_action_pauses()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'sync_queue_action_pauses' AND p.pk = jsonb_build_object('oauth_app_id', NEW.oauth_app_id, 'action_type', NEW.action_type)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.sync_queue_action_pauses t SET paused_until = NEW.paused_until, reason = NEW.reason, strikes = NEW.strikes, updated_at = NEW.updated_at WHERE (t.oauth_app_id, t.action_type) = (NEW.oauth_app_id, NEW.action_type);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.sync_queue_action_pauses FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_sync_queue_action_pauses();
ALTER TABLE public.sync_queue_action_pauses ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.sync_queue_action_pauses FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.sync_queue_action_pauses ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.taste_model_versions FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_taste_model_versions', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_taste_model_versions()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
    k1 boolean := true;
    k2 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'taste_model_versions' AND p.pk = jsonb_build_object('version', NEW.version)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := NEW.collection IS DISTINCT FROM OLD.collection;
        k1 := NEW.input_object IS DISTINCT FROM OLD.input_object;
        k2 := (((true)), (NEW.active)) IS DISTINCT FROM (((true)), (OLD.active));
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.version FROM public.taste_model_versions t WHERE t.collection = NEW.collection AND t.version IS DISTINCT FROM NEW.version LOOP
            PERFORM public.scd_purge_taste_model_versions(jsonb_build_object('version', r.version));
        END LOOP;
    END IF;
    IF k1 THEN
        FOR r IN SELECT t.version FROM public.taste_model_versions t WHERE t.input_object = NEW.input_object AND t.version IS DISTINCT FROM NEW.version LOOP
            PERFORM public.scd_purge_taste_model_versions(jsonb_build_object('version', r.version));
        END LOOP;
    END IF;
    IF k2 AND (NEW.active) THEN
        FOR r IN SELECT t.version FROM public.taste_model_versions t WHERE ((true)) = ((true)) AND (t.active) AND t.version IS DISTINCT FROM NEW.version LOOP
            PERFORM public.scd_purge_taste_model_versions(jsonb_build_object('version', r.version));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.taste_model_versions t SET active = NEW.active, applied_at = NEW.applied_at, collection = NEW.collection, created_at = NEW.created_at, dim = NEW.dim, input_object = NEW.input_object, items_count = NEW.items_count, metrics = NEW.metrics, pooling = NEW.pooling, refreshed_through = NEW.refreshed_through, trained_at = NEW.trained_at, users_count = NEW.users_count WHERE t.version = NEW.version;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.taste_model_versions FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_taste_model_versions();
ALTER TABLE public.taste_model_versions ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.taste_model_versions FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.taste_model_versions ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.taste_schedule FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_taste_schedule', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.created_at := OLD.created_at;
            NEW.note := OLD.note;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.track_artist_blocks t WHERE (t.track_id, t.artist_id) = (NEW.track_id, NEW.artist_id);
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.track_artist_blocks t SET created_at = NEW.created_at, note = NEW.note, row_v = NEW.row_v WHERE (t.track_id, t.artist_id) = (NEW.track_id, NEW.artist_id);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.track_artist_blocks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_track_artist_blocks();
ALTER TABLE public.track_artist_blocks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('track_artist_blocks', jsonb_build_object('track_id', OLD.track_id, 'artist_id', OLD.artist_id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.track_artist_blocks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_track_artist_blocks();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'track_artist_blocks' AND x.pk = jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.track_artist_blocks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_track_artist_blocks();
ALTER TABLE public.track_artist_blocks ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_track_artist_blocks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.created_at, NEW.note) IS DISTINCT FROM (OLD.created_at, OLD.note) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.track_artist_blocks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_track_artist_blocks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.track_artist_blocks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.track_artist_blocks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.track_artists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_track_artists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_track_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_artists' AND p.pk = jsonb_build_object('track_id', NEW.track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.track_artists t SET confidence = NEW.confidence, evidence = NEW.evidence, "position" = NEW."position", source = NEW.source WHERE (t.track_id, t.artist_id, t.role) = (NEW.track_id, NEW.artist_id, NEW.role);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.track_artists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_track_artists();
ALTER TABLE public.track_artists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.track_artists FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.track_artists ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.track_comments FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_track_comments', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_track_comments()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'track_comments' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := NEW.sc_comment_id IS DISTINCT FROM OLD.sc_comment_id;
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.id FROM public.track_comments t WHERE t.sc_comment_id = NEW.sc_comment_id AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_track_comments(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.track_comments t SET body = NEW.body, created_at = NEW.created_at, sc_comment_id = NEW.sc_comment_id, sc_created_at = NEW.sc_created_at, sc_track_id = NEW.sc_track_id, synced_at = NEW.synced_at, track_position_ms = NEW.track_position_ms, user_urn = NEW.user_urn WHERE t.id = NEW.id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.track_comments FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_track_comments();
ALTER TABLE public.track_comments ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.track_comments FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.track_comments ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.track_work_aliases FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_track_work_aliases', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_tracks', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
    k1 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := NEW.sc_track_id IS DISTINCT FROM OLD.sc_track_id;
        k1 := NEW.urn IS DISTINCT FROM OLD.urn;
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.id FROM public.tracks t WHERE t.sc_track_id = NEW.sc_track_id AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_tracks(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF k1 THEN
        FOR r IN SELECT t.id FROM public.tracks t WHERE t.urn = NEW.urn AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_tracks(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.tracks t SET album_id = NEW.album_id, album_position = NEW.album_position, artwork_url = NEW.artwork_url, audio_fingerprint = NEW.audio_fingerprint, canonical_track_id = NEW.canonical_track_id, comments_count_sc = NEW.comments_count_sc, cover_of_artist_id = NEW.cover_of_artist_id, created_at = NEW.created_at, deleted_at = NEW.deleted_at, description = NEW.description, duration_ms = NEW.duration_ms, duration_resolve_attempts = NEW.duration_resolve_attempts, duration_resolve_retry_at = NEW.duration_resolve_retry_at, enrich_attempts = NEW.enrich_attempts, enrich_confidence = NEW.enrich_confidence, enrich_error = NEW.enrich_error, enrich_locked_at = NEW.enrich_locked_at, enrich_next_run_at = NEW.enrich_next_run_at, enrich_source = NEW.enrich_source, enrich_state = NEW.enrich_state, enriched_at = NEW.enriched_at, genius_song_id = NEW.genius_song_id, genius_url = NEW.genius_url, genre = NEW.genre, hq_upgrade_attempts = NEW.hq_upgrade_attempts, hq_upgrade_last_at = NEW.hq_upgrade_last_at, hq_upgrade_pending = NEW.hq_upgrade_pending, index_attempts = NEW.index_attempts, index_priority = NEW.index_priority, index_state = NEW.index_state, indexed_at = NEW.indexed_at, is_cover = NEW.is_cover, isrc = NEW.isrc, language = NEW.language, language_confidence = NEW.language_confidence, likes_count_sc = NEW.likes_count_sc, metadata_artist = NEW.metadata_artist, needs_duration_resolve = NEW.needs_duration_resolve, permalink_url = NEW.permalink_url, pipeline_held = NEW.pipeline_held, play_count_sc = NEW.play_count_sc, primary_artist_id = NEW.primary_artist_id, quality_model_version = NEW.quality_model_version, quality_score = NEW.quality_score, recording_key = NEW.recording_key, release_date = NEW.release_date, release_year = NEW.release_year, reposts_count_sc = NEW.reposts_count_sc, s3_missing_at = NEW.s3_missing_at, s3_verified_at = NEW.s3_verified_at, sc_created_at = NEW.sc_created_at, sc_desired = NEW.sc_desired, sc_last_modified = NEW.sc_last_modified, sc_metadata = NEW.sc_metadata, sc_mutation_observation = NEW.sc_mutation_observation, sc_observation = NEW.sc_observation, sc_synced_at = NEW.sc_synced_at, sc_track_id = NEW.sc_track_id, sc_write_confirmed = NEW.sc_write_confirmed, sharing = NEW.sharing, storage_attempts = NEW.storage_attempts, storage_priority = NEW.storage_priority, storage_quality = NEW.storage_quality, storage_state = NEW.storage_state, superseded_by = NEW.superseded_by, tags = NEW.tags, title = NEW.title, title_normalized = NEW.title_normalized, transcribe_at = NEW.transcribe_at, transcribe_state = NEW.transcribe_state, updated_at = NEW.updated_at, upload_kind = NEW.upload_kind, uploader_avatar_url = NEW.uploader_avatar_url, uploader_sc_user_id = NEW.uploader_sc_user_id, uploader_urn = NEW.uploader_urn, uploader_username = NEW.uploader_username, urn = NEW.urn, waveform_url = NEW.waveform_url, work_key = NEW.work_key, work_normalizer_version = NEW.work_normalizer_version WHERE t.id = NEW.id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_tracks();
ALTER TABLE public.tracks ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.tracks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.tracks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.transcription_sync_versions FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_transcription_sync_versions', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.transcription_wire_state FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_transcription_wire_state', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_user_auras()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.aura_id := OLD.aura_id;
            NEW.custom_hex := OLD.custom_hex;
            NEW.updated_at := OLD.updated_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.user_auras t WHERE t.user_urn = NEW.user_urn;
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.user_auras t SET aura_id = NEW.aura_id, custom_hex = NEW.custom_hex, updated_at = NEW.updated_at, row_v = NEW.row_v WHERE t.user_urn = NEW.user_urn;
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_auras FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_auras();
ALTER TABLE public.user_auras ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_ver_user_auras()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.aura_id, NEW.custom_hex, NEW.updated_at) IS DISTINCT FROM (OLD.aura_id, OLD.custom_hex, OLD.updated_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_auras FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_auras();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_auras FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_auras ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_user_blocked_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.avatar_url := OLD.avatar_url;
            NEW.created_at := OLD.created_at;
            NEW.name := OLD.name;
            NEW.sc_user_ids := OLD.sc_user_ids;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.user_blocked_artists t WHERE (t.sc_user_id, t.kind, t.target_id) = (NEW.sc_user_id, NEW.kind, NEW.target_id);
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.user_blocked_artists t SET avatar_url = NEW.avatar_url, created_at = NEW.created_at, name = NEW.name, sc_user_ids = NEW.sc_user_ids, row_v = NEW.row_v WHERE (t.sc_user_id, t.kind, t.target_id) = (NEW.sc_user_id, NEW.kind, NEW.target_id);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_blocked_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_blocked_artists();
ALTER TABLE public.user_blocked_artists ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_user_blocked_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('user_blocked_artists', jsonb_build_object('sc_user_id', OLD.sc_user_id, 'kind', OLD.kind, 'target_id', OLD.target_id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.user_blocked_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_user_blocked_artists();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_user_blocked_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'user_blocked_artists' AND x.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id, 'kind', NEW.kind, 'target_id', NEW.target_id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.user_blocked_artists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_user_blocked_artists();
ALTER TABLE public.user_blocked_artists ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_user_blocked_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.avatar_url, NEW.created_at, NEW.name, NEW.sc_user_ids) IS DISTINCT FROM (OLD.avatar_url, OLD.created_at, OLD.name, OLD.sc_user_ids) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_blocked_artists FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_blocked_artists();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_blocked_artists FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_blocked_artists ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_collection_sync FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_collection_sync', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_skip_user_events()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    PERFORM 1 FROM public.user_events t WHERE t.id = NEW.id;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_skip BEFORE INSERT ON public.user_events FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_skip_user_events();
ALTER TABLE public.user_events ENABLE REPLICA TRIGGER zz_skip;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_events FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_events ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_followers FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_followers', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_followers()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_followers' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'target_user_urn', NEW.target_user_urn)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.user_followers t SET created_at = NEW.created_at, progress = NEW.progress, synced_at = NEW.synced_at WHERE (t.user_id, t.target_user_urn) = (NEW.user_id, NEW.target_user_urn);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.user_followers FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_followers();
ALTER TABLE public.user_followers ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_followers FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_followers ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard_intent BEFORE UPDATE OF created_at, wanted_state, wanted_v ON public.user_followings FOR EACH STATEMENT WHEN (public.scd_role() = 'core') EXECUTE FUNCTION public.scd_guard('public.scd_guard_intent_user_followings', 'mirror', 'migration', 'repl', 'api');

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF progress, result_v, synced_at, synced_for_v ON public.user_followings FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_user_followings', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_user_followings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.wanted_v <= OLD.wanted_v THEN
            NEW.wanted_state := OLD.wanted_state;
            NEW.created_at := OLD.created_at;
            NEW.wanted_v := OLD.wanted_v;
        END IF;
        IF NEW.result_v <= OLD.result_v THEN
            NEW.progress := OLD.progress;
            NEW.synced_at := OLD.synced_at;
            NEW.synced_for_v := OLD.synced_for_v;
            NEW.result_v := OLD.result_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.wanted_v, t.result_v INTO o FROM public.user_followings t WHERE (t.user_id, t.target_user_urn) = (NEW.user_id, NEW.target_user_urn);
    IF FOUND THEN
        IF NEW.wanted_v > o.wanted_v OR NEW.result_v > o.result_v THEN
            UPDATE public.user_followings t SET wanted_state = NEW.wanted_state, created_at = NEW.created_at, wanted_v = NEW.wanted_v, progress = NEW.progress, synced_at = NEW.synced_at, synced_for_v = NEW.synced_for_v, result_v = NEW.result_v WHERE (t.user_id, t.target_user_urn) = (NEW.user_id, NEW.target_user_urn);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_followings FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_followings();
ALTER TABLE public.user_followings ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_user_followings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'user_followings' AND x.pk = jsonb_build_object('user_id', OLD.user_id, 'target_user_urn', OLD.target_user_urn);
    IF OLD.wanted_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_followings', jsonb_build_object('user_id', OLD.user_id, 'target_user_urn', OLD.target_user_urn)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_followings'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.user_followings FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_user_followings();
ALTER TABLE public.user_followings ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_followings()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF public.scd_role() = 'api' OR current_setting('scd.writer', true) = 'api' THEN
            NEW.wanted_v := public.scd_hlc(NEW.wanted_v);
        END IF;
        IF public.scd_role() = 'core' AND coalesce(current_setting('scd.writer', true), '') <> 'api' THEN
            NEW.result_v := public.scd_hlc(NEW.result_v);
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.wanted_state, NEW.created_at) IS DISTINCT FROM (OLD.wanted_state, OLD.created_at) THEN
        NEW.wanted_v := public.scd_hlc(greatest(OLD.wanted_v, NEW.wanted_v));
    ELSE
        NEW.wanted_v := greatest(OLD.wanted_v, NEW.wanted_v);
    END IF;
    IF (NEW.progress, NEW.synced_at, NEW.synced_for_v) IS DISTINCT FROM (OLD.progress, OLD.synced_at, OLD.synced_for_v) THEN
        NEW.result_v := public.scd_hlc(greatest(OLD.result_v, NEW.result_v));
    ELSE
        NEW.result_v := greatest(OLD.result_v, NEW.result_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_followings FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_followings();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_followings FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_followings ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_user_like_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.like_created_at := OLD.like_created_at;
            NEW.rank := OLD.rank;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.user_like_ranks t WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.user_like_ranks t SET like_created_at = NEW.like_created_at, rank = NEW.rank, row_v = NEW.row_v WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_like_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_like_ranks();
ALTER TABLE public.user_like_ranks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_user_like_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('user_like_ranks', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.user_like_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_user_like_ranks();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_user_like_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'user_like_ranks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.user_like_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_user_like_ranks();
ALTER TABLE public.user_like_ranks ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_user_like_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.like_created_at, NEW.rank) IS DISTINCT FROM (OLD.like_created_at, OLD.rank) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_like_ranks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_like_ranks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_like_ranks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_like_ranks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard_intent BEFORE UPDATE OF created_at, wanted_state, wanted_v ON public.user_likes_playlists FOR EACH STATEMENT WHEN (public.scd_role() = 'core') EXECUTE FUNCTION public.scd_guard('public.scd_guard_intent_user_likes_playlists', 'mirror', 'migration', 'repl', 'api');

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF progress, result_v, synced_at, synced_for_v ON public.user_likes_playlists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_user_likes_playlists', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_user_likes_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.wanted_v <= OLD.wanted_v THEN
            NEW.wanted_state := OLD.wanted_state;
            NEW.created_at := OLD.created_at;
            NEW.wanted_v := OLD.wanted_v;
        END IF;
        IF NEW.result_v <= OLD.result_v THEN
            NEW.progress := OLD.progress;
            NEW.synced_at := OLD.synced_at;
            NEW.synced_for_v := OLD.synced_for_v;
            NEW.result_v := OLD.result_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.wanted_v, t.result_v INTO o FROM public.user_likes_playlists t WHERE (t.user_id, t.playlist_urn) = (NEW.user_id, NEW.playlist_urn);
    IF FOUND THEN
        IF NEW.wanted_v > o.wanted_v OR NEW.result_v > o.result_v THEN
            UPDATE public.user_likes_playlists t SET wanted_state = NEW.wanted_state, created_at = NEW.created_at, wanted_v = NEW.wanted_v, progress = NEW.progress, synced_at = NEW.synced_at, synced_for_v = NEW.synced_for_v, result_v = NEW.result_v WHERE (t.user_id, t.playlist_urn) = (NEW.user_id, NEW.playlist_urn);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_likes_playlists FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_likes_playlists();
ALTER TABLE public.user_likes_playlists ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_user_likes_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'user_likes_playlists' AND x.pk = jsonb_build_object('user_id', OLD.user_id, 'playlist_urn', OLD.playlist_urn);
    IF OLD.wanted_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_likes_playlists', jsonb_build_object('user_id', OLD.user_id, 'playlist_urn', OLD.playlist_urn)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_likes_playlists'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.user_likes_playlists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_user_likes_playlists();
ALTER TABLE public.user_likes_playlists ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_likes_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF public.scd_role() = 'api' OR current_setting('scd.writer', true) = 'api' THEN
            NEW.wanted_v := public.scd_hlc(NEW.wanted_v);
        END IF;
        IF public.scd_role() = 'core' AND coalesce(current_setting('scd.writer', true), '') <> 'api' THEN
            NEW.result_v := public.scd_hlc(NEW.result_v);
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.wanted_state, NEW.created_at) IS DISTINCT FROM (OLD.wanted_state, OLD.created_at) THEN
        NEW.wanted_v := public.scd_hlc(greatest(OLD.wanted_v, NEW.wanted_v));
    ELSE
        NEW.wanted_v := greatest(OLD.wanted_v, NEW.wanted_v);
    END IF;
    IF (NEW.progress, NEW.synced_at, NEW.synced_for_v) IS DISTINCT FROM (OLD.progress, OLD.synced_at, OLD.synced_for_v) THEN
        NEW.result_v := public.scd_hlc(greatest(OLD.result_v, NEW.result_v));
    ELSE
        NEW.result_v := greatest(OLD.result_v, NEW.result_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_likes_playlists FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_likes_playlists();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_likes_playlists FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_likes_playlists ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard_intent BEFORE UPDATE OF created_at, wanted_state, wanted_v ON public.user_likes_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'core') EXECUTE FUNCTION public.scd_guard('public.scd_guard_intent_user_likes_tracks', 'mirror', 'migration', 'repl', 'api');

CREATE OR REPLACE TRIGGER zz_guard_result BEFORE UPDATE OF liked_at, progress, result_v, synced_at, synced_for_v ON public.user_likes_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_result_user_likes_tracks', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_merge_user_likes_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.wanted_v <= OLD.wanted_v THEN
            NEW.wanted_state := OLD.wanted_state;
            NEW.created_at := OLD.created_at;
            NEW.wanted_v := OLD.wanted_v;
        END IF;
        IF NEW.result_v <= OLD.result_v THEN
            NEW.progress := OLD.progress;
            NEW.synced_at := OLD.synced_at;
            NEW.liked_at := OLD.liked_at;
            NEW.synced_for_v := OLD.synced_for_v;
            NEW.result_v := OLD.result_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.wanted_v, t.result_v INTO o FROM public.user_likes_tracks t WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
    IF FOUND THEN
        IF NEW.wanted_v > o.wanted_v OR NEW.result_v > o.result_v THEN
            UPDATE public.user_likes_tracks t SET wanted_state = NEW.wanted_state, created_at = NEW.created_at, wanted_v = NEW.wanted_v, progress = NEW.progress, synced_at = NEW.synced_at, liked_at = NEW.liked_at, synced_for_v = NEW.synced_for_v, result_v = NEW.result_v WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_likes_tracks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_likes_tracks();
ALTER TABLE public.user_likes_tracks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_purge_guard_user_likes_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    pv bigint;
BEGIN
    SELECT max(x.purged_v) INTO pv FROM public.repl_purges x WHERE x.tbl = 'user_likes_tracks' AND x.pk = jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id);
    IF OLD.wanted_v > coalesce(pv, 0) THEN
        INSERT INTO public.repl_resend (tbl, pk) VALUES ('user_likes_tracks', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id)) ON CONFLICT DO NOTHING;
        PERFORM nextval('public.scd_resend_user_likes_tracks'::regclass);
        RETURN NULL;
    END IF;
    RETURN OLD;
END
$f$;
CREATE OR REPLACE TRIGGER zz_purge_guard BEFORE DELETE ON public.user_likes_tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_purge_guard_user_likes_tracks();
ALTER TABLE public.user_likes_tracks ENABLE REPLICA TRIGGER zz_purge_guard;

CREATE OR REPLACE FUNCTION public.scd_ver_user_likes_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF public.scd_role() = 'api' OR current_setting('scd.writer', true) = 'api' THEN
            NEW.wanted_v := public.scd_hlc(NEW.wanted_v);
        END IF;
        IF public.scd_role() = 'core' AND coalesce(current_setting('scd.writer', true), '') <> 'api' THEN
            NEW.result_v := public.scd_hlc(NEW.result_v);
        END IF;
        RETURN NEW;
    END IF;
    IF (NEW.wanted_state, NEW.created_at) IS DISTINCT FROM (OLD.wanted_state, OLD.created_at) THEN
        NEW.wanted_v := public.scd_hlc(greatest(OLD.wanted_v, NEW.wanted_v));
    ELSE
        NEW.wanted_v := greatest(OLD.wanted_v, NEW.wanted_v);
    END IF;
    IF (NEW.progress, NEW.synced_at, NEW.liked_at, NEW.synced_for_v) IS DISTINCT FROM (OLD.progress, OLD.synced_at, OLD.liked_at, OLD.synced_for_v) THEN
        NEW.result_v := public.scd_hlc(greatest(OLD.result_v, NEW.result_v));
    ELSE
        NEW.result_v := greatest(OLD.result_v, NEW.result_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_likes_tracks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_likes_tracks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_likes_tracks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_likes_tracks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_owned_playlists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_owned_playlists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_owned_playlists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_playlists' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'playlist_urn', NEW.playlist_urn)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.user_owned_playlists t SET created_at = NEW.created_at, progress = NEW.progress, synced_at = NEW.synced_at WHERE (t.user_id, t.playlist_urn) = (NEW.user_id, NEW.playlist_urn);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.user_owned_playlists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_owned_playlists();
ALTER TABLE public.user_owned_playlists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_owned_playlists FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_owned_playlists ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_owned_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_owned_tracks', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_owned_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_owned_tracks' AND p.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.user_owned_tracks t SET created_at = NEW.created_at, progress = NEW.progress, synced_at = NEW.synced_at WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.user_owned_tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_owned_tracks();
ALTER TABLE public.user_owned_tracks ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_owned_tracks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_owned_tracks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_profiles FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_profiles', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_profiles()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_profiles' AND p.pk = jsonb_build_object('soundcloud_user_id', NEW.soundcloud_user_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.user_profiles t SET profile_json = NEW.profile_json, sc_observation = NEW.sc_observation, synced_at = NEW.synced_at WHERE t.soundcloud_user_id = NEW.soundcloud_user_id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.user_profiles FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_profiles();
ALTER TABLE public.user_profiles ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_profiles FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_profiles ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_user_superlike_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.rank := OLD.rank;
            NEW.superlike_created_at := OLD.superlike_created_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.user_superlike_ranks t WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.user_superlike_ranks t SET rank = NEW.rank, superlike_created_at = NEW.superlike_created_at, row_v = NEW.row_v WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_superlike_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_superlike_ranks();
ALTER TABLE public.user_superlike_ranks ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_user_superlike_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('user_superlike_ranks', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.user_superlike_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_user_superlike_ranks();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_user_superlike_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'user_superlike_ranks' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.user_superlike_ranks FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_user_superlike_ranks();
ALTER TABLE public.user_superlike_ranks ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_user_superlike_ranks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.rank, NEW.superlike_created_at) IS DISTINCT FROM (OLD.rank, OLD.superlike_created_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_superlike_ranks FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_superlike_ranks();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_superlike_ranks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_superlike_ranks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE FUNCTION public.scd_merge_user_superlikes()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    o record;
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.row_v <= OLD.row_v THEN
            NEW.created_at := OLD.created_at;
            NEW.like_created_at := OLD.like_created_at;
            NEW.row_v := OLD.row_v;
        END IF;
        RETURN NEW;
    END IF;
    SELECT t.row_v INTO o FROM public.user_superlikes t WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
    IF FOUND THEN
        IF NEW.row_v > o.row_v THEN
            UPDATE public.user_superlikes t SET created_at = NEW.created_at, like_created_at = NEW.like_created_at, row_v = NEW.row_v WHERE (t.user_id, t.sc_track_id) = (NEW.user_id, NEW.sc_track_id);
        END IF;
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_merge BEFORE INSERT OR UPDATE ON public.user_superlikes FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_merge_user_superlikes();
ALTER TABLE public.user_superlikes ENABLE REPLICA TRIGGER zz_merge;

CREATE OR REPLACE FUNCTION public.scd_tomb_user_superlikes()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    INSERT INTO public.repl_tombstones (tbl, pk, v, at)
    VALUES ('user_superlikes', jsonb_build_object('user_id', OLD.user_id, 'sc_track_id', OLD.sc_track_id), OLD.row_v, now())
    ON CONFLICT (tbl, pk) DO UPDATE SET v = EXCLUDED.v, at = EXCLUDED.at;
    RETURN NULL;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb AFTER DELETE ON public.user_superlikes FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_user_superlikes();

CREATE OR REPLACE FUNCTION public.scd_tomb_check_user_superlikes()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    tv bigint;
BEGIN
    SELECT x.v INTO tv FROM public.repl_tombstones x WHERE x.tbl = 'user_superlikes' AND x.pk = jsonb_build_object('user_id', NEW.user_id, 'sc_track_id', NEW.sc_track_id);
    IF FOUND AND NEW.row_v <= tv THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_tomb_check BEFORE INSERT ON public.user_superlikes FOR EACH ROW WHEN (public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_tomb_check_user_superlikes();
ALTER TABLE public.user_superlikes ENABLE REPLICA TRIGGER zz_tomb_check;

CREATE OR REPLACE FUNCTION public.scd_ver_user_superlikes()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.row_v := public.scd_hlc(NEW.row_v);
        RETURN NEW;
    END IF;
    IF (NEW.created_at, NEW.like_created_at) IS DISTINCT FROM (OLD.created_at, OLD.like_created_at) THEN
        NEW.row_v := public.scd_hlc(greatest(OLD.row_v, NEW.row_v));
    ELSE
        NEW.row_v := greatest(OLD.row_v, NEW.row_v);
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_ver BEFORE INSERT OR UPDATE ON public.user_superlikes FOR EACH ROW EXECUTE FUNCTION public.scd_ver_user_superlikes();

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_superlikes FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_superlikes ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_taste_vectors FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_taste_vectors', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_taste_vectors()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_taste_vectors' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id, 'version', NEW.version)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.user_taste_vectors t SET updated_at = NEW.updated_at, vec = NEW.vec WHERE (t.sc_user_id, t.version) = (NEW.sc_user_id, NEW.version);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.user_taste_vectors FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_taste_vectors();
ALTER TABLE public.user_taste_vectors ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_taste_vectors FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_taste_vectors ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.user_web_profiles FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_user_web_profiles', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_user_web_profiles()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'user_web_profiles' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.user_web_profiles t SET profiles = NEW.profiles, sc_observation = NEW.sc_observation, synced_at = NEW.synced_at WHERE t.sc_user_id = NEW.sc_user_id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.user_web_profiles FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_user_web_profiles();
ALTER TABLE public.user_web_profiles ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.user_web_profiles FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.user_web_profiles ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.users FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_users', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_users()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'users' AND p.pk = jsonb_build_object('sc_user_id', NEW.sc_user_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := NEW.urn IS DISTINCT FROM OLD.urn;
    END IF;
    IF k0 THEN
        FOR r IN SELECT t.sc_user_id FROM public.users t WHERE t.urn = NEW.urn AND t.sc_user_id IS DISTINCT FROM NEW.sc_user_id LOOP
            PERFORM public.scd_purge_users(jsonb_build_object('sc_user_id', r.sc_user_id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.users t SET avatar_url = NEW.avatar_url, city = NEW.city, comments_count = NEW.comments_count, country = NEW.country, created_at = NEW.created_at, description = NEW.description, first_name = NEW.first_name, followers_count = NEW.followers_count, followings_count = NEW.followings_count, full_name = NEW.full_name, kind = NEW.kind, last_name = NEW.last_name, permalink = NEW.permalink, permalink_url = NEW.permalink_url, playlists_count = NEW.playlists_count, reposts_count = NEW.reposts_count, sc_created_at = NEW.sc_created_at, sc_last_modified = NEW.sc_last_modified, sc_observation = NEW.sc_observation, sc_synced_at = NEW.sc_synced_at, tracks_count = NEW.tracks_count, updated_at = NEW.updated_at, urn = NEW.urn, username = NEW.username, username_normalized = NEW.username_normalized, verified = NEW.verified WHERE t.sc_user_id = NEW.sc_user_id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.users FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_users();
ALTER TABLE public.users ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.users FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.users ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.wanted_track_albums FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_wanted_track_albums', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_wanted_track_albums()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_albums' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'album_id', NEW.album_id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.wanted_track_albums t SET "position" = NEW."position" WHERE (t.wanted_track_id, t.album_id) = (NEW.wanted_track_id, NEW.album_id);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.wanted_track_albums FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_wanted_track_albums();
ALTER TABLE public.wanted_track_albums ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.wanted_track_albums FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.wanted_track_albums ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.wanted_track_artists FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_wanted_track_artists', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_wanted_track_artists()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_track_artists' AND p.pk = jsonb_build_object('wanted_track_id', NEW.wanted_track_id, 'artist_id', NEW.artist_id, 'role', NEW.role)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.wanted_track_artists t SET "position" = NEW."position" WHERE (t.wanted_track_id, t.artist_id, t.role) = (NEW.wanted_track_id, NEW.artist_id, NEW.role);
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.wanted_track_artists FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_wanted_track_artists();
ALTER TABLE public.wanted_track_artists ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.wanted_track_artists FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.wanted_track_artists ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.wanted_track_work_aliases FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_wanted_track_work_aliases', 'cmd', 'migration', 'repl');

CREATE OR REPLACE TRIGGER zz_guard BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON public.wanted_tracks FOR EACH STATEMENT WHEN (public.scd_role() = 'api') EXECUTE FUNCTION public.scd_guard('public.scd_guard_wanted_tracks', 'cmd', 'migration', 'repl');

CREATE OR REPLACE FUNCTION public.scd_owner_wanted_tracks()
RETURNS trigger
LANGUAGE plpgsql
AS $f$
DECLARE
    r record;
    k0 boolean := true;
    k1 boolean := true;
BEGIN
    IF EXISTS (SELECT 1 FROM public.cmd_pending p WHERE p.tbl = 'wanted_tracks' AND p.pk = jsonb_build_object('id', NEW.id)) THEN
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        k0 := (NEW.source, NEW.external_id, ((NEW.external_id IS NOT NULL))) IS DISTINCT FROM (OLD.source, OLD.external_id, ((OLD.external_id IS NOT NULL)));
        k1 := (NEW.isrc, ((NEW.isrc IS NOT NULL))) IS DISTINCT FROM (OLD.isrc, ((OLD.isrc IS NOT NULL)));
    END IF;
    IF k0 AND ((NEW.external_id IS NOT NULL)) THEN
        FOR r IN SELECT t.id FROM public.wanted_tracks t WHERE (t.source, t.external_id) = (NEW.source, NEW.external_id) AND ((t.external_id IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_wanted_tracks(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF k1 AND ((NEW.isrc IS NOT NULL)) THEN
        FOR r IN SELECT t.id FROM public.wanted_tracks t WHERE t.isrc = NEW.isrc AND ((t.isrc IS NOT NULL)) AND t.id IS DISTINCT FROM NEW.id LOOP
            PERFORM public.scd_purge_wanted_tracks(jsonb_build_object('id', r.id));
        END LOOP;
    END IF;
    IF TG_OP = 'INSERT' THEN
        UPDATE public.wanted_tracks t SET discovered_at = NEW.discovered_at, duration_ms = NEW.duration_ms, external_id = NEW.external_id, isrc = NEW.isrc, normalized_title = NEW.normalized_title, primary_artist_id = NEW.primary_artist_id, recording_key = NEW.recording_key, release_year = NEW.release_year, resolve_attempts = NEW.resolve_attempts, resolve_error = NEW.resolve_error, resolve_locked_at = NEW.resolve_locked_at, resolve_next_run_at = NEW.resolve_next_run_at, source = NEW.source, status = NEW.status, title = NEW.title, track_id = NEW.track_id, updated_at = NEW.updated_at, work_key = NEW.work_key, work_normalizer_version = NEW.work_normalizer_version, work_reconciled_at = NEW.work_reconciled_at WHERE t.id = NEW.id;
        IF FOUND THEN
            RETURN NULL;
        END IF;
    END IF;
    RETURN NEW;
END
$f$;
CREATE OR REPLACE TRIGGER zz_owner BEFORE INSERT OR UPDATE ON public.wanted_tracks FOR EACH ROW WHEN (public.scd_role() = 'api' AND public.scd_mode() = 'split') EXECUTE FUNCTION public.scd_owner_wanted_tracks();
ALTER TABLE public.wanted_tracks ENABLE REPLICA TRIGGER zz_owner;

CREATE OR REPLACE TRIGGER zzz_suppress BEFORE UPDATE ON public.wanted_tracks FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger();
ALTER TABLE public.wanted_tracks ENABLE REPLICA TRIGGER zzz_suppress;

CREATE OR REPLACE VIEW public.scd_counters AS
SELECT c.counter, c.tbl, coalesce(s.last_value, 0) AS n
FROM (VALUES
    ('conflict', 'cdn_tracks', 'scd_conflict_cdn_tracks'),
    ('conflict', 'discover_promoted', 'scd_conflict_discover_promoted'),
    ('conflict', 'disliked_tracks', 'scd_conflict_disliked_tracks'),
    ('conflict', 'oauth_apps', 'scd_conflict_oauth_apps'),
    ('conflict', 'sync_queue', 'scd_conflict_sync_queue'),
    ('guard', 'admin_maintenance_runs', 'scd_guard_admin_maintenance_runs'),
    ('guard', 'ai_resolver_budget', 'scd_guard_ai_resolver_budget'),
    ('guard', 'ai_resolver_cache', 'scd_guard_ai_resolver_cache'),
    ('guard', 'album_artists', 'scd_guard_album_artists'),
    ('guard', 'album_tracks', 'scd_guard_album_tracks'),
    ('guard', 'albums', 'scd_guard_albums'),
    ('guard', 'artist_attribution_revalidation', 'scd_guard_artist_attribution_revalidation'),
    ('guard', 'artist_colike', 'scd_guard_artist_colike'),
    ('guard', 'artist_coplay', 'scd_guard_artist_coplay'),
    ('guard', 'artist_sc_accounts', 'scd_guard_artist_sc_accounts'),
    ('guard', 'artist_socials', 'scd_guard_artist_socials'),
    ('guard', 'artists', 'scd_guard_artists'),
    ('guard', 'audio_index_wire_state', 'scd_guard_audio_index_wire_state'),
    ('guard', 'background_job_enqueues', 'scd_guard_background_job_enqueues'),
    ('guard', 'background_job_failures', 'scd_guard_background_job_failures'),
    ('guard', 'background_jobs', 'scd_guard_background_jobs'),
    ('guard', 'background_schedules', 'scd_guard_background_schedules'),
    ('guard', 'catalog_audience', 'scd_guard_catalog_audience'),
    ('guard', 'catalog_collection_cursors', 'scd_guard_catalog_collection_cursors'),
    ('guard', 'catalog_collection_seen', 'scd_guard_catalog_collection_seen'),
    ('guard', 'catalog_collection_sync', 'scd_guard_catalog_collection_sync'),
    ('guard', 'catalog_merge_state', 'scd_guard_catalog_merge_state'),
    ('guard', 'catalog_work_links', 'scd_guard_catalog_work_links'),
    ('guard', 'cmd_dead', 'scd_guard_cmd_dead'),
    ('guard', 'discover_tag_counts', 'scd_guard_discover_tag_counts'),
    ('guard', 'enrich_calibration', 'scd_guard_enrich_calibration'),
    ('guard', 'job_status', 'scd_guard_job_status'),
    ('guard', 'lyrics_cache', 'scd_guard_lyrics_cache'),
    ('guard', 'lyrics_embedding_wire_state', 'scd_guard_lyrics_embedding_wire_state'),
    ('guard', 'lyrics_lookup_backfill_state', 'scd_guard_lyrics_lookup_backfill_state'),
    ('guard', 'lyrics_lookup_state', 'scd_guard_lyrics_lookup_state'),
    ('guard', 'oauth_app_token_issuance_reservations', 'scd_guard_oauth_app_token_issuance_reservations'),
    ('guard', 'oauth_app_token_refresh_state', 'scd_guard_oauth_app_token_refresh_state'),
    ('guard', 'oauth_app_tokens', 'scd_guard_oauth_app_tokens'),
    ('guard', 'pipeline_event_receipts', 'scd_guard_pipeline_event_receipts'),
    ('guard', 'playlist_legacy_membership_intents', 'scd_guard_playlist_legacy_membership_intents'),
    ('guard', 'playlist_membership_operations', 'scd_guard_playlist_membership_operations'),
    ('guard', 'playlist_membership_state', 'scd_guard_playlist_membership_state'),
    ('guard', 'playlist_reconcile_runs', 'scd_guard_playlist_reconcile_runs'),
    ('guard', 'playlist_remote_observations', 'scd_guard_playlist_remote_observations'),
    ('guard', 'playlist_remote_snapshot_tracks', 'scd_guard_playlist_remote_snapshot_tracks'),
    ('guard', 'playlist_remote_snapshots', 'scd_guard_playlist_remote_snapshots'),
    ('guard', 'playlist_sweep_cursor', 'scd_guard_playlist_sweep_cursor'),
    ('guard', 'playlist_track_projection', 'scd_guard_playlist_track_projection'),
    ('guard', 'playlists', 'scd_guard_playlists'),
    ('guard', 'recommendation_quality_models', 'scd_guard_recommendation_quality_models'),
    ('guard', 'search_terms', 'scd_guard_search_terms'),
    ('guard', 'star_track_tiers', 'scd_guard_star_track_tiers'),
    ('guard', 'storage_event_state', 'scd_guard_storage_event_state'),
    ('guard', 'subscription_snapshot_state', 'scd_guard_subscription_snapshot_state'),
    ('guard', 'sync_queue_action_pauses', 'scd_guard_sync_queue_action_pauses'),
    ('guard', 'taste_model_versions', 'scd_guard_taste_model_versions'),
    ('guard', 'taste_schedule', 'scd_guard_taste_schedule'),
    ('guard', 'track_artists', 'scd_guard_track_artists'),
    ('guard', 'track_comments', 'scd_guard_track_comments'),
    ('guard', 'track_work_aliases', 'scd_guard_track_work_aliases'),
    ('guard', 'tracks', 'scd_guard_tracks'),
    ('guard', 'transcription_sync_versions', 'scd_guard_transcription_sync_versions'),
    ('guard', 'transcription_wire_state', 'scd_guard_transcription_wire_state'),
    ('guard', 'user_collection_sync', 'scd_guard_user_collection_sync'),
    ('guard', 'user_followers', 'scd_guard_user_followers'),
    ('guard', 'user_owned_playlists', 'scd_guard_user_owned_playlists'),
    ('guard', 'user_owned_tracks', 'scd_guard_user_owned_tracks'),
    ('guard', 'user_profiles', 'scd_guard_user_profiles'),
    ('guard', 'user_taste_vectors', 'scd_guard_user_taste_vectors'),
    ('guard', 'user_web_profiles', 'scd_guard_user_web_profiles'),
    ('guard', 'users', 'scd_guard_users'),
    ('guard', 'wanted_track_albums', 'scd_guard_wanted_track_albums'),
    ('guard', 'wanted_track_artists', 'scd_guard_wanted_track_artists'),
    ('guard', 'wanted_track_work_aliases', 'scd_guard_wanted_track_work_aliases'),
    ('guard', 'wanted_tracks', 'scd_guard_wanted_tracks'),
    ('guard_intent', 'sync_queue', 'scd_guard_intent_sync_queue'),
    ('guard_intent', 'user_followings', 'scd_guard_intent_user_followings'),
    ('guard_intent', 'user_likes_playlists', 'scd_guard_intent_user_likes_playlists'),
    ('guard_intent', 'user_likes_tracks', 'scd_guard_intent_user_likes_tracks'),
    ('guard_result', 'soundcloud_connections', 'scd_guard_result_soundcloud_connections'),
    ('guard_result', 'sync_queue', 'scd_guard_result_sync_queue'),
    ('guard_result', 'user_followings', 'scd_guard_result_user_followings'),
    ('guard_result', 'user_likes_playlists', 'scd_guard_result_user_likes_playlists'),
    ('guard_result', 'user_likes_tracks', 'scd_guard_result_user_likes_tracks'),
    ('resend', 'soundcloud_connections', 'scd_resend_soundcloud_connections'),
    ('resend', 'sync_queue', 'scd_resend_sync_queue'),
    ('resend', 'user_followings', 'scd_resend_user_followings'),
    ('resend', 'user_likes_playlists', 'scd_resend_user_likes_playlists'),
    ('resend', 'user_likes_tracks', 'scd_resend_user_likes_tracks')
) AS c (counter, tbl, seq)
LEFT JOIN pg_catalog.pg_sequences s ON s.schemaname = 'public' AND s.sequencename = c.seq;

DO $d$
DECLARE
    r record;
BEGIN
    FOR r IN
        SELECT c.relname::text AS tbl, t.tgname::text AS name
        FROM pg_catalog.pg_trigger t
        JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public' AND NOT t.tgisinternal AND t.tgname::text IN ('zz_guard', 'zz_guard_intent', 'zz_guard_result', 'zz_max', 'zz_merge', 'zz_owner', 'zz_purge_guard', 'zz_skip', 'zz_tomb', 'zz_tomb_check', 'zz_ver', 'zzz_suppress')
        EXCEPT VALUES
            ('admin_maintenance_runs', 'zz_guard'),
            ('admin_maintenance_runs', 'zz_owner'),
            ('admin_maintenance_runs', 'zzz_suppress'),
            ('ai_resolver_budget', 'zz_guard'),
            ('ai_resolver_cache', 'zz_guard'),
            ('album_artists', 'zz_guard'),
            ('album_artists', 'zz_owner'),
            ('album_artists', 'zzz_suppress'),
            ('album_tracks', 'zz_guard'),
            ('album_tracks', 'zz_owner'),
            ('album_tracks', 'zzz_suppress'),
            ('albums', 'zz_guard'),
            ('albums', 'zz_owner'),
            ('albums', 'zzz_suppress'),
            ('artist_attribution_revalidation', 'zz_guard'),
            ('artist_attribution_revalidation', 'zz_owner'),
            ('artist_attribution_revalidation', 'zzz_suppress'),
            ('artist_colike', 'zz_guard'),
            ('artist_colike', 'zz_owner'),
            ('artist_colike', 'zzz_suppress'),
            ('artist_coplay', 'zz_guard'),
            ('artist_coplay', 'zz_owner'),
            ('artist_coplay', 'zzz_suppress'),
            ('artist_sc_accounts', 'zz_guard'),
            ('artist_sc_accounts', 'zz_owner'),
            ('artist_sc_accounts', 'zzz_suppress'),
            ('artist_socials', 'zz_guard'),
            ('artist_socials', 'zz_owner'),
            ('artist_socials', 'zzz_suppress'),
            ('artists', 'zz_guard'),
            ('artists', 'zz_owner'),
            ('artists', 'zzz_suppress'),
            ('audio_index_wire_state', 'zz_guard'),
            ('background_job_enqueues', 'zz_guard'),
            ('background_job_failures', 'zz_guard'),
            ('background_job_failures', 'zz_owner'),
            ('background_job_failures', 'zzz_suppress'),
            ('background_jobs', 'zz_guard'),
            ('background_schedules', 'zz_guard'),
            ('catalog_audience', 'zz_guard'),
            ('catalog_audience', 'zz_owner'),
            ('catalog_audience', 'zzz_suppress'),
            ('catalog_collection_cursors', 'zz_guard'),
            ('catalog_collection_seen', 'zz_guard'),
            ('catalog_collection_sync', 'zz_guard'),
            ('catalog_collection_sync', 'zz_owner'),
            ('catalog_collection_sync', 'zzz_suppress'),
            ('catalog_merge_state', 'zz_guard'),
            ('catalog_work_links', 'zz_guard'),
            ('cdn_tracks', 'zz_merge'),
            ('cdn_tracks', 'zz_ver'),
            ('cdn_tracks', 'zzz_suppress'),
            ('cmd_dead', 'zz_guard'),
            ('diag_bundles', 'zz_skip'),
            ('diag_bundles', 'zzz_suppress'),
            ('diag_events', 'zz_skip'),
            ('diag_events', 'zzz_suppress'),
            ('diag_issue_days', 'zz_merge'),
            ('diag_issue_days', 'zz_ver'),
            ('diag_issue_days', 'zzz_suppress'),
            ('diag_issue_origins', 'zz_skip'),
            ('diag_issue_origins', 'zzz_suppress'),
            ('discover_promoted', 'zz_merge'),
            ('discover_promoted', 'zz_tomb'),
            ('discover_promoted', 'zz_tomb_check'),
            ('discover_promoted', 'zz_ver'),
            ('discover_promoted', 'zzz_suppress'),
            ('discover_settings', 'zz_merge'),
            ('discover_settings', 'zz_ver'),
            ('discover_settings', 'zzz_suppress'),
            ('discover_tag_counts', 'zz_guard'),
            ('discover_tag_counts', 'zz_owner'),
            ('discover_tag_counts', 'zzz_suppress'),
            ('disliked_tracks', 'zz_merge'),
            ('disliked_tracks', 'zz_tomb'),
            ('disliked_tracks', 'zz_tomb_check'),
            ('disliked_tracks', 'zz_ver'),
            ('disliked_tracks', 'zzz_suppress'),
            ('enrich_calibration', 'zz_guard'),
            ('entity_reads', 'zz_max'),
            ('entity_reads', 'zzz_suppress'),
            ('featured_items', 'zz_merge'),
            ('featured_items', 'zz_tomb'),
            ('featured_items', 'zz_tomb_check'),
            ('featured_items', 'zz_ver'),
            ('featured_items', 'zzz_suppress'),
            ('job_status', 'zz_guard'),
            ('job_status', 'zz_owner'),
            ('job_status', 'zzz_suppress'),
            ('link_requests', 'zz_merge'),
            ('link_requests', 'zz_tomb'),
            ('link_requests', 'zz_tomb_check'),
            ('link_requests', 'zz_ver'),
            ('link_requests', 'zzz_suppress'),
            ('listening_history', 'zz_skip'),
            ('listening_history', 'zz_tomb'),
            ('listening_history', 'zz_tomb_check'),
            ('listening_history', 'zzz_suppress'),
            ('login_requests', 'zz_merge'),
            ('login_requests', 'zz_tomb'),
            ('login_requests', 'zz_tomb_check'),
            ('login_requests', 'zz_ver'),
            ('login_requests', 'zzz_suppress'),
            ('lyrics_cache', 'zz_guard'),
            ('lyrics_cache', 'zz_owner'),
            ('lyrics_cache', 'zzz_suppress'),
            ('lyrics_embedding_wire_state', 'zz_guard'),
            ('lyrics_lookup_backfill_state', 'zz_guard'),
            ('lyrics_lookup_state', 'zz_guard'),
            ('lyrics_lookup_state', 'zz_owner'),
            ('lyrics_lookup_state', 'zzz_suppress'),
            ('oauth_app_request_cooldowns', 'zz_merge'),
            ('oauth_app_request_cooldowns', 'zz_ver'),
            ('oauth_app_request_cooldowns', 'zzz_suppress'),
            ('oauth_app_token_issuance_reservations', 'zz_guard'),
            ('oauth_app_token_refresh_state', 'zz_guard'),
            ('oauth_app_tokens', 'zz_guard'),
            ('oauth_app_tokens', 'zz_owner'),
            ('oauth_app_tokens', 'zzz_suppress'),
            ('oauth_apps', 'zz_merge'),
            ('oauth_apps', 'zz_tomb'),
            ('oauth_apps', 'zz_tomb_check'),
            ('oauth_apps', 'zz_ver'),
            ('oauth_apps', 'zzz_suppress'),
            ('pipeline_event_receipts', 'zz_guard'),
            ('playlist_legacy_membership_intents', 'zz_guard'),
            ('playlist_legacy_membership_intents', 'zz_owner'),
            ('playlist_legacy_membership_intents', 'zzz_suppress'),
            ('playlist_membership_operations', 'zz_guard'),
            ('playlist_membership_operations', 'zz_owner'),
            ('playlist_membership_operations', 'zzz_suppress'),
            ('playlist_membership_state', 'zz_guard'),
            ('playlist_membership_state', 'zz_owner'),
            ('playlist_membership_state', 'zzz_suppress'),
            ('playlist_reconcile_runs', 'zz_guard'),
            ('playlist_remote_observations', 'zz_guard'),
            ('playlist_remote_observations', 'zz_owner'),
            ('playlist_remote_observations', 'zzz_suppress'),
            ('playlist_remote_snapshot_tracks', 'zz_guard'),
            ('playlist_remote_snapshots', 'zz_guard'),
            ('playlist_sweep_cursor', 'zz_guard'),
            ('playlist_track_projection', 'zz_guard'),
            ('playlist_track_projection', 'zz_owner'),
            ('playlist_track_projection', 'zzz_suppress'),
            ('playlists', 'zz_guard'),
            ('playlists', 'zz_owner'),
            ('playlists', 'zzz_suppress'),
            ('recommendation_quality_models', 'zz_guard'),
            ('repl_checksums', 'zz_max'),
            ('repl_checksums', 'zzz_suppress'),
            ('repl_tombstones', 'zz_merge'),
            ('repl_tombstones', 'zz_ver'),
            ('repl_tombstones', 'zzz_suppress'),
            ('sc_egress_health', 'zz_merge'),
            ('sc_egress_health', 'zz_tomb'),
            ('sc_egress_health', 'zz_tomb_check'),
            ('sc_egress_health', 'zz_ver'),
            ('sc_egress_health', 'zzz_suppress'),
            ('sc_track_counters', 'zz_max'),
            ('sc_track_counters', 'zzz_suppress'),
            ('search_terms', 'zz_guard'),
            ('search_terms', 'zz_owner'),
            ('search_terms', 'zzz_suppress'),
            ('sessions', 'zz_merge'),
            ('sessions', 'zz_tomb'),
            ('sessions', 'zz_tomb_check'),
            ('sessions', 'zz_ver'),
            ('sessions', 'zzz_suppress'),
            ('soundcloud_connections', 'zz_guard_result'),
            ('soundcloud_connections', 'zz_merge'),
            ('soundcloud_connections', 'zz_purge_guard'),
            ('soundcloud_connections', 'zz_ver'),
            ('soundcloud_connections', 'zzz_suppress'),
            ('star_track_tiers', 'zz_guard'),
            ('storage_event_state', 'zz_guard'),
            ('subscription_snapshot_state', 'zz_guard'),
            ('subscriptions', 'zz_merge'),
            ('subscriptions', 'zz_tomb'),
            ('subscriptions', 'zz_tomb_check'),
            ('subscriptions', 'zz_ver'),
            ('subscriptions', 'zzz_suppress'),
            ('sync_queue', 'zz_guard_intent'),
            ('sync_queue', 'zz_guard_result'),
            ('sync_queue', 'zz_merge'),
            ('sync_queue', 'zz_purge_guard'),
            ('sync_queue', 'zz_ver'),
            ('sync_queue', 'zzz_suppress'),
            ('sync_queue_action_pauses', 'zz_guard'),
            ('sync_queue_action_pauses', 'zz_owner'),
            ('sync_queue_action_pauses', 'zzz_suppress'),
            ('taste_model_versions', 'zz_guard'),
            ('taste_model_versions', 'zz_owner'),
            ('taste_model_versions', 'zzz_suppress'),
            ('taste_schedule', 'zz_guard'),
            ('track_artist_blocks', 'zz_merge'),
            ('track_artist_blocks', 'zz_tomb'),
            ('track_artist_blocks', 'zz_tomb_check'),
            ('track_artist_blocks', 'zz_ver'),
            ('track_artist_blocks', 'zzz_suppress'),
            ('track_artists', 'zz_guard'),
            ('track_artists', 'zz_owner'),
            ('track_artists', 'zzz_suppress'),
            ('track_comments', 'zz_guard'),
            ('track_comments', 'zz_owner'),
            ('track_comments', 'zzz_suppress'),
            ('track_work_aliases', 'zz_guard'),
            ('tracks', 'zz_guard'),
            ('tracks', 'zz_owner'),
            ('tracks', 'zzz_suppress'),
            ('transcription_sync_versions', 'zz_guard'),
            ('transcription_wire_state', 'zz_guard'),
            ('user_auras', 'zz_merge'),
            ('user_auras', 'zz_ver'),
            ('user_auras', 'zzz_suppress'),
            ('user_blocked_artists', 'zz_merge'),
            ('user_blocked_artists', 'zz_tomb'),
            ('user_blocked_artists', 'zz_tomb_check'),
            ('user_blocked_artists', 'zz_ver'),
            ('user_blocked_artists', 'zzz_suppress'),
            ('user_collection_sync', 'zz_guard'),
            ('user_events', 'zz_skip'),
            ('user_events', 'zzz_suppress'),
            ('user_followers', 'zz_guard'),
            ('user_followers', 'zz_owner'),
            ('user_followers', 'zzz_suppress'),
            ('user_followings', 'zz_guard_intent'),
            ('user_followings', 'zz_guard_result'),
            ('user_followings', 'zz_merge'),
            ('user_followings', 'zz_purge_guard'),
            ('user_followings', 'zz_ver'),
            ('user_followings', 'zzz_suppress'),
            ('user_like_ranks', 'zz_merge'),
            ('user_like_ranks', 'zz_tomb'),
            ('user_like_ranks', 'zz_tomb_check'),
            ('user_like_ranks', 'zz_ver'),
            ('user_like_ranks', 'zzz_suppress'),
            ('user_likes_playlists', 'zz_guard_intent'),
            ('user_likes_playlists', 'zz_guard_result'),
            ('user_likes_playlists', 'zz_merge'),
            ('user_likes_playlists', 'zz_purge_guard'),
            ('user_likes_playlists', 'zz_ver'),
            ('user_likes_playlists', 'zzz_suppress'),
            ('user_likes_tracks', 'zz_guard_intent'),
            ('user_likes_tracks', 'zz_guard_result'),
            ('user_likes_tracks', 'zz_merge'),
            ('user_likes_tracks', 'zz_purge_guard'),
            ('user_likes_tracks', 'zz_ver'),
            ('user_likes_tracks', 'zzz_suppress'),
            ('user_owned_playlists', 'zz_guard'),
            ('user_owned_playlists', 'zz_owner'),
            ('user_owned_playlists', 'zzz_suppress'),
            ('user_owned_tracks', 'zz_guard'),
            ('user_owned_tracks', 'zz_owner'),
            ('user_owned_tracks', 'zzz_suppress'),
            ('user_profiles', 'zz_guard'),
            ('user_profiles', 'zz_owner'),
            ('user_profiles', 'zzz_suppress'),
            ('user_superlike_ranks', 'zz_merge'),
            ('user_superlike_ranks', 'zz_tomb'),
            ('user_superlike_ranks', 'zz_tomb_check'),
            ('user_superlike_ranks', 'zz_ver'),
            ('user_superlike_ranks', 'zzz_suppress'),
            ('user_superlikes', 'zz_merge'),
            ('user_superlikes', 'zz_tomb'),
            ('user_superlikes', 'zz_tomb_check'),
            ('user_superlikes', 'zz_ver'),
            ('user_superlikes', 'zzz_suppress'),
            ('user_taste_vectors', 'zz_guard'),
            ('user_taste_vectors', 'zz_owner'),
            ('user_taste_vectors', 'zzz_suppress'),
            ('user_web_profiles', 'zz_guard'),
            ('user_web_profiles', 'zz_owner'),
            ('user_web_profiles', 'zzz_suppress'),
            ('users', 'zz_guard'),
            ('users', 'zz_owner'),
            ('users', 'zzz_suppress'),
            ('wanted_track_albums', 'zz_guard'),
            ('wanted_track_albums', 'zz_owner'),
            ('wanted_track_albums', 'zzz_suppress'),
            ('wanted_track_artists', 'zz_guard'),
            ('wanted_track_artists', 'zz_owner'),
            ('wanted_track_artists', 'zzz_suppress'),
            ('wanted_track_work_aliases', 'zz_guard'),
            ('wanted_tracks', 'zz_guard'),
            ('wanted_tracks', 'zz_owner'),
            ('wanted_tracks', 'zzz_suppress')
        ORDER BY 1, 2
    LOOP
        EXECUTE format('DROP TRIGGER %I ON public.%I', r.name, r.tbl);
    END LOOP;
END
$d$;
