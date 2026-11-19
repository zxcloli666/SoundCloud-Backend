CREATE TEMP TABLE colike_likers AS
SELECT artist_id, count(*) AS likers
FROM colike_users
GROUP BY artist_id;
ALTER TABLE colike_likers ADD PRIMARY KEY (artist_id);
CREATE TEMP TABLE colike_fresh (
    a uuid NOT NULL,
    b uuid NOT NULL,
    co int NOT NULL,
    weight real NOT NULL,
    PRIMARY KEY (a, b)
);
ANALYZE colike_users, colike_likers
