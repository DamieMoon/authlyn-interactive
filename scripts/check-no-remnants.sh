#!/usr/bin/env bash
# Fail if any KalmarOS-derived names slipped into the repo.
# Excludes design docs (specs + plans), where naming the template is legitimate.

set -euo pipefail

PATTERN='kalmaros|KalmarOS|kalmaroS|ku-chronicles'
EXCLUDE_PATH='docs/superpowers/'

if git grep -in -E "$PATTERN" -- ":!$EXCLUDE_PATH" > /tmp/authlyn-remnants.$$ 2>/dev/null; then
    echo "KalmarOS-derived names leaked into the codebase:" >&2
    cat /tmp/authlyn-remnants.$$ >&2
    rm -f /tmp/authlyn-remnants.$$
    exit 1
fi

rm -f /tmp/authlyn-remnants.$$
echo "no-remnants check OK"
