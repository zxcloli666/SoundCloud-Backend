DELETE
FROM track_work_aliases
WHERE track_id = ANY ($1)
