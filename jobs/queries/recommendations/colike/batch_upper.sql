SELECT coalesce(
           least(
               (
                   SELECT a
                   FROM colike_fresh
                   WHERE a >= $1
                   ORDER BY a
                   OFFSET $2 - 1
                   LIMIT 1
               ),
               (
                   SELECT a_id
                   FROM artist_colike
                   WHERE a_id >= $1
                   ORDER BY a_id
                   OFFSET $2 - 1
                   LIMIT 1
               )
           ),
           greatest(
               (
                   SELECT a
                   FROM colike_fresh
                   WHERE a >= $1
                   ORDER BY a DESC
                   LIMIT 1
               ),
               (
                   SELECT a_id
                   FROM artist_colike
                   WHERE a_id >= $1
                   ORDER BY a_id DESC
                   LIMIT 1
               )
           )
       ) AS upper
