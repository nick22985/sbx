#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

cargo build --release

DEST="${HOME}/.local/bin/sbx"

if [ -f "$DEST" ] && ! file "$DEST" | grep -q 'ELF'; then
    echo "Replacing existing $DEST (was: $(file -b "$DEST"))"
fi

mkdir -p "$(dirname "$DEST")"
cp target/release/sbx "$DEST"

echo "Installed sbx to $DEST"
echo ""
echo "To enable completions, add this to your shell rc file:"
echo ""
echo "  # Bash (~/.bashrc)"
echo '  source <(COMPLETE=bash sbx)'
echo ""
echo "  # Zsh (~/.zshrc)"
echo '  source <(COMPLETE=zsh sbx)'
echo ""
echo "  # Fish (~/.config/fish/config.fish)"
echo '  COMPLETE=fish sbx | source'
echo ""
echo "Then restart your shell."
