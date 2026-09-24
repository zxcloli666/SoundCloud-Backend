SELECT track.storage_state,
       track.index_state,
       track.needs_duration_resolve,
       track.updated_at,
       EXISTS (
           SELECT 1
           FROM audio_index_wire_state AS wire
           WHERE wire.sc_track_id = track.sc_track_id
             AND wire.status = 'pending'
             AND wire.dispatched_at > now() - $2::bigint * interval '1 second'
       ) AS "index_in_flight!"
FROM tracks AS track
WHERE track.sc_track_id = $1
