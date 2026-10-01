"""Measure validated Rust save roles with bounded blocks and exact JS numeric text sizes.

Requires Python 3.14 (standard-library Zstandard) and Node 24 or newer.
Codec timings belong to this independent Python/Zstandard oracle, not Rust production.
JavaScript formats one scalar at a time; no population JSON is assembled or written.
"""
import argparse
import hashlib
import json
import pathlib
import struct
import subprocess
import sys
import tarfile
import time
from compression import zstd

# Match the production SFZ1 decoded block limit and frame format.
BLOCK_BYTES = 1024 * 1024
ZSTD_OPTIONS = {zstd.CompressionParameter.compression_level: 1,
                zstd.CompressionParameter.window_log: 20,
                zstd.CompressionParameter.checksum_flag: 0,
                zstd.CompressionParameter.content_size_flag: 1}
NUMERIC_ENCODINGS = {"raw-f32le-v1", "f32le-shuffle4-zstd-v1"}


def unshuffle(data):
    """Restore one bounded four-lane Float32 block without converting its values."""
    count = len(data) // 4
    result = bytearray(len(data))
    for lane in range(4):
        result[lane::4] = data[lane * count:(lane + 1) * count]
    return result


def shuffle(data):
    """Pack the same four byte lanes used by the Rust adaptive candidate."""
    return b"".join(data[lane::4] for lane in range(4))


def decode_frame(frame, count):
    """Require one bounded dictionary-free frame and exactly its declared output."""
    decoder = zstd.ZstdDecompressor(options={zstd.DecompressionParameter.window_log_max: 20})
    decoded = decoder.decompress(frame, max_length=count * 4 + 1)
    if not decoder.eof or decoder.unused_data or len(decoded) != count * 4:
        raise ValueError("invalid bounded Zstandard frame")
    return unshuffle(decoded)


def blocks(source, encoding):
    """Yield at most one MiB of decoded input per block plus its stored envelope."""
    if encoding == "f32le-shuffle4-zstd-v1":
        while header := source.read(12):
            if len(header) != 12 or header[:4] != b"SFZ1":
                raise ValueError("invalid SFZ1 block")
            count, size = struct.unpack("<II", header[4:])
            if count < 1 or count > BLOCK_BYTES // 4 or size < 1 or size > 2 * BLOCK_BYTES:
                raise ValueError("SFZ1 block exceeds bounds")
            frame = source.read(size)
            if len(frame) != size:
                raise ValueError("truncated Zstandard frame")
            yield decode_frame(frame, count), header + frame
    else:
        while raw := source.read(BLOCK_BYTES):
            if encoding in NUMERIC_ENCODINGS and len(raw) % 4:
                raise ValueError("unaligned Float32 role")
            yield raw, raw


