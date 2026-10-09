# Stage 2 persistence and save-format inventory

This is the source-defined compatibility inventory required before the Rust
archive reader is implemented. It distinguishes current-source facts,
Git-history evidence, locally available artifacts and owner files that have not
been supplied. It does not narrow or retire a compatibility reader.

## Evidence identity

- Current-source baseline:
  `2baf4176c666e3d8b620c32aba8fb7e2cd8071b7`.
- Historical compressed-BLOB introduction:
  `05e0b53c02854558008444ccfc59a7c47da51ac5`, parent
  `60749784b3d2ecbaf50d482256e4b7341359868b`.
- Current bounded-reader introduction:
  `308c6f0dd91eca8091bc75dcf08ca87904da2d50`.
- Historical browser-local population implementations:
  `3989d2653c1df97ebc6ca6284a5eb14cdb7c48c9` (localStorage) and
  `2ea606eb66cc440f4bd180ad7581c509b80f829b` (IndexedDB fallback).
- Removal of browser-local population loading:
  `258ac69e80df411fa724ad16f1b2cb19e1ae210c`.
- Current workspace database evidence:
  `windows-5800x/database-slither-db.json`, which identifies
  `data/slither.db` by SHA-256
  `9b8774387cff7aa82e64dbf75f4807d06ada9072814d24b71b8c76c4fe4bd8a4`.

The current-source inspection used:

```powershell
rg -n "genomes_blob|snapshot_genomes|SNAPSHOT_FORMAT_VERSION|exportSnapshotJsonChunks|validateSnapshotPayload|PopulationFilePayload|readAsText|exportToFile" server src
rg -n "localeCompare|compileBrainSpec|archKey" src/brains server scripts/stage2/graph-baseline.ts
```

The Git-history inspection used:

```powershell
git log --all --oneline -S "genomes_blob" -- server/persistence.ts
git log --all --oneline -G "\.json\.gz|pako|fflate|gzipSync|gunzipSync" -- .
git diff 05e0b53^ 05e0b53 -- server/persistence.ts
git grep -n -I -E "genomes_blob|gzip|gunzip|\.json\.gz|pako|fflate|compress|decompress|exportToFile|importFromFile" 05e0b53 -- ":!node_modules" ":!docs/todo/archive"
git rev-list --objects --all | Select-String -Pattern "(?i)(slither_neuroevo_gen|population|snapshot|save).*(\.json|\.gz|\.zip|\.tar|\.zst)$"
```

The last command found no tracked user save/archive object. That is a statement
about this Git repository, not about files the owner may have downloaded,
copied to the server or retained outside the checkout.

## Source-defined format matrix

