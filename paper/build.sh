#!/usr/bin/env bash
set -euo pipefail

paper_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
input="$paper_dir/paper.html"
output="${1:-$paper_dir/paper.pdf}"

required=(
  "$input"
  "$paper_dir/paper.css"
  "$paper_dir/assets/05_agent_architecture.svg"
  "$paper_dir/assets/06_mandala_editor.svg"
)

for file in "${required[@]}"; do
  if [[ ! -s "$file" ]]; then
    echo "rebis paper: required source is missing or empty: $file" >&2
    exit 1
  fi
done

mkdir -p "$(dirname "$output")"

if [[ -n "${WEASYPRINT:-}" ]]; then
  renderer="$WEASYPRINT"
elif command -v weasyprint >/dev/null 2>&1; then
  renderer="$(command -v weasyprint)"
elif [[ -x /home/facu/code/ra/.venv/bin/weasyprint ]]; then
  renderer=/home/facu/code/ra/.venv/bin/weasyprint
else
  renderer=""
fi

if [[ -n "$renderer" ]]; then
  "$renderer" \
    --base-url "$paper_dir" \
    --pdf-tags \
    --srgb \
    --optimize-images \
    "$input" "$output"
elif command -v chromium >/dev/null 2>&1; then
  chromium \
    --headless \
    --disable-gpu \
    --no-sandbox \
    --print-to-pdf="$output" \
    "file://$input"
elif command -v google-chrome >/dev/null 2>&1; then
  google-chrome \
    --headless \
    --disable-gpu \
    --no-sandbox \
    --print-to-pdf="$output" \
    "file://$input"
else
  echo "rebis paper: no HTML-to-PDF renderer found" >&2
  echo "set WEASYPRINT=/path/to/weasyprint or install WeasyPrint/Chromium" >&2
  exit 1
fi

if [[ ! -s "$output" ]]; then
  echo "rebis paper: renderer did not create a non-empty PDF: $output" >&2
  exit 1
fi

echo "rebis paper: wrote $output"
