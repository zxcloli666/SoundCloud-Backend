DELETE
FROM wanted_track_work_aliases
WHERE wanted_track_id = ANY ($1)
