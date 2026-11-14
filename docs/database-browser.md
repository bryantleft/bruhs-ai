# Read-only project databases

Open **Files → Databases** beside a project thread on web or desktop. Connect an existing SQLite file or a Postgres server on `localhost`, `127.0.0.1`, or `::1`, then inspect tables, describe columns, browse rows, or run a read-only query. This is a local working-tree implementation; it has not been published as a release.

Connections belong to the exact canonical working folder. A worktree has its own connections and agent-data opt-ins; it does not inherit another folder's access. The companion server owns the connection catalog and query execution for both interfaces. It must be running for either interface to use this surface.

## Use and sharing

1. Choose **Connect**, name the connection, and enter its file path or local Postgres fields. A newly entered password exists transiently in the form and travels in a same-origin request to the native credential service, which saves it in the existing OS keychain store. Saving clears the form. Editing a connection never retrieves a saved password into either interface. A blank password retains the saved value only when the target is unchanged.
2. Choose a schema/table, then use **Browse rows** or **Describe columns**. Filtering is a literal substring match on the named column. Sorting and filtering use quoted identifiers; an unknown column produces an error. Changed filters require **Browse rows** before paging. Queries and table pages are separate results, and each page reads the current database rather than a saved transaction snapshot.
3. Select result rows and choose **Add selected to draft** to copy only those rows into the current thread's unsent draft. Review and send it when ready. Manual query results and SQL are not automatically saved as history or sent to an agent.
4. **Agent access: off** is the default. Turn it on explicitly for each connection to let interactive agents whose turn runs from this working folder discover that connection, inspect tables/columns, and run read-only queries. This grants read access across the connection, independently of selected draft attachments; selecting rows alone grants no ongoing access. Changing the target resets this permission. Revocation invalidates stale revisions, cancels this host's active reads, and blocks new agent reads. Refresh reconciles a connection changed in another window.

The injected `bruhs_database` MCP server exposes `database_connections`, `database_schema`, and `database_query`. Its project scope comes from the turn's working folder, not model-supplied arguments. Configuration, grants, and database writes are not model tools. Saved connection passwords are never returned by these tools. These app controls share the existing local Host/Origin boundary; they are not an OS sandbox against another program running as the same account. See [local server access](local-server-access.md).

Bruhs' private app-database paths are excluded. Use the existing workspace, people, and transcript surfaces and their permissions to read that data.

## Read boundary and limits

SQLite opens a fresh connection with engine read-only and no-follow flags, `query_only`, an authorizer that rejects writes/attachments/unsafe pragmas and functions, and a single row-returning statement check. It also disables SQLite's double-quoted-string fallback so a mistyped column cannot silently become a constant. The file's current canonical target is rechecked before each operation. The connection's SQL/value limits and progress cancellation do not alter the SQLite settings used by meeting storage. See [SQLite authorizers](https://www.sqlite.org/c3ref/set_authorizer.html) and [connection limits](https://www.sqlite.org/limits.html).

Postgres uses a dedicated connection and an engine `READ ONLY` transaction whose access mode is fixed before user SQL. Read-only transactions alone do not contain a function that opens another connection. Therefore user SQL is parsed before preparation and restricted to SELECT queries over ordinary heap tables and a fixed set of built-in expressions/functions. CTEs, arbitrary functions, views, RLS, inheritance, custom types/access methods, virtual generated columns, and expression/partial indexes are outside this version's supported query subset. The exact accepted built-ins and AST forms are in [`postgres_policy.rs`](../bruhs-core/src/database/postgres_policy.rs). Unsupported input fails visibly; it is not retried with broader access. Schema inspection can list tables and views and describe their columns, including relations whose rows this version cannot query. It does not inspect arbitrary functions or CTEs. Relation locks, catalog checks, and a fixed `pg_catalog` search path protect the supported path. See [Postgres transaction modes](https://www.postgresql.org/docs/current/sql-set-transaction.html).

Data results contain at most 50 rows and 32 KiB of encoded JSON. The row count reuses the app's existing history-page contract, and half the 64 KiB MCP frame leaves space for transport metadata. The 10-second execution budget leaves half of the native HTTP caller's 20-second budget for delivery and cleanup. PostgreSQL backend frames are bounded before the driver allocates a large payload; a value that cannot fit must be narrowed in SQL. Schema lists use the byte budget rather than the data-grid row limit. A partial result is marked, and table continuation advances by rows actually returned, including byte-limited pages. NULL is distinct from text, and numeric values travel as text to preserve int64/decimal precision.

## Verification

The browser fixture uses the production WASM build, fresh browser storage, and mocked APIs. It checks the Files → Databases UI flow at 640×480 and touch 390×844, selection into an unsent draft, private-field clearing, blocked-query errors, cancellation, literal HTML rendering, and panel overflow. Separate Rust tests use real SQLite and real Postgres, the desktop's production HTTP client against the server router, native GPUI controls, and project/grant/revision checks. The GPUI check uses a control harness rather than a launched desktop session. Credential tests use an isolated in-memory substitute for the existing OS keychain adapter; they do not prove an OS keychain prompt or write. Mocked browser tests prove neither engine behavior nor native integration. Real Postgres verification requires the separate ignored-test command below; the ordinary commands alone do not cover it.

Run the ordinary checks from the repository root:

```sh
CARGO_INCREMENTAL=0 cargo test -p bruhs-core database --locked
CARGO_INCREMENTAL=0 cargo test -p bruhs-server database_native_http --locked
CARGO_INCREMENTAL=0 cargo test -p bruhs-desktop database_native_controls --locked
```

The external stdio helper is exercised only when its executable is supplied. Build it first, then run the same HTTP test with that executable:

```sh
CARGO_INCREMENTAL=0 cargo build -p bruhs-server --locked
BRUHS_DATABASE_MCP_EXECUTABLE="$PWD/target/debug/bruhs-server" CARGO_INCREMENTAL=0 cargo test -p bruhs-server database_native_http --locked -- --nocapture
```

For Postgres, supply an installed server's `bin` directory to the fixture helper. It creates a private temporary cluster, generated password, loopback port, and synthetic data; it never selects an existing cluster. The JSON receipt contains that generated fixture password and must remain private and uncommitted.

```sh
python3 scripts/database_fixture.py start --pg-bin /path/to/postgres/bin --out /tmp/bruhs-postgres-fixture.json
BRUHS_DATABASE_TEST_FIXTURE=/tmp/bruhs-postgres-fixture.json CARGO_INCREMENTAL=0 cargo test -p bruhs-core live_postgres_readonly_boundary --locked -- --ignored --nocapture
python3 scripts/database_fixture.py stop /tmp/bruhs-postgres-fixture.json --remove
```

To run the browser fixture, first build from `bruhs-web` with `CARGO_INCREMENTAL=0 env -u NO_COLOR trunk build --dist ../target/database-ui-dist`, then run `node bruhs-web/tests/database_browser.mjs` from the root. It requires Playwright and a compatible Chromium installation; set `BRUHS_CHROME` to its executable when needed. Screenshots and assertions are written under `target/database-ui-qa`.

Recorded local validation and source hashes are in [the database browser evidence](../bench/database-browser/results/2026-11-13/validation.json).
