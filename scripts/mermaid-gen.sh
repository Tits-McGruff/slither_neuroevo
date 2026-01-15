#!/bin/sh
set -eu

# Run from inside the scripts/ directory, but do not rely on the caller's CWD.
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

OUT_DIR="$REPO_ROOT/docs/mermaid"
MADGE_JSON="$OUT_DIR/madge.json"
MERMAID_MMD="$OUT_DIR/deps.mmd"
MERMAID_MD="$OUT_DIR/DEPENDENCIES.md"

# Ensure output folder exists
mkdir -p "$OUT_DIR"

# 1) Extract dependency map from both ./src and ./server into mermaid/madge.json
# If madge isn't installed yet, this will still work via npx if your npm setup allows it,
# but in most repos you'll add it as a devDependency.
cd "$REPO_ROOT"
npx madge \
  --extensions ts,tsx \
  --ts-config ./tsconfig.json \
  ./src ./server \
  --json > "$MADGE_JSON"

# 2) Convert Madge JSON -> Mermaid flowchart text into mermaid/deps.mmd
node "$SCRIPT_DIR/madge-to-mermaid.mts" "$MADGE_JSON" "$MERMAID_MMD" LR

# 3) Wrap Mermaid in Markdown so GitHub renders it into mermaid/DEPENDENCIES.md
{
  printf '%s\n' '```mermaid'
  cat "$MERMAID_MMD"
  printf '%s\n' '```'
} > "$MERMAID_MD"

printf '%s\n' "Wrote $MADGE_JSON"
printf '%s\n' "Wrote $MERMAID_MMD"
printf '%s\n' "Wrote $MERMAID_MD"
