# Phase 4 plan: Persistence (SQLite + save/load)

## Purpose and scope

This phase adds server-side persistence for Hall of Fame entries and population
snapshots. The database is deliberately kept off the hot path; writes only
happen at safe boundaries such as end-of-generation checkpoints or explicit
manual saves. The server exposes HTTP endpoints for import/export and manual
save triggers.

## Architecture narrative

Persistence is isolated in its own module. The sim server calls persistence at
well-defined points: end of generation and manual save. The schema is simple
and stores population snapshots as JSON, keeping the system flexible for future
changes. SQLite is chosen because it keeps the stack self-contained without a
separate database server, which matches the project goals.

## Decisions locked for Phase 4

This phase uses SQLite via `better-sqlite3`, stores the database at
`./data/slither.db`, and enables WAL mode with synchronous set to NORMAL.
Checkpoints run every generation by default but remain configurable, and a
manual save endpoint is supported.

## Database schema

```sql
CREATE TABLE IF NOT EXISTS hof_entries (
  id INTEGER PRIMARY KEY,
  created_at INTEGER,
  gen INTEGER,
  seed INTEGER,
  fitness REAL,
  points REAL,
  length REAL,
  genome_json TEXT
);

CREATE TABLE IF NOT EXISTS population_snapshots (
  id INTEGER PRIMARY KEY,
  created_at INTEGER,
  gen INTEGER,
  payload_json TEXT
);

CREATE TABLE IF NOT EXISTS players (
  id TEXT PRIMARY KEY,
  name TEXT,
  created_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_hof_gen ON hof_entries(gen);
CREATE INDEX IF NOT EXISTS idx_snap_gen ON population_snapshots(gen);
```

The schema stores snapshots as JSON for flexibility and avoids migrations in
this phase. The `players` table exists for future use but is not required for
sim operation.

## Persistence module API

```ts
export function initDb(dbPath: string): Database;
export function saveHofEntry(db: Database, entry: HallOfFameEntry): void;
export function saveSnapshot(db: Database, payload: PopulationFilePayload): number;
export function loadLatestSnapshot(db: Database): PopulationFilePayload | null;
export function listSnapshots(db: Database, limit: number): SnapshotMeta[];
export function exportSnapshot(db: Database, id: number): PopulationFilePayload;
```

The `saveSnapshot` method returns the inserted row id so callers can report it
back to the UI or CLI tools.

## HTTP API

Endpoints are minimal and intended for local use:

- `GET /health` returns `{ ok: true, tick, clients }`.
- `POST /api/save` triggers an immediate snapshot and returns an id.
- `GET /api/export/latest` returns the latest snapshot JSON.
- `POST /api/import` loads a new population snapshot into the current world.

The import handler must validate that the snapshot `cfgHash` matches the
server's current config hash unless an explicit override flag is provided. This
prevents confusing behavior where a population evolved under one brain layout
is imported into a different layout and fails silently.

## Validation rules

Payload validation occurs before any database writes or world modifications.
The `generation` value must be finite, `genomes` must be a non-empty array,
each genome must have `archKey` and `weights`, and the `weights` arrays must be
bounded by a size limit. Payloads larger than 50 MB are rejected.

Snapshots should include configuration metadata alongside the population. At a
minimum, store `cfgHash` and `worldSeed` in the snapshot payload so imports can
detect mismatches. If the config hash does not match the current server config,
the import should either reject or require a manual override, rather than
silently loading an incompatible population.

## Checkpoint timing

On each generation end, if `gen % checkpointEveryGenerations == 0`, save a
snapshot. Manual save always writes immediately. The checkpoint interval is
configurable via server config.

## Detailed design notes

The persistence layer should use prepared statements and keep them cached for
the life of the process. This reduces overhead and keeps DB interactions
predictable. Write operations are small and infrequent, so a synchronous API is
acceptable in this phase as long as writes are kept out of the tick loop.

Imports are treated as full population replacement. The server should pause
the sim loop briefly while applying an import to avoid partial state. If the
payload fails validation, the server must reject it without modifying the
current world. This guarantees that a malformed import does not corrupt the
running simulation.

All persisted JSON is treated as opaque from the DB's perspective. The DB does
not attempt to query inside JSON, which keeps schema evolution simple. If the
JSON format changes in a future phase, older snapshots remain readable as long
as the import validator can handle legacy fields.

## Tests

Unit tests verify that the database schema is created and that snapshots round
trip correctly. Integration tests use the HTTP API to import and export data.

## Footguns

`better-sqlite3` is native and requires build tools for install. Never write to
the DB from inside the tick loop. Large JSON payloads must be size capped to
avoid memory spikes and event loop stalls.

## Acceptance criteria

The DB file is created in `./data`, snapshots can be saved and loaded, and the
import/export endpoints work.

## Execution checklist

- [x] Add `better-sqlite3` dependency
- [x] Implement `persistence.ts`
- [x] Implement `httpApi.ts`
- [x] Add config keys for `dbPath` and `checkpointEveryGenerations`
- [x] Wire checkpointing to generation end
- [x] Add persistence tests

## Function-by-function pseudocode

### `persistence.ts`

```text
function initDb(path):
  ensure data directory exists
  db = open sqlite file
  db.exec(PRAGMA journal_mode=WAL)
  db.exec(PRAGMA synchronous=NORMAL)
  db.exec(schema SQL)
  return db

function saveSnapshot(db, payload):
  validate payload
  json = JSON.stringify(payload)
  if json.size > limit: throw
  insert into population_snapshots
  return inserted id

function loadLatestSnapshot(db):
  row = select latest snapshot
  if none: return null
  return JSON.parse(row.payload_json)
```

### `httpApi.ts`

```text
function handleSave(req, res):
  snapshotId = saveSnapshot(db, currentWorldPayload)
  respond with { ok:true, snapshotId }

function handleImport(req, res):
  payload = parse JSON body
  validate payload
  simServer.replacePopulation(payload)
  respond { ok:true }
```

## Error handling and edge cases

If the DB cannot be opened, the server should fail fast on startup with a clear
error. Invalid JSON in `POST /api/import` returns a 400 response and does not
modify the running world. If a snapshot exceeds the size limit, the server
rejects it and logs a warning. The sim loop should pause briefly during import
to avoid partial state updates.

## Sample payloads and example session transcript

Example HTTP request:

```http
POST /api/save HTTP/1.1
Host: localhost:5174
Content-Type: application/json
```

Example HTTP response:

```json
{ "ok": true, "snapshotId": 12 }
```

Example WS session transcript (unchanged from earlier phases):

```text
client -> server: {"type":"hello","clientType":"ui","version":1}
server -> client: {"type":"welcome",...}
server -> client: <binary frame>
```

## Test matrix

| Test name | Setup / input | Expected result | Failure cases to verify |
| --- | --- | --- | --- |
| db_init | Start server with empty data dir | Tables created, no crash | Missing directory causes crash |
| save_snapshot | Call POST /api/save | Snapshot row inserted | Snapshot id missing or null |
| import_invalid_json | POST malformed JSON | 400 response, world unchanged | World partially updated |
| snapshot_size_limit | Payload > 50MB | 413 or 400 response | Memory spike or crash |
