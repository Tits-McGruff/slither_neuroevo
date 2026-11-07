//! Bounded legacy reads on the caller-owned SQLite connection; no schema or publication work.

use super::*;

pub(super) fn legacy_sqlite_parent_columns(
    connection: &Connection,
) -> Result<Vec<String>, CheckpointError> {
    let mut statement = connection
        .prepare("PRAGMA table_info(population_snapshots)")
        .map_err(legacy_sqlite_error)?;
    let mut rows = statement.query([]).map_err(legacy_sqlite_error)?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next().map_err(legacy_sqlite_error)? {
        columns.push(row.get(1).map_err(legacy_sqlite_error)?);
    }
    Ok(columns)
}

pub(super) fn legacy_sqlite_column_length(
    connection: &Connection,
    snapshot_id: i64,
    column: &str,
) -> Result<Option<usize>, CheckpointError> {
    let query = match column {
        "payload_json" => {
            "SELECT length(CAST(payload_json AS BLOB)) FROM population_snapshots WHERE id = ?1"
        }
        "settings_json" => {
            "SELECT length(CAST(settings_json AS BLOB)) FROM population_snapshots WHERE id = ?1"
        }
        "updates_json" => {
            "SELECT length(CAST(updates_json AS BLOB)) FROM population_snapshots WHERE id = ?1"
        }
        "genomes_blob" => "SELECT length(genomes_blob) FROM population_snapshots WHERE id = ?1",
        _ => {
            return Err(CheckpointError::format(
                "LEGACY_SQLITE_SCHEMA",
                "unsupported legacy SQLite column",
            ));
        }
    };
    let length: Option<i64> = connection
        .query_row(query, [snapshot_id], |row| row.get(0))
        .map_err(legacy_sqlite_error)?;
    length
        .map(|length| {
            usize::try_from(length).map_err(|_| {
                CheckpointError::format(
                    "LEGACY_SQLITE_LIMIT",
                    "legacy SQLite column length is invalid",
                )
            })
        })
        .transpose()
}

pub(super) fn read_legacy_sqlite_json_column<T: DeserializeOwned>(
    connection: &Connection,
    snapshot_id: i64,
    column: &str,
    maximum_bytes: usize,
) -> Result<Option<T>, CheckpointError> {
    let Some(length) = legacy_sqlite_column_length(connection, snapshot_id, column)? else {
        return Ok(None);
    };
    if length == 0 || length > maximum_bytes {
        return Err(CheckpointError::format(
            "LEGACY_SQLITE_LIMIT",
            format!("legacy {column} is empty or exceeds its compatibility limit"),
        ));
    }
    let mut input = connection
        .blob_open(MAIN_DB, "population_snapshots", column, snapshot_id, true)
        .map_err(legacy_sqlite_error)?;
    if input.len() != length {
        return Err(CheckpointError::format(
            "LEGACY_SQLITE_READ",
            format!("legacy {column} changed during conversion"),
        ));
    }
    let mut decoder = serde_json::Deserializer::from_reader(BufReader::new(&mut input));
    let value = T::deserialize(&mut decoder).map_err(|error| {
        CheckpointError::format(
            "LEGACY_SQLITE_JSON",
            format!("legacy {column} JSON is invalid: {error}"),
        )
    })?;
    decoder.end().map_err(|error| {
        CheckpointError::format(
            "LEGACY_SQLITE_JSON",
            format!("legacy {column} JSON has trailing data: {error}"),
        )
    })?;
    Ok(Some(value))
}

pub(super) fn read_legacy_sqlite_gzip_genomes(
    connection: &Connection,
    snapshot_id: i64,
    compressed_bytes: usize,
    expected_population: Option<usize>,
) -> Result<Vec<LegacyGenomeWire>, CheckpointError> {
    let input = connection
        .blob_open(
            MAIN_DB,
            "population_snapshots",
            "genomes_blob",
            snapshot_id,
            true,
        )
        .map_err(legacy_sqlite_error)?;
    if input.len() != compressed_bytes {
        return Err(CheckpointError::format(
            "LEGACY_SQLITE_READ",
            "legacy combined population changed during conversion",
        ));
    }
    let mut decoder = MultiGzDecoder::new(BufReader::new(input));
    let mut genomes = Vec::new();
    if let Some(expected) = expected_population {
        genomes.try_reserve_exact(expected).map_err(|_| {
            CheckpointError::format("ALLOCATION", "legacy genome table allocation failed")
        })?;
    }
    let mut decoded_bytes = 0usize;
    loop {
        let mut prefix = [0u8; 4];
        let first = decoder
            .read(&mut prefix[..1])
            .map_err(legacy_sqlite_io_error)?;
        if first == 0 {
            break;
        }
        decoder
            .read_exact(&mut prefix[1..])
            .map_err(legacy_sqlite_io_error)?;
        decoded_bytes = decoded_bytes.checked_add(prefix.len()).ok_or_else(|| {
            CheckpointError::format("LEGACY_SQLITE_GZIP_LIMIT", "legacy gzip size overflow")
        })?;
        let length = u32::from_le_bytes(prefix) as usize;
        if length == 0 || length > MAX_LEGACY_GENOME_JSON_BYTES {
            return Err(CheckpointError::format(
                "LEGACY_SQLITE_GZIP_LIMIT",
                "legacy genome JSON is empty or exceeds 64 MiB",
            ));
        }
        decoded_bytes = decoded_bytes.checked_add(length).ok_or_else(|| {
            CheckpointError::format("LEGACY_SQLITE_GZIP_LIMIT", "legacy gzip size overflow")
        })?;
        if decoded_bytes > MAX_LEGACY_JSON_BYTES as usize {
            return Err(CheckpointError::format(
                "LEGACY_SQLITE_GZIP_LIMIT",
                "legacy combined population exceeds 512 MiB after decompression",
            ));
        }
        let mut encoded = Vec::new();
        encoded.try_reserve_exact(length).map_err(|_| {
            CheckpointError::format("ALLOCATION", "legacy genome JSON allocation failed")
        })?;
        encoded.resize(length, 0);
        decoder
            .read_exact(&mut encoded)
            .map_err(legacy_sqlite_io_error)?;
        let genome = serde_json::from_slice(&encoded).map_err(|error| {
            CheckpointError::format(
                "LEGACY_SQLITE_GZIP_JSON",
                format!("legacy genome {} JSON is invalid: {error}", genomes.len()),
            )
        })?;
        genomes.push(genome);
        if genomes.len() > MAX_LEGACY_GENOMES {
            return Err(CheckpointError::format(
                "LEGACY_SQLITE_POPULATION",
                "legacy population exceeds 300 genomes",
            ));
        }
    }
    if expected_population.is_some_and(|expected| expected != genomes.len()) {
        return Err(CheckpointError::format(
            "LEGACY_SQLITE_POPULATION",
            "legacy combined population count does not match its parent row",
        ));
    }
    Ok(genomes)
}

pub(super) fn legacy_sqlite_error(error: rusqlite::Error) -> CheckpointError {
    CheckpointError::format(
        "LEGACY_SQLITE",
        format!("legacy SQLite read failed: {error}"),
    )
}

pub(super) fn legacy_sqlite_io_error(error: io::Error) -> CheckpointError {
    CheckpointError::format(
        "LEGACY_SQLITE_READ",
        format!("legacy SQLite stream failed: {error}"),
    )
}
