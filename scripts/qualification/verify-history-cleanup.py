"""Verify rewritten source equivalence and reject reachable retired evidence objects."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def git(root, *arguments, data=None):
    """Read Git data without refreshing or altering repository indexes."""
    return subprocess.check_output(["git", *arguments], cwd=root, input=data,
                                   env=dict(os.environ, GIT_OPTIONAL_LOCKS="0"))


def source_inventory(root, commit):
    """Bind every non-performance path, mode and blob to the audited source tree."""
    records = []
    for line in git(root, "ls-tree", "-r", "-z", "--full-tree", commit).split(b"\0"):
        if line:
            _, path = line.split(b"\t", 1)
            if not path.startswith(b"docs/performance/"):
                records.append(line + b"\0")
    return hashlib.sha256(b"slither.history-source-equivalence.v1\0"
                          + b"".join(records)).hexdigest()


def commit_metadata(root, commit):
    """Verify original authorship, times, message and other preserved header fields."""
    raw = git(root, "cat-file", "commit", commit)
    header, message = raw.split(b"\n\n", 1)
    lines = [line for line in header.split(b"\n")
             if not line.startswith((b"tree ", b"parent "))]
    return hashlib.sha256(b"\n".join(lines) + b"\n\n" + message).hexdigest()


def verify(root, manifest, all_roots=False, refs=None):
    """Require all mapped sources and preserve the graph; missing objects are failures."""
    if manifest.get("kind") != "slither-feature-history-equivalence-v1":
        raise ValueError("unsupported history equivalence map")
    if len(set(manifest["removedEvidenceBlobs"])) != 2794:
        raise ValueError("complete 2794-object obsolete evidence inventory required")
    commits = manifest["commits"]
    if len(commits) != 147 or len(set(commits.values())) != 147:
        raise ValueError("all 147 distinct rewritten commits are required")
    for old, new in commits.items():
        detail = manifest["details"][old]
        if detail["rewritten"] != new:
            raise ValueError("inconsistent rewritten commit mapping")
        if source_inventory(root, new) != detail["nonPerformanceTreeSha256"]:
            raise ValueError("rewritten non-performance source differs: " + new)
        if commit_metadata(root, new) != detail["preservedCommitMetadataSha256"]:
            raise ValueError("rewritten commit metadata differs: " + new)
        expected = [commits.get(parent, parent) for parent in detail["originalParents"]]
        actual = git(root, "show", "-s", "--format=%P", new).decode().strip().split()
        if actual != expected:
            raise ValueError("rewritten parent graph differs: " + new)
    if all_roots:
        reachable = git(root, "rev-list", "--objects", "--no-object-names", "--all",
                        "--reflog", "--indexed-objects").decode().splitlines()
    else:
        roots = refs or git(root, "for-each-ref", "--format=%(objectname)",
                           "refs/heads", "refs/remotes", "refs/tags").decode().splitlines()
        reachable = git(root, "rev-list", "--objects", "--no-object-names", "--stdin",
                        data=("\n".join(roots) + "\n").encode()).decode().splitlines()
    obsolete = set(manifest["removedEvidenceBlobs"]) & set(reachable)
    if obsolete:
        raise ValueError(str(len(obsolete)) + " obsolete evidence blobs remain reachable; "
                         "review the selected refs, worktrees, indexes and reflogs")
    return {"rewrittenCommitsVerified": len(commits), "obsoleteReachableBlobs": 0,
            "roots": "all including worktrees/indexes/reflogs" if all_roots else "intended refs"}


def main():
    """Perform a read-only gate; never remove roots, expire logs or collect objects."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--map", type=Path,
                        default=Path("benchmarks/history-equivalence-20261025.json"))
    parser.add_argument("--all-roots", action="store_true")
    parser.add_argument("--ref", action="append")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    print(json.dumps(verify(root, json.loads(args.map.read_bytes()), args.all_roots, args.ref)))


if __name__ == "__main__":
    main()
