#!/usr/bin/env bash
# Sync examples/config/flavors/ -> ~/dotfiles/env/.config/sbx/flavors/
#                              -> ~/.config/sbx/flavors/
#
# Source of truth: examples/config/flavors/ in this repo.
# Destinations get every file in source (creating missing flavor dirs).
# Destination-only flavors (e.g. tauri) are left alone.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SRC="$REPO_ROOT/examples/config/flavors"

DOTFILES_DEST="${SBX_DOTFILES_FLAVORS:-$HOME/dotfiles/env/.config/sbx/flavors}"
LIVE_DEST="${SBX_LIVE_FLAVORS:-$HOME/.config/sbx/flavors}"

DRY_RUN=0
case "${1:-}" in
    -n|--dry-run) DRY_RUN=1 ;;
    -h|--help)
        cat <<EOF
usage: $(basename "$0") [--dry-run|-n]

Copies every flavor under $SRC into:
  $DOTFILES_DEST
  $LIVE_DEST

Env overrides: SBX_DOTFILES_FLAVORS, SBX_LIVE_FLAVORS.
EOF
        exit 0
        ;;
esac

[[ -d "$SRC" ]] || { echo "source missing: $SRC" >&2; exit 1; }

sync_one() {
    local dest="$1"
    echo "==> $SRC -> $dest"
    [[ "$DRY_RUN" == "1" ]] || mkdir -p "$dest"
    local flavor target rel changed
    for flavor_dir in "$SRC"/*/; do
        flavor="$(basename "$flavor_dir")"
        target="$dest/$flavor"
        changed=0
        while IFS= read -r -d '' f; do
            rel="${f#$flavor_dir}"
            local out="$target/$rel"
            if [[ -f "$out" ]] && cmp -s "$f" "$out"; then
                continue
            fi
            changed=1
            if [[ "$DRY_RUN" == "1" ]]; then
                if [[ -f "$out" ]]; then
                    echo "  ~ $flavor/$rel"
                else
                    echo "  + $flavor/$rel"
                fi
            else
                mkdir -p "$(dirname "$out")"
                cp -p "$f" "$out"
                echo "  -> $flavor/$rel"
            fi
        done < <(find "$flavor_dir" -type f -print0)
        if [[ "$changed" == "0" ]]; then
            echo "  = $flavor (up to date)"
        fi
    done
}

sync_one "$DOTFILES_DEST"
sync_one "$LIVE_DEST"

if [[ "$DRY_RUN" == "1" ]]; then
    echo "(dry run — no files written)"
fi
