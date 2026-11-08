"""Read the ordinary downloaded TAR independently, without extracting files."""
import hashlib
import json
import pathlib
import sys
import tarfile
import struct
from compression import zstd

path = pathlib.Path(sys.argv[1])
entries = []
manifest = None
with tarfile.open(path, mode="r:") as archive:
    for entry in archive:
        if not entry.isfile() or entry.name.startswith("/") or ".." in pathlib.PurePosixPath(entry.name).parts:
            raise ValueError("nonregular or unsafe archive entry")
        digest = hashlib.sha256()
        with archive.extractfile(entry) as source:
            while chunk := source.read(1024 * 1024):
                digest.update(chunk)
        entries.append({"name": entry.name, "bytes": entry.size, "storedSha256": digest.hexdigest()})
        if "manifest" in entry.name:
            if entry.size > 1024 * 1024:
                raise ValueError("unbounded JSON manifest")
            with archive.extractfile(entry) as source:
                candidate = json.load(source)
            if "checkpoint_logical_root_sha256" in candidate or "checkpointLogicalRootSha256" in candidate:
                manifest = candidate
if manifest is None:
    raise ValueError("save manifest missing")
root = hashlib.sha256(b"slither-neuroevo-save-root\0v1\0")
root.update(struct.pack("<I", len(manifest["roles"])))
validated = []
with tarfile.open(path, mode="r:") as archive:
    for role in manifest["roles"]:
        entry = archive.getmember(role["path"])
        if entry.size != int(role["storedBytesHex"], 16):
            raise ValueError("stored byte count mismatch")
        digest = hashlib.sha256()
        decoded_bytes = 0
        with archive.extractfile(entry) as source:
            if role["encoding"] == "f32le-shuffle4-zstd-v1":
                while header := source.read(12):
                    if len(header) != 12 or header[:4] != b"SFZ1":
                        raise ValueError("invalid shuffled block")
                    count, compressed = struct.unpack("<II", header[4:])
                    if count > 262144 or compressed > 2 * 1024 * 1024:
                        raise ValueError("unbounded shuffled block")
                    decoder = zstd.ZstdDecompressor(options={zstd.DecompressionParameter.window_log_max: 20})
                    shuffled = decoder.decompress(source.read(compressed), max_length=count * 4 + 1)
                    if not decoder.eof or decoder.unused_data or len(shuffled) != count * 4:
                        raise ValueError("decoded block size mismatch")
                    raw = bytearray(count * 4)
                    for lane in range(4):
                        raw[lane::4] = shuffled[lane * count:(lane + 1) * count]
                    digest.update(raw)
                    decoded_bytes += len(raw)
            else:
                while chunk := source.read(1024 * 1024):
                    digest.update(chunk)
                    decoded_bytes += len(chunk)
        if decoded_bytes != int(role["decodedBytesHex"], 16) or digest.hexdigest() != role["logicalSha256"]:
            raise ValueError("decoded count or logical hash mismatch")
        role_name = role["role"].encode()
        root.update(struct.pack("<H", len(role_name)))
        root.update(role_name)
        root.update(struct.pack("<Q", decoded_bytes))
        root.update(digest.digest())
        validated.append({"role": role["role"], "decodedBytes": decoded_bytes, "logicalSha256": digest.hexdigest()})
if root.hexdigest() != manifest["logicalRootSha256"]:
    raise ValueError("save root mismatch")
digest = hashlib.file_digest(path.open("rb"), "sha256").hexdigest()
print(json.dumps({"file": path.name, "archiveBytes": path.stat().st_size, "archiveSha256": digest,
                  "independentTarReader": "Python 3.14 tarfile, regular entries, bounded reads and per-entry stored hashes",
                  "entries": entries, "validatedRoles": validated, "saveRootVerified": True, "manifest": manifest}, indent=2))
