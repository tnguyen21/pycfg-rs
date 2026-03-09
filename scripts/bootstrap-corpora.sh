#!/usr/bin/env bash
# Clone test corpora for smoke testing and benchmarks.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORPORA_DIR="$SCRIPT_DIR/../benchmark/corpora"

mkdir -p "$CORPORA_DIR"

clone_if_missing() {
    local name="$1"
    local url="$2"
    local dest="$CORPORA_DIR/$name"
    if [ -d "$dest" ]; then
        echo "  $name already present, skipping"
    else
        echo "  Cloning $name..."
        git clone --depth 1 "$url" "$dest"
    fi
}

echo "Bootstrapping test corpora into $CORPORA_DIR/"
clone_if_missing "requests" "https://github.com/psf/requests.git"
clone_if_missing "flask"    "https://github.com/pallets/flask.git"
clone_if_missing "rich"     "https://github.com/Textualize/rich.git"
echo "Done."
