ALTER TABLE playlist_membership_state
    ADD COLUMN owner_sc_user_id text NOT NULL DEFAULT '';

UPDATE playlist_membership_state AS state
SET owner_sc_user_id = coalesce(playlist.owner_sc_user_id, '')
FROM playlists AS playlist
WHERE playlist.urn = state.playlist_urn;

CREATE INDEX playlist_membership_state_owner_due_idx
    ON playlist_membership_state (owner_sc_user_id, next_reconcile_at, playlist_urn)
    WHERE sync_status <> 'clean';

CREATE TABLE playlist_sweep_cursor (
    id boolean PRIMARY KEY DEFAULT true CHECK (id),
    last_owner_sc_user_id text,
    updated_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO playlist_sweep_cursor (id) VALUES (true);
