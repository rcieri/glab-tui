#!/usr/bin/env bash
# Fails when AGENTS.md is larger than MAX_BYTES, or more than SLACK_BYTES
# smaller, so MAX_BYTES follows the file down and freed space stays free.
set -euo pipefail

MAX_BYTES=143516
SLACK_BYTES=1024

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
size="$(wc -c <"$root/AGENTS.md" | tr -d ' ')"

if ((size > MAX_BYTES)); then
    cat >&2 <<EOF
AGENTS.md has $size bytes, more than its ceiling of $MAX_BYTES.
It holds only what applies everywhere. Put what the callers of one module
need into that module's //! doc, and reference material into docs/ with a
one-line pointer in AGENTS.md. Raising MAX_BYTES in
scripts/check-agents-md-size.sh needs a reason in the same PR.
EOF
    exit 1
fi

if ((size < MAX_BYTES - SLACK_BYTES)); then
    cat >&2 <<EOF
AGENTS.md has $size bytes, more than $SLACK_BYTES below its ceiling of $MAX_BYTES.
Lower MAX_BYTES in scripts/check-agents-md-size.sh to $size, so the space
freed is not filled again.
EOF
    exit 1
fi

echo "AGENTS.md: $size of $MAX_BYTES bytes"
