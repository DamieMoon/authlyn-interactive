#!/usr/bin/env bash
# Launch a local SurrealDB instance for development.
# In-memory storage; data is discarded when the process exits.

set -euo pipefail

exec surreal start \
    --user root \
    --pass root \
    --bind 127.0.0.1:8000 \
    memory
