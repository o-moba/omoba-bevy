# Career database migrations

The running server uses PostgreSQL exclusively. The implemented migration and
adapter contract are documented in [postgres/README.md](postgres/README.md).

The earlier unshipped SQLite prototype was superseded before runtime integration.
Its old fixture logs are historical task evidence, not PostgreSQL verification.
Never apply a SQLite schema to a career database.
