SELECT EXISTS(
    SELECT 1
    FROM playlists AS playlist
    LEFT JOIN entity_reads AS reads ON reads.entity_urn = playlist.urn
    WHERE playlist.urn = $1
      AND playlist.sharing = 'public'
      AND playlist.deleted_at IS NULL
      AND greatest(playlist.last_read_at, reads.last_read_at) >= now() - interval '7 days'
) AS "public!"