def measure_role(archive, role, node, formatter):
    """Check role hashes, measure the candidate and stream raw bits to the JS counter."""
    entry = archive.getmember(role["path"])
    if not entry.isfile() or entry.size != int(role["storedBytesHex"], 16):
        raise ValueError("role is not the declared regular file")
    numeric = role["encoding"] in NUMERIC_ENCODINGS
    if not numeric and role["encoding"] not in {"raw-binary-v1", "raw-history-v1", "raw-hof-index-v1"}:
        raise ValueError("unsupported role encoding")
    process = subprocess.Popen([node, str(formatter), str(int(role["decodedCountHex"], 16))],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE) if numeric else None
    decoded_hash = hashlib.sha256()
    stored_hash = hashlib.sha256()
    candidate_hash = hashlib.sha256()
    decoded_bytes = stored_bytes = candidate_bytes = block_count = 0
    decode_seconds = encode_seconds = candidate_decode_seconds = 0.0
    max_raw = max_frame = 0
    try:
        with archive.extractfile(entry) as source:
            iterator = iter(blocks(source, role["encoding"]))
            while True:
                start = time.perf_counter()
                try:
                    raw, stored = next(iterator)
                except StopIteration:
                    break
                decode_seconds += time.perf_counter() - start
                decoded_hash.update(raw)
                stored_hash.update(stored)
                decoded_bytes += len(raw)
                stored_bytes += len(stored)
                max_raw = max(max_raw, len(raw))
                block_count += 1
                if numeric:
                    start = time.perf_counter()
                    frame = zstd.compress(shuffle(raw), options=ZSTD_OPTIONS)
                    envelope = b"SFZ1" + struct.pack("<II", len(raw) // 4, len(frame)) + frame
                    encode_seconds += time.perf_counter() - start
                    candidate_hash.update(envelope)
                    candidate_bytes += len(envelope)
                    max_frame = max(max_frame, len(frame))
                    start = time.perf_counter()
                    if decode_frame(frame, len(raw) // 4) != raw:
                        raise ValueError("candidate changed Float32 bits")
                    candidate_decode_seconds += time.perf_counter() - start
                    process.stdin.write(raw)
        if decoded_bytes != int(role["decodedBytesHex"], 16) or stored_bytes != entry.size or decoded_hash.hexdigest() != role["logicalSha256"]:
            raise ValueError("role length or decoded SHA-256 mismatch")
        result = {"role": role["role"], "encoding": role["encoding"], "storedBytes": stored_bytes,
                  "rawBytes": decoded_bytes, "logicalSha256": decoded_hash.hexdigest(), "bitExact": True}
        if numeric:
            process.stdin.close()
            output = process.stdout.read()
            error = process.stderr.read()
            if process.wait() != 0:
                raise ValueError("JS decimal counter failed: " + error.decode())
            text = json.loads(output)
            selected = "f32le-shuffle4-zstd-v1" if candidate_bytes < decoded_bytes else "raw-f32le-v1"
            if selected != role["encoding"] or stored_bytes != min(candidate_bytes, decoded_bytes):
                raise ValueError(f"{role['role']}: stored {stored_bytes} bytes using {role['encoding']}, "
                                 f"but measured candidate={candidate_bytes}, raw={decoded_bytes}")
            candidate_matches = role["encoding"] != "f32le-shuffle4-zstd-v1" or candidate_hash.digest() == stored_hash.digest()
            if not candidate_matches:
                raise ValueError("independent candidate bytes differ from production stored bytes")
            result.update(shuffledCandidateBytes=candidate_bytes, adaptiveSelectionVerified=True,
                          storedCandidateByteExact=candidate_matches, decodedBlocks=block_count,
                          largestRawBlockBytes=max_raw, largestCandidateFrameBytes=max_frame,
                          readDecodeMs=decode_seconds * 1000, candidateEncodeMs=encode_seconds * 1000,
                          candidateDecodeMs=candidate_decode_seconds * 1000,
                          candidateEncodeRawMiBPerSecond=decoded_bytes / 1024**2 / encode_seconds if encode_seconds else None,
                          candidateDecodeRawMiBPerSecond=decoded_bytes / 1024**2 / candidate_decode_seconds if candidate_decode_seconds else None,
                          decimalJson=text)
        return result
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        if process is not None:
            for pipe in (process.stdin, process.stdout, process.stderr):
                try:
                    pipe.close()
                except BrokenPipeError:
                    pass  # Preserve the original validation error if the formatter already exited.


def main():
    """Read a fixed-role ordinary save and write only compact metrics to a new file."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--archive", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--node", default="node")
    args = parser.parse_args()
    if args.output.exists():
        raise ValueError("output already exists")
    formatter = pathlib.Path(__file__).with_name("f32-json-size.ts")
    with tarfile.open(args.archive, mode="r:") as archive:
        members = archive.getmembers()
        names = [member.name for member in members]
        if len(names) != 9 or len(set(names)) != 9 or any(not member.isfile() for member in members):
            raise ValueError("expected exactly nine unique regular entries")
        member = archive.getmember("manifest.json")
        if not member.isfile() or member.size > BLOCK_BYTES:
            raise ValueError("unbounded manifest")
        with archive.extractfile(member) as source:
            manifest = json.load(source)
        if len(manifest["roles"]) != 8:
            raise ValueError("expected eight save roles")
        if manifest["magic"] != "slither-neuroevo-save" or manifest["archiveVersion"] != 1:
            raise ValueError("unsupported save version")
        expected_roles = ["checkpoint", "graph", "population-index", "population-weights",
                          "population-recurrent", "history", "hall-of-fame-index", "hall-of-fame-weights"]
        if [role["role"] for role in manifest["roles"]] != expected_roles:
            raise ValueError("unexpected role ordering")
        paths = [role["path"] for role in manifest["roles"]]
        if set(names) != {"manifest.json", *paths} or any(
                pathlib.PurePosixPath(path).is_absolute() or ".." in pathlib.PurePosixPath(path).parts
                or "\\" in path for path in paths):
            raise ValueError("unsafe or undeclared archive entry")
        roles = []
        for role in manifest["roles"]:
            measured = measure_role(archive, role, args.node, formatter)
            roles.append(measured)
            print(f"role={role['role']} raw={measured['rawBytes']} stored={measured['storedBytes']}", file=sys.stderr, flush=True)
    root = hashlib.sha256(b"slither-neuroevo-save-root\0v1\0")
    root.update(struct.pack("<I", len(roles)))
    for role in roles:
        name = role["role"].encode()
        root.update(struct.pack("<H", len(name)) + name + struct.pack("<Q", role["rawBytes"]) + bytes.fromhex(role["logicalSha256"]))
    if root.hexdigest() != manifest["logicalRootSha256"]:
        raise ValueError("save root mismatch")
    archive_bytes = args.archive.stat().st_size
    with args.archive.open("rb") as source:
        archive_hash = hashlib.file_digest(source, "sha256").hexdigest()
    numeric_json_bytes = sum(role["decimalJson"]["jsonArrayBytes"] for role in roles if "decimalJson" in role)
    result = {"schemaVersion": 1, "file": args.archive.name, "archiveBytes": archive_bytes,
              "archiveSha256": archive_hash,
              "runId": manifest["runId"], "generation": manifest["generationHex"],
              "completedStep": manifest["completedStepHex"],
              "checkpointId": manifest["checkpointLogicalRootSha256"], "saveRoot": root.hexdigest(),
              "populationCount": int(manifest["checkpointManifest"]["populationCountHex"], 16),
              "zstandardOracleVersion": zstd.zstd_version, "zstandardLevel": 1, "windowLog": 20,
              "roles": roles, "manifestBytes": member.size,
              "containerAndManifestBytes": archive_bytes - sum(role["storedBytes"] for role in roles),
              "numericJsonArraysOnlyBytes": numeric_json_bytes,
              "numericJsonLowerBoundToArchiveRatio": numeric_json_bytes / archive_bytes,
              "scope": "Independent per-block codec/read timings, exact JS JSON.stringify scalar sizes, and byte-exact production candidate comparison. Numeric JSON arrays alone are a lower bound on a complete JSON experiment. No Rust production timing or total process peak-memory claim."}
    with args.output.open("x", encoding="utf8", newline="\n") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    print(json.dumps({"output": str(args.output), "numericJsonLowerBoundToArchiveRatio": numeric_json_bytes / archive_bytes}), flush=True)


if __name__ == "__main__":
    main()
