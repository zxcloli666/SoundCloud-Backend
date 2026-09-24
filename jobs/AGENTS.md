# Jobs service

- Write Rust 2024 for people reading top to bottom. Production code explains itself through names and structure.
- Do not add comments unless an invariant cannot be expressed safely in code. Never narrate implementation history.
- Do not use `unwrap`, `expect`, unchecked indexing, warning suppression, or detached tasks.
- Keep modules focused and files small enough to review without jumping around.
- Put static SQL in `queries/` and validate it online against core plus ops migrations.
- Treat core and ops PostgreSQL as mandatory, independent databases with separate fast and bulk pools.
- Do not hold a database connection across network I/O.
- Every queue terminal write is fenced by job id, lease id, and lease generation.
- Claim only work that has execution capacity, bound every recovery batch, and supervise every long-lived task.
