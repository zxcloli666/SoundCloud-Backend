SELECT wanted.id AS "id!",
       wanted.title AS "title!",
       ARRAY(SELECT alias.alias_key
             FROM wanted_track_work_aliases AS alias
             WHERE alias.wanted_track_id = wanted.id) AS "aliases!: Vec<String>"
FROM wanted_tracks AS wanted
WHERE wanted.id > $1
ORDER BY wanted.id
LIMIT $2
