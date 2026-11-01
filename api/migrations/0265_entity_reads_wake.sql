SET LOCAL lock_timeout = '5s';

CREATE OR REPLACE FUNCTION public.playlist_read_wake()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_catalog
AS $f$
BEGIN
    UPDATE public.playlist_membership_state AS state
    SET sync_status = 'retry_wait',
        next_reconcile_at = clock_timestamp(),
        reconcile_failure_streak = 0,
        updated_at = clock_timestamp()
    FROM public.playlists AS playlist
    WHERE state.playlist_urn = NEW.entity_urn
      AND playlist.urn = state.playlist_urn
      AND playlist.sharing = 'public'
      AND state.sync_status = 'auth_required'
      AND (playlist.last_read_at IS NULL OR playlist.last_read_at < NEW.last_read_at - INTERVAL '6 hours');
    UPDATE public.playlists AS playlist
    SET last_read_at = NEW.last_read_at
    WHERE playlist.urn = NEW.entity_urn
      AND (playlist.last_read_at IS NULL OR playlist.last_read_at < NEW.last_read_at - INTERVAL '6 hours');
    RETURN NULL;
END
$f$;

CREATE OR REPLACE TRIGGER entity_reads_playlist_wake AFTER INSERT OR UPDATE OF last_read_at ON public.entity_reads
FOR EACH ROW WHEN (NEW.entity_urn LIKE 'soundcloud:playlists:%' AND public.scd_derived())
EXECUTE FUNCTION public.playlist_read_wake();

ALTER TABLE public.entity_reads ENABLE ALWAYS TRIGGER entity_reads_playlist_wake;
