SELECT EXISTS (
    SELECT 1
    FROM repl_tombstones
    WHERE tbl = 'sessions'
      AND pk = jsonb_build_object('id', $1::uuid)
) AS "logged_out!"
