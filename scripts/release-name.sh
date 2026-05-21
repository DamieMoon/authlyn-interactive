#!/usr/bin/env bash
# Print a random release codename: <adjective>-<noun>.
# Use the output as the `codename` field under [package.metadata.release] in Cargo.toml.

set -euo pipefail

adjectives=(
    amber azure brisk crimson curious dapper dusky eager ember fabled
    feral frosty gentle gilded giggly glassy hidden hollow hushed jagged
    jaunty lively lucid mellow misty noble opal placid prismatic quiet
    quirky restless rugged saffron salty silken solemn spry sunken supple
    tawny tender thorny tidal twilight uncanny velvet wandering whispering
    woven wry zealous
)

nouns=(
    anchor archer beacon bramble brook canyon catalyst cinder cipher
    crescent dawn delta dervish drifter eddy ember falcon ferret fjord
    glade glyph harbor heron horizon ibis kestrel lantern lattice ledger
    loom mire orchard otter pier prism quartz raven rookery sable signal
    sigil sparrow steeple stoat tangent thicket thrush tide totem vault
    vector verge warden willow wraith
)

a=${adjectives[$RANDOM % ${#adjectives[@]}]}
n=${nouns[$RANDOM % ${#nouns[@]}]}
echo "${a}-${n}"
