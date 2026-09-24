DELETE FROM sync_queue
WHERE user_id = $1 AND target_urn = $2 AND action_type = 'playlist_update'
