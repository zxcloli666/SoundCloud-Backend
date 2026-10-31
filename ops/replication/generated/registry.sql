CREATE OR REPLACE FUNCTION public.scd_kinds()
RETURNS TABLE (kind text, optimistic boolean)
LANGUAGE sql STABLE
AS $f$
VALUES
    ('app_token.reject', true),
    ('cold_refresh.clear', true),
    ('comment.post', true),
    ('connection.drop_orphan', false),
    ('connection.login', true),
    ('job.enqueue', false),
    ('lyrics.wake', true),
    ('match.album_track', true),
    ('match.genius_song', true),
    ('match.sc_account', true),
    ('match.wanted_link', true),
    ('observe.playlists', true),
    ('observe.profile', true),
    ('observe.tracks', true),
    ('observe.users', true),
    ('owned.delete', true),
    ('owned.insert', true),
    ('playlist.delete', true),
    ('playlist.edit', true),
    ('playlist.membership_op', true),
    ('sync_queue.purge', false),
    ('sync_queue.revive', false),
    ('track.delete', true),
    ('track.edit', true),
    ('track.release_held', true),
    ('track.s3_state', true),
    ('track.storage_lost', true),
    ('track.too_long', true)
$f$;

CREATE OR REPLACE FUNCTION public.scd_table_class(tbl text)
RETURNS text
LANGUAGE sql STABLE
AS $f$
SELECT v.cls FROM (VALUES
    ('admin_maintenance_runs', 'C'),
    ('ai_resolver_budget', 'P'),
    ('ai_resolver_cache', 'P'),
    ('album_artists', 'C'),
    ('album_tracks', 'C'),
    ('albums', 'C'),
    ('artist_attribution_revalidation', 'C'),
    ('artist_colike', 'C'),
    ('artist_coplay', 'C'),
    ('artist_sc_accounts', 'C'),
    ('artist_socials', 'C'),
    ('artists', 'C'),
    ('audio_index_wire_state', 'P'),
    ('background_job_enqueues', 'P'),
    ('background_job_failures', 'C'),
    ('background_jobs', 'P'),
    ('background_schedules', 'P'),
    ('catalog_audience', 'C'),
    ('catalog_collection_cursors', 'P'),
    ('catalog_collection_seen', 'P'),
    ('catalog_collection_sync', 'C'),
    ('catalog_merge_state', 'P'),
    ('catalog_work_links', 'P'),
    ('cdn_tracks', 'A'),
    ('cluster_bandit_stats', 'L'),
    ('cmd_dead', 'P'),
    ('cmd_done', 'X'),
    ('cmd_outbox', 'X'),
    ('cmd_pending', 'L'),
    ('cmd_purge', 'X'),
    ('cmd_reassert', 'X'),
    ('diag_bundles', 'A'),
    ('diag_events', 'A'),
    ('diag_issue_days', 'A'),
    ('diag_issue_origins', 'A'),
    ('discover_promoted', 'A'),
    ('discover_settings', 'A'),
    ('discover_tag_counts', 'C'),
    ('disliked_tracks', 'A'),
    ('enrich_calibration', 'P'),
    ('entity_reads', 'K'),
    ('featured_items', 'A'),
    ('job_status', 'C'),
    ('link_requests', 'A'),
    ('listening_history', 'A'),
    ('login_requests', 'A'),
    ('lyrics_cache', 'C'),
    ('lyrics_embedding_wire_state', 'P'),
    ('lyrics_lookup_backfill_state', 'P'),
    ('lyrics_lookup_state', 'C'),
    ('oauth_app_request_cooldowns', 'A'),
    ('oauth_app_token_issuance_reservations', 'P'),
    ('oauth_app_token_refresh_state', 'P'),
    ('oauth_app_tokens', 'C'),
    ('oauth_app_tokens_local', 'L'),
    ('oauth_apps', 'A'),
    ('pipeline_event_receipts', 'P'),
    ('playlist_legacy_membership_intents', 'C'),
    ('playlist_membership_operations', 'C'),
    ('playlist_membership_state', 'C'),
    ('playlist_reconcile_runs', 'P'),
    ('playlist_remote_observations', 'C'),
    ('playlist_remote_snapshot_tracks', 'P'),
    ('playlist_remote_snapshots', 'C'),
    ('playlist_sweep_cursor', 'P'),
    ('playlist_track_projection', 'C'),
    ('playlists', 'C'),
    ('recommendation_quality_models', 'P'),
    ('repl_checksums', 'K'),
    ('repl_purges', 'X'),
    ('repl_repair', 'X'),
    ('repl_resend', 'L'),
    ('repl_tombstones', 'A'),
    ('sc_egress_health', 'A'),
    ('sc_track_counters', 'K'),
    ('scd_unpublished_columns', 'L'),
    ('search_terms', 'C'),
    ('sessions', 'A'),
    ('soundcloud_connections', 'M'),
    ('star_track_tiers', 'P'),
    ('storage_event_repairs', 'P'),
    ('storage_event_state', 'P'),
    ('subscription_snapshot_state', 'P'),
    ('subscriptions', 'A'),
    ('sync_queue', 'M'),
    ('sync_queue_action_pauses', 'C'),
    ('taste_model_versions', 'C'),
    ('taste_schedule', 'P'),
    ('track_artist_blocks', 'A'),
    ('track_artists', 'C'),
    ('track_comments', 'C'),
    ('track_work_aliases', 'P'),
    ('tracks', 'C'),
    ('transcription_sync_versions', 'P'),
    ('transcription_wire_state', 'P'),
    ('user_auras', 'A'),
    ('user_blocked_artists', 'A'),
    ('user_collection_sync', 'P'),
    ('user_events', 'A'),
    ('user_followers', 'C'),
    ('user_followings', 'M'),
    ('user_like_ranks', 'A'),
    ('user_likes_playlists', 'M'),
    ('user_likes_tracks', 'M'),
    ('user_owned_playlists', 'C'),
    ('user_owned_tracks', 'C'),
    ('user_profiles', 'C'),
    ('user_superlike_ranks', 'A'),
    ('user_superlikes', 'A'),
    ('user_taste_vectors', 'C'),
    ('user_web_profiles', 'C'),
    ('users', 'C'),
    ('wanted_track_albums', 'C'),
    ('wanted_track_artists', 'C'),
    ('wanted_track_work_aliases', 'P'),
    ('wanted_tracks', 'C')
) AS v (tbl, cls) WHERE v.tbl = $1
$f$;