| Representation | Where it exists | Large population representation | Identity/resume meaning | Current reader or writer | Required v3 treatment |
|---|---|---|---|---|---|
| Current format-2 SQLite checkpoint | `population_snapshots` parent plus ordered `snapshot_genomes` child rows | One little-endian Float32 BLOB and SHA-256 per dense population slot | Exact generation-boundary metadata includes run/seed/config/graph, RNG and allocator state; `population-export` rows are explicitly non-resumable | `server/persistence.ts::saveCheckpoint`, `loadCurrentCheckpoint`; current writer | Preserve a bounded read-only migration reader. Iterate child rows rather than `.all()` when the Rust migration reader is implemented. |
| Historical SQLite combined gzip BLOB | Format-null/0 parent row plus `genomes_blob` | Gzip stream of repeated 4-byte little-endian JSON byte length followed by one UTF-8 genome JSON record; each genome still contains decimal weights | Population-compatible legacy state; older records may omit exact-resume fields | `deserializeLegacyGenomes` and `loadLegacyCheckpoint`; read-only, 512 MiB compressed/decompressed and 64 MiB per-record limits | Preserve bounded read-only conversion. Read SQLite BLOB slices into a streaming gzip/JSON converter; never rewrite the source row. |
| Older SQLite all-in-parent JSON | Format-null/0 `payload_json` with no `genomes_blob` | Complete `genomes` array with decimal weights in one JSON value | Population-compatible legacy state; completeness depends on fields present | `loadLegacyCheckpoint`; current implementation materializes the parent text | Preserve bounded read-only conversion using SQLite byte slices and a streaming JSON visitor; do not select a population-sized TEXT value into JavaScript. |
| Current/older browser population file | `slither_neuroevo_gen<generation>.json` | Complete `genomes` array with decimal weights; optional HoF also repeats decimal genome data | Unversioned population-transfer file, not guaranteed exact resume | `src/storage.ts::importFromFile` and `src/main.ts::importServerSnapshot`; current browser reads/parses/stringifies the whole file | Preserve a direct-upload, bounded server-side legacy JSON reader. Classify incomplete files as `legacy-population-import`; never route them through the generic 50 MiB JSON body. |
| Current HTTP JSON export stream | `GET /api/export/latest` | JSON tokens are yielded one genome at a time by the server, but the browser calls `response.json()`, attaches UI fields/HoF, stringifies again and creates a Blob | Exports the selected current or legacy database snapshot in a JSON-compatible population shape | `exportSnapshotJsonChunks`, `sendJsonChunks`, `exportServerSnapshot` | Replace normal export with one direct archive-v1 download. Retain only bounded compatibility needed to migrate existing JSON files. |
| Standalone graph-spec JSON | `slither_neuroevo_graph_spec.json` | One small plain `GraphSpec` JSON object; no population weights | Auxiliary graph definition only; it cannot resume or restore a population by itself | `src/main.ts` exports the current graph-editor draft with `exportJsonToFile`; restoration is through the JSON editor, graph preset path or a population file carrying `graphSpec` | Preserve graph-definition compatibility independently of the population archive. Archive v1 also carries the exact graph/layout needed to interpret its weights. |
| Historical browser-local population state | `slither_neuroevo_pop` in localStorage or IndexedDB | Structured clone or JSON population object with decimal genome weights | Old browser-local population state, not a standalone file | Current code retains the key only so Clear All removes it; it is not loaded as authority | Do not silently promise file compatibility. If the owner still has valuable browser storage, export/copy it before clearing that browser profile and review a one-time converter. |
| New archive v1 / checkpoint v3 | Managed immutable file or ordinary downloaded file | Independently selected raw packed Float32 or bounded shuffled-Zstandard numeric entries inside the approved USTAR envelope | One self-contained resumable experiment at an ordinary generation boundary | Not implemented during Stage 2 | This becomes the only new export format and managed-checkpoint payload. |

## Historical findings

At commit `60749784...`, SQLite stored the complete population inside
`payload_json`. Commit `05e0b53...` added `genomes_blob` and changed new writes
to:

1. stringify every genome separately;
2. prefix each UTF-8 JSON record with a four-byte little-endian length;
3. concatenate all prefixes and records;
4. gzip the combined buffer; and
5. store it in the SQLite parent row while leaving an empty `genomes` array in
   `payload_json`.

This is the prior compressed population representation found in source
history. It reduced database bytes relative to one giant decimal JSON value,
but it was a SQLite BLOB rather than an ordinary downloaded archive and its
writer still accumulated the complete encoded population before gzip.

Commit `308c6f0...` stopped new BLOB writes, retained the old format as a
bounded read-only reader and introduced current format 2 with per-slot raw
Float32 child rows. Current source rejects a format-2 parent that unexpectedly
also contains a legacy BLOB.

Across the inspected `src/storage.ts` history, standalone population downloads
use a `.json` filename, `JSON.stringify`, a browser Blob and
`FileReader.readAsText` on import. No tracked source implementation of a
standalone `.json.gz`, `.zip`, `.tar` or `.zst` population export was found.
The old `pako` package appeared in historical lockfiles but no inspected
browser save call path used it. This does not prove that an untracked or
external compressed save never existed.

## Current limits and known unbounded paths

- The generic HTTP JSON body limit is 50 MiB.
- The current legacy database reader allows at most 512 MiB compressed,
  512 MiB decompressed and 64 MiB for one genome JSON record.
- Current format-2 validation allows at most 10,000 population entries and
  2,000,000 Float32 weights per genome.
