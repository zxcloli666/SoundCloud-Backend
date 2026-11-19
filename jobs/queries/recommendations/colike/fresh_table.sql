CREATE TEMP TABLE colike_fresh (
    a uuid NOT NULL,
    b uuid NOT NULL,
    co int NOT NULL,
    weight real NOT NULL,
    PRIMARY KEY (a, b)
)