CREATE OR REPLACE FUNCTION public.scd_core_columns()
RETURNS TABLE (tbl text, col text)
LANGUAGE sql STABLE
AS $f$
VALUES
    ('artists', 'has_sc_account'),
    ('lyrics_cache', 'fts')
$f$;

CREATE OR REPLACE FUNCTION public.scd_reassert_always(tbl text)
RETURNS boolean
LANGUAGE sql STABLE
AS $f$
SELECT $1 = ANY (ARRAY['artist_sc_accounts', 'artists', 'playlist_membership_operations', 'playlist_track_projection', 'track_comments', 'wanted_tracks']::text[])
$f$;

CREATE OR REPLACE FUNCTION public.scd_resend(tbl text)
RETURNS void
LANGUAGE plpgsql
AS $f$
BEGIN
    CASE tbl
        WHEN 'admin_maintenance_runs' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'admin_maintenance_runs', jsonb_build_object('kind', t.kind) FROM public.admin_maintenance_runs t ON CONFLICT DO NOTHING;
        WHEN 'album_artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'album_artists', jsonb_build_object('album_id', t.album_id, 'artist_id', t.artist_id, 'role', t.role) FROM public.album_artists t ON CONFLICT DO NOTHING;
        WHEN 'album_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'album_tracks', jsonb_build_object('album_id', t.album_id, 'track_id', t.track_id) FROM public.album_tracks t ON CONFLICT DO NOTHING;
        WHEN 'albums' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'albums', jsonb_build_object('id', t.id) FROM public.albums t ON CONFLICT DO NOTHING;
        WHEN 'artist_attribution_revalidation' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_attribution_revalidation', jsonb_build_object('singleton', t.singleton) FROM public.artist_attribution_revalidation t ON CONFLICT DO NOTHING;
        WHEN 'artist_colike' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_colike', jsonb_build_object('a_id', t.a_id, 'b_id', t.b_id) FROM public.artist_colike t ON CONFLICT DO NOTHING;
        WHEN 'artist_coplay' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_coplay', jsonb_build_object('a_id', t.a_id, 'b_id', t.b_id) FROM public.artist_coplay t ON CONFLICT DO NOTHING;
        WHEN 'artist_sc_accounts' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_sc_accounts', jsonb_build_object('artist_id', t.artist_id, 'sc_user_id', t.sc_user_id) FROM public.artist_sc_accounts t ON CONFLICT DO NOTHING;
        WHEN 'artist_socials' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artist_socials', jsonb_build_object('artist_id', t.artist_id, 'url', t.url) FROM public.artist_socials t ON CONFLICT DO NOTHING;
        WHEN 'artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'artists', jsonb_build_object('id', t.id) FROM public.artists t ON CONFLICT DO NOTHING;
        WHEN 'background_job_failures' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'background_job_failures', jsonb_build_object('id', t.id, 'generation', t.generation) FROM public.background_job_failures t ON CONFLICT DO NOTHING;
        WHEN 'catalog_audience' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'catalog_audience', jsonb_build_object('subject_urn', t.subject_urn, 'relation', t.relation, 'user_urn', t.user_urn) FROM public.catalog_audience t ON CONFLICT DO NOTHING;
        WHEN 'catalog_collection_sync' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'catalog_collection_sync', jsonb_build_object('subject_id', t.subject_id, 'collection', t.collection, 'scope', t.scope) FROM public.catalog_collection_sync t ON CONFLICT DO NOTHING;
        WHEN 'cdn_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'cdn_tracks', jsonb_build_object('id', t.id) FROM public.cdn_tracks t ON CONFLICT DO NOTHING;
        WHEN 'diag_bundles' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'diag_bundles', jsonb_build_object('code', t.code) FROM public.diag_bundles t ON CONFLICT DO NOTHING;
        WHEN 'diag_events' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'diag_events', jsonb_build_object('id', t.id) FROM public.diag_events t ON CONFLICT DO NOTHING;
        WHEN 'diag_issue_days' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'diag_issue_days', jsonb_build_object('fingerprint', t.fingerprint, 'day', t.day, 'host', t.host, 'app_version', t.app_version) FROM public.diag_issue_days t ON CONFLICT DO NOTHING;
        WHEN 'diag_issue_origins' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'diag_issue_origins', jsonb_build_object('fingerprint', t.fingerprint, 'day', t.day, 'host', t.host, 'origin', t.origin) FROM public.diag_issue_origins t ON CONFLICT DO NOTHING;
        WHEN 'discover_promoted' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'discover_promoted', jsonb_build_object('id', t.id) FROM public.discover_promoted t ON CONFLICT DO NOTHING;
        WHEN 'discover_settings' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'discover_settings', jsonb_build_object('id', t.id) FROM public.discover_settings t ON CONFLICT DO NOTHING;
        WHEN 'discover_tag_counts' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'discover_tag_counts', jsonb_build_object('tag', t.tag) FROM public.discover_tag_counts t ON CONFLICT DO NOTHING;
        WHEN 'disliked_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'disliked_tracks', jsonb_build_object('id', t.id) FROM public.disliked_tracks t ON CONFLICT DO NOTHING;
        WHEN 'entity_reads' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'entity_reads', jsonb_build_object('entity_urn', t.entity_urn) FROM public.entity_reads t ON CONFLICT DO NOTHING;
        WHEN 'featured_items' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'featured_items', jsonb_build_object('id', t.id) FROM public.featured_items t ON CONFLICT DO NOTHING;
        WHEN 'job_status' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'job_status', jsonb_build_object('kind', t.kind, 'dedup_key', t.dedup_key) FROM public.job_status t ON CONFLICT DO NOTHING;
        WHEN 'link_requests' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'link_requests', jsonb_build_object('id', t.id) FROM public.link_requests t ON CONFLICT DO NOTHING;
        WHEN 'listening_history' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'listening_history', jsonb_build_object('id', t.id) FROM public.listening_history t ON CONFLICT DO NOTHING;
        WHEN 'login_requests' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'login_requests', jsonb_build_object('id', t.id) FROM public.login_requests t ON CONFLICT DO NOTHING;
        WHEN 'lyrics_cache' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'lyrics_cache', jsonb_build_object('sc_track_id', t.sc_track_id) FROM public.lyrics_cache t ON CONFLICT DO NOTHING;
        WHEN 'lyrics_lookup_state' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'lyrics_lookup_state', jsonb_build_object('track_id', t.track_id) FROM public.lyrics_lookup_state t ON CONFLICT DO NOTHING;
        WHEN 'oauth_app_request_cooldowns' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'oauth_app_request_cooldowns', jsonb_build_object('oauth_app_id', t.oauth_app_id) FROM public.oauth_app_request_cooldowns t ON CONFLICT DO NOTHING;
        WHEN 'oauth_app_tokens' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'oauth_app_tokens', jsonb_build_object('oauth_app_id', t.oauth_app_id) FROM public.oauth_app_tokens t ON CONFLICT DO NOTHING;
        WHEN 'oauth_apps' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'oauth_apps', jsonb_build_object('id', t.id) FROM public.oauth_apps t ON CONFLICT DO NOTHING;
        WHEN 'playlist_legacy_membership_intents' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_legacy_membership_intents', jsonb_build_object('archive_id', t.archive_id) FROM public.playlist_legacy_membership_intents t ON CONFLICT DO NOTHING;
        WHEN 'playlist_membership_operations' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_membership_operations', jsonb_build_object('operation_id', t.operation_id) FROM public.playlist_membership_operations t ON CONFLICT DO NOTHING;
        WHEN 'playlist_membership_state' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_membership_state', jsonb_build_object('playlist_urn', t.playlist_urn) FROM public.playlist_membership_state t ON CONFLICT DO NOTHING;
        WHEN 'playlist_remote_observations' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_remote_observations', jsonb_build_object('id', t.id) FROM public.playlist_remote_observations t ON CONFLICT DO NOTHING;
        WHEN 'playlist_remote_snapshots' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_remote_snapshots', jsonb_build_object('id', t.id) FROM public.playlist_remote_snapshots t ON CONFLICT DO NOTHING;
        WHEN 'playlist_track_projection' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlist_track_projection', jsonb_build_object('playlist_urn', t.playlist_urn, 'position', t."position") FROM public.playlist_track_projection t ON CONFLICT DO NOTHING;
        WHEN 'playlists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'playlists', jsonb_build_object('urn', t.urn) FROM public.playlists t ON CONFLICT DO NOTHING;
        WHEN 'repl_checksums' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'repl_checksums', jsonb_build_object('node', t.node, 'tbl', t.tbl, 'range', t.range) FROM public.repl_checksums t ON CONFLICT DO NOTHING;
        WHEN 'repl_tombstones' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'repl_tombstones', jsonb_build_object('tbl', t.tbl, 'pk', t.pk, 'v', t.v) FROM public.repl_tombstones t ON CONFLICT DO NOTHING;
        WHEN 'sc_egress_health' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sc_egress_health', jsonb_build_object('channel', t.channel) FROM public.sc_egress_health t ON CONFLICT DO NOTHING;
        WHEN 'sc_track_counters' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sc_track_counters', jsonb_build_object('sc_track_id', t.sc_track_id) FROM public.sc_track_counters t ON CONFLICT DO NOTHING;
        WHEN 'search_terms' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'search_terms', jsonb_build_object('word', t.word) FROM public.search_terms t ON CONFLICT DO NOTHING;
        WHEN 'sessions' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sessions', jsonb_build_object('id', t.id) FROM public.sessions t ON CONFLICT DO NOTHING;
        WHEN 'soundcloud_connections' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'soundcloud_connections', jsonb_build_object('id', t.id) FROM public.soundcloud_connections t ON CONFLICT DO NOTHING;
        WHEN 'subscriptions' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'subscriptions', jsonb_build_object('user_urn', t.user_urn) FROM public.subscriptions t ON CONFLICT DO NOTHING;
        WHEN 'sync_queue' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sync_queue', jsonb_build_object('id', t.id) FROM public.sync_queue t ON CONFLICT DO NOTHING;
        WHEN 'sync_queue_action_pauses' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'sync_queue_action_pauses', jsonb_build_object('oauth_app_id', t.oauth_app_id, 'action_type', t.action_type) FROM public.sync_queue_action_pauses t ON CONFLICT DO NOTHING;
        WHEN 'taste_model_versions' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'taste_model_versions', jsonb_build_object('version', t.version) FROM public.taste_model_versions t ON CONFLICT DO NOTHING;
        WHEN 'track_artist_blocks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'track_artist_blocks', jsonb_build_object('track_id', t.track_id, 'artist_id', t.artist_id) FROM public.track_artist_blocks t ON CONFLICT DO NOTHING;
        WHEN 'track_artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'track_artists', jsonb_build_object('track_id', t.track_id, 'artist_id', t.artist_id, 'role', t.role) FROM public.track_artists t ON CONFLICT DO NOTHING;
        WHEN 'track_comments' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'track_comments', jsonb_build_object('id', t.id) FROM public.track_comments t ON CONFLICT DO NOTHING;
        WHEN 'tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'tracks', jsonb_build_object('id', t.id) FROM public.tracks t ON CONFLICT DO NOTHING;
        WHEN 'user_auras' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_auras', jsonb_build_object('user_urn', t.user_urn) FROM public.user_auras t ON CONFLICT DO NOTHING;
        WHEN 'user_blocked_artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_blocked_artists', jsonb_build_object('sc_user_id', t.sc_user_id, 'kind', t.kind, 'target_id', t.target_id) FROM public.user_blocked_artists t ON CONFLICT DO NOTHING;
        WHEN 'user_events' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_events', jsonb_build_object('id', t.id) FROM public.user_events t ON CONFLICT DO NOTHING;
        WHEN 'user_followers' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_followers', jsonb_build_object('user_id', t.user_id, 'target_user_urn', t.target_user_urn) FROM public.user_followers t ON CONFLICT DO NOTHING;
        WHEN 'user_followings' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_followings', jsonb_build_object('user_id', t.user_id, 'target_user_urn', t.target_user_urn) FROM public.user_followings t ON CONFLICT DO NOTHING;
        WHEN 'user_like_ranks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_like_ranks', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_like_ranks t ON CONFLICT DO NOTHING;
        WHEN 'user_likes_playlists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_likes_playlists', jsonb_build_object('user_id', t.user_id, 'playlist_urn', t.playlist_urn) FROM public.user_likes_playlists t ON CONFLICT DO NOTHING;
        WHEN 'user_likes_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_likes_tracks', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_likes_tracks t ON CONFLICT DO NOTHING;
        WHEN 'user_owned_playlists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_owned_playlists', jsonb_build_object('user_id', t.user_id, 'playlist_urn', t.playlist_urn) FROM public.user_owned_playlists t ON CONFLICT DO NOTHING;
        WHEN 'user_owned_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_owned_tracks', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_owned_tracks t ON CONFLICT DO NOTHING;
        WHEN 'user_profiles' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_profiles', jsonb_build_object('soundcloud_user_id', t.soundcloud_user_id) FROM public.user_profiles t ON CONFLICT DO NOTHING;
        WHEN 'user_superlike_ranks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_superlike_ranks', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_superlike_ranks t ON CONFLICT DO NOTHING;
        WHEN 'user_superlikes' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_superlikes', jsonb_build_object('user_id', t.user_id, 'sc_track_id', t.sc_track_id) FROM public.user_superlikes t ON CONFLICT DO NOTHING;
        WHEN 'user_taste_vectors' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_taste_vectors', jsonb_build_object('sc_user_id', t.sc_user_id, 'version', t.version) FROM public.user_taste_vectors t ON CONFLICT DO NOTHING;
        WHEN 'user_web_profiles' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'user_web_profiles', jsonb_build_object('sc_user_id', t.sc_user_id) FROM public.user_web_profiles t ON CONFLICT DO NOTHING;
        WHEN 'users' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'users', jsonb_build_object('sc_user_id', t.sc_user_id) FROM public.users t ON CONFLICT DO NOTHING;
        WHEN 'wanted_track_albums' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'wanted_track_albums', jsonb_build_object('wanted_track_id', t.wanted_track_id, 'album_id', t.album_id) FROM public.wanted_track_albums t ON CONFLICT DO NOTHING;
        WHEN 'wanted_track_artists' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'wanted_track_artists', jsonb_build_object('wanted_track_id', t.wanted_track_id, 'artist_id', t.artist_id, 'role', t.role) FROM public.wanted_track_artists t ON CONFLICT DO NOTHING;
        WHEN 'wanted_tracks' THEN
            INSERT INTO public.repl_resend (tbl, pk) SELECT 'wanted_tracks', jsonb_build_object('id', t.id) FROM public.wanted_tracks t ON CONFLICT DO NOTHING;
        ELSE
            RAISE EXCEPTION 'public.scd_resend: unknown table %', tbl;
    END CASE;
    RETURN;
