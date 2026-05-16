#!/usr/bin/env bash
# Render diagrams in this directory in both light and dark variants.
# Each <name>.d2 source has a `# PALETTE-START` / `# PALETTE-END` block
# holding the light palette. For dark, render.sh substitutes that block
# with the diagram-specific dark palette defined below, and renders with
# d2's dark theme.
#
# Requires `d2` on PATH (or set D2 to its full path).
set -euo pipefail

cd "$(dirname "$0")"
D2="${D2:-d2}"
LAYOUT="${D2_LAYOUT:-elk}"

# ────────── dark palettes (one per diagram) ──────────

dark_palette_architecture() {
cat <<'EOF'
vars: {
  netns-fill:        "#3a2410"
  netns-stroke:      "#fb923c"
  netns-text:        "#fed7aa"
  netns-accent-fill: "#5a3416"
  proxy-fill:        "#0f2547"
  proxy-stroke:      "#60a5fa"
  proxy-text:        "#bfdbfe"
  host-fill:         "#241a3a"
  host-stroke:       "#a78bfa"
  host-text:         "#ddd6fe"
  edge-proxy:        "#60a5fa"
  edge-proxy-text:   "#bfdbfe"
  edge-host:         "#a78bfa"
  edge-host-text:    "#ddd6fe"
}
EOF
}

dark_palette_exposure() {
cat <<'EOF'
vars: {
  client-fill:    "#1e293b"
  client-stroke:  "#94a3b8"
  client-text:    "#e2e8f0"
  cf-fill:        "#3a2410"
  cf-stroke:      "#fb923c"
  cf-text:        "#fed7aa"
  proxy-fill:     "#0f2547"
  proxy-stroke:   "#60a5fa"
  proxy-text:     "#bfdbfe"
  sandbox-fill:   "#0c3024"
  sandbox-stroke: "#34d399"
  sandbox-text:   "#a7f3d0"
  group-stroke:   "#64748b"
  group-text:     "#cbd5e1"
  arrow:          "#cbd5e1"
  arrow-tls:      "#60a5fa"
  arrow-tunnel:   "#fb923c"
}
EOF
}

# ────────── render loop ──────────

render() {
  local name="$1"
  local src="${name}.d2"
  local out_light="${name}-light.svg"
  local out_dark="${name}-dark.svg"

  "$D2" --layout="$LAYOUT" "$src" "$out_light"

  local dark_fn="dark_palette_${name}"
  if ! declare -F "$dark_fn" >/dev/null; then
    echo "warn: no dark palette for $name; using light palette in dark theme" >&2
    "$D2" --layout="$LAYOUT" --theme=200 "$src" "$out_dark"
  else
    local tmp
    tmp=$(mktemp --suffix=.d2)
    awk -v dark="$($dark_fn)" '
      /^# PALETTE-START$/ { print; print dark; in_p=1; next }
      /^# PALETTE-END$/   { in_p=0; print; next }
      !in_p               { print }
    ' "$src" > "$tmp"
    "$D2" --layout="$LAYOUT" --theme=200 "$tmp" "$out_dark"
    rm -f "$tmp"
  fi

  # Strip the d2-emitted background rect from the dark SVG so it composites
  # transparently on the GitHub dark canvas (or any other dark backdrop).
  sed -i.bak -E 's/<rect x="-?[0-9.]+" y="-?[0-9.]+" width="[0-9.]+" height="[0-9.]+" rx="[0-9.]+" fill="[^"]+" class=" fill-N7" stroke-width="0" \/>/<\!-- bg removed -->/' "$out_dark"
  rm -f "${out_dark}.bak"

  echo "  ${out_light}  +  ${out_dark}"
}

for src in *.d2; do
  render "${src%.d2}"
done