- The current format-2 loader uses `.all()` for all `snapshot_genomes` rows.
- The current legacy BLOB reader selects the complete SQLite BLOB and then
  uses synchronous whole-output gunzip.
- The old all-parent JSON reader selects and parses the complete TEXT value.
- The current browser-file path reads and parses the complete file and then
  stringifies its population into the 50 MiB request.

The first three are current limits, not approved final archive limits. The
last four are migration defects that the bounded compatibility readers must
replace.

## Locally available owner-data inventory

The repository contains one ignored database:
`data/slither.db` (6,041,600 bytes). Its retained read-only inventory records
two format-2 run-start checkpoints, 110 child genome rows, no legacy BLOB, no
Hall-of-Fame row and no graph preset. It is not a legacy-format fixture.

On 2026-07-29, a filename/metadata-only search of the owner's
`Downloads`, `Desktop`, `Documents` and `source/repos` directories found no
additional file whose name contained `slither` or `neuroevo`, and no relevant
`.json.gz`, `.sqlite`, `.sqlite3` or `.db` file outside this checkout. File
contents were not searched. This limited scan does not cover the Debian VM,
Unraid shares, browser download history, other disks, backups, renamed files
or directories not named above.

The exact filename-only scan commands were:

```powershell
$scanRoots = @('C:\Users\jlow8\Downloads','C:\Users\jlow8\Desktop','C:\Users\jlow8\Documents')
Get-ChildItem -LiteralPath $scanRoots -File -Recurse -ErrorAction SilentlyContinue |
  Where-Object { $_.Name -match '(?i)(slither|neuroevo)' -or $_.Name -match '(?i)\.(json\.gz|sqlite|sqlite3|db)$' } |
  Select-Object FullName,Length,LastWriteTimeUtc
Get-ChildItem -LiteralPath C:\Users\jlow8\source\repos -File -Recurse -ErrorAction SilentlyContinue |
  Where-Object { $_.Name -match '(?i)(slither|neuroevo)' -and $_.Name -match '(?i)\.(json|json\.gz|sqlite|sqlite3|db|gz|zip|zst)$' } |
  Select-Object FullName,Length,LastWriteTimeUtc
```

An independent review found one unrelated Visual Studio
`Browse.VC.db` outside this checkout when it broadened the second search to all
database filenames. It is not Slither data. No candidate Slither save was
opened or hashed by these searches.

Therefore the owner-file inventory requirement remains open. Before any reader
is narrowed or retired, the implementation needs the paths or copies of:

- every database the owner may want to resume or convert;
- every downloaded population JSON or compressed save/archive the owner may
  want to import;
- any browser profile whose old localStorage/IndexedDB population is valuable;
- graph/preset exports associated with those saves; and
- the separate RL trainer's exact Protocol 2 contract fixture.

Inventorying those files records only format, size, digest, provenance and
whether exact or population-compatible restoration is possible. Conversion
uses copies and never mutates the original.

## Reproduced compatibility tests

The command

```powershell
node .\node_modules\vitest\vitest.mjs run server/persistence.test.ts server/recoveryPhase7.persistence.test.ts --reporter=dot
```

passed 27 tests on 2026-07-29. The exercised cases include current format-2
Float32/checksum round trips, historical combined-gzip loading and bounded
gzip failure reporting, current JSON token streaming, exact checkpoint
reconstruction and bootstrapping a legacy database without rewriting its
historical row.

This proves only those generated fixtures. It does not prove compatibility
with an owner file, bounded SQLite slice behavior, streaming conversion of an
all-parent JSON population, archive v1 or the future Rust reader.

## Compatibility work still required

- Retain a generated fixture for all-parent format-null/0 JSON without a BLOB.
- Prove the installed SQLite/`better-sqlite3` bounded-slice behavior before
  choosing it for large legacy BLOB/TEXT migration.
- Inventory actual owner files and add one redacted or generated fixture per
  distinct format found.
- Record Windows and Debian graph ordering/layout for each real compatible
  graph before converting weights.
- Implement direct-upload format detection and bounded one-genome-at-a-time
  legacy JSON/gzip conversion in the later approved persistence stage.
- Leave every original legacy row and file unchanged.