END
$f$;
CREATE OR REPLACE FUNCTION public.scd_resend_all()
RETURNS bigint
LANGUAGE plpgsql
AS $f$
DECLARE
    sent text[] := CASE public.scd_role() WHEN 'core' THEN ARRAY['C', 'M', 'A'] ELSE ARRAY['M', 'A', 'K'] END;
    r record;
BEGIN
    FOR r IN SELECT v.tbl FROM (VALUES
        ('admin_maintenance_runs', 'C'),
        ('album_artists', 'C'),
        ('album_tracks', 'C'),
        ('albums', 'C'),
        ('artist_attribution_revalidation', 'C'),
        ('artist_coplay', 'C'),
        ('artist_sc_accounts', 'C'),
        ('artist_socials', 'C'),
        ('artists', 'C'),
        ('background_job_failures', 'C'),
        ('catalog_audience', 'C'),
        ('catalog_collection_sync', 'C'),
        ('cdn_tracks', 'A'),
        ('diag_bundles', 'A'),
        ('diag_events', 'A'),
        ('diag_issue_days', 'A'),
        ('diag_issue_origins', 'A'),
        ('discover_promoted', 'A'),
        ('discover_settings', 'A'),
        ('discover_tag_counts', 'C'),
        ('disliked_tracks', 'A'),
        ('entity_reads', 'K'),
        ('featured_items', 'A'),
        ('job_status', 'C'),
        ('link_requests', 'A'),
        ('listening_history', 'A'),
        ('login_requests', 'A'),
        ('lyrics_cache', 'C'),
        ('lyrics_lookup_state', 'C'),
        ('oauth_app_request_cooldowns', 'A'),
        ('oauth_app_tokens', 'C'),
        ('oauth_apps', 'A'),
        ('playlist_legacy_membership_intents', 'C'),
        ('playlist_membership_operations', 'C'),
        ('playlist_membership_state', 'C'),
        ('playlist_remote_observations', 'C'),
        ('playlist_remote_snapshots', 'C'),
        ('playlist_track_projection', 'C'),
        ('playlists', 'C'),
        ('repl_checksums', 'K'),
        ('repl_tombstones', 'A'),
        ('sc_egress_health', 'A'),
        ('sc_track_counters', 'K'),
        ('search_terms', 'C'),
        ('sessions', 'A'),
        ('soundcloud_connections', 'M'),
        ('subscriptions', 'A'),
        ('sync_queue', 'M'),
        ('sync_queue_action_pauses', 'C'),
        ('taste_model_versions', 'C'),
        ('track_artist_blocks', 'A'),
        ('track_artists', 'C'),
        ('track_comments', 'C'),
        ('tracks', 'C'),
        ('user_auras', 'A'),
        ('user_blocked_artists', 'A'),
        ('user_events', 'A'),
        ('user_followers', 'C'),
        ('user_followings', 'M'),
        ('user_like_ranks', 'A'),
        ('user_likes_playlists', 'M'),
        ('user_likes_tracks', 'M'),
        ('user_owned_playlists', 'C'),
        ('user_owned_tracks', 'C'),
        ('user_profiles', 'C'),
        ('user_superlike_ranks', 'A'),
        ('user_superlikes', 'A'),
        ('user_taste_vectors', 'C'),
        ('user_web_profiles', 'C'),
        ('users', 'C'),
        ('wanted_track_albums', 'C'),
        ('wanted_track_artists', 'C'),
        ('wanted_tracks', 'C')
    ) AS v (tbl, cls) WHERE v.cls = ANY (sent) ORDER BY v.tbl LOOP
        PERFORM public.scd_resend(r.tbl);
    END LOOP;
    RETURN (SELECT count(*) FROM public.repl_resend);
END
$f$;
