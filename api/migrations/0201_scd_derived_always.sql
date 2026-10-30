-- no-transaction
BEGIN;
SET LOCAL lock_timeout = '5s';
ALTER FUNCTION public.artist_sc_account_flag() SET search_path = public;
CREATE OR REPLACE TRIGGER artist_sc_account_flag_trg
    AFTER INSERT OR UPDATE OF role, source, verified OR DELETE
    ON public.artist_sc_accounts
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.artist_sc_account_flag();
ALTER TABLE public.artist_sc_accounts ENABLE ALWAYS TRIGGER artist_sc_account_flag_trg;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
ALTER FUNCTION public.lyrics_cache_fts_refresh() SET search_path = public;
CREATE OR REPLACE TRIGGER lyrics_cache_fts_trg
    BEFORE INSERT OR UPDATE OF plain_text, synced_lrc
    ON public.lyrics_cache
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.lyrics_cache_fts_refresh();
ALTER TABLE public.lyrics_cache ENABLE ALWAYS TRIGGER lyrics_cache_fts_trg;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
ALTER FUNCTION public.track_artists_block_guard() SET search_path = public;
CREATE OR REPLACE TRIGGER track_artists_block_guard_trg
    BEFORE INSERT
    ON public.track_artists
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.track_artists_block_guard();
ALTER TABLE public.track_artists ENABLE ALWAYS TRIGGER track_artists_block_guard_trg;
COMMIT;

BEGIN;
SET LOCAL lock_timeout = '5s';
ALTER FUNCTION public.tracks_primary_block_guard() SET search_path = public;
ALTER FUNCTION public.lyrics_lookup_track_state_refresh() SET search_path = public;
CREATE OR REPLACE TRIGGER tracks_primary_block_guard_trg
    BEFORE INSERT OR UPDATE OF primary_artist_id
    ON public.tracks
    FOR EACH ROW
    WHEN (public.scd_derived())
    EXECUTE FUNCTION public.tracks_primary_block_guard();
CREATE OR REPLACE TRIGGER tracks_lyrics_lookup_state_refresh
    AFTER INSERT OR UPDATE OF
        sc_track_id,
        title,
        metadata_artist,
        uploader_username,
        duration_ms,
        genius_song_id,
        genius_url,
        release_date,
        sc_created_at,
        index_priority,
        pipeline_held
    ON public.tracks
    FOR EACH ROW
    WHEN (NOT NEW.pipeline_held AND public.scd_derived())
    EXECUTE FUNCTION public.lyrics_lookup_track_state_refresh();
ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER tracks_primary_block_guard_trg;
ALTER TABLE public.tracks ENABLE ALWAYS TRIGGER tracks_lyrics_lookup_state_refresh;
COMMIT;
