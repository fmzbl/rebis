# Rebis language paper

This directory is the editable, reproducible source for the Rebis paper.
Everything is local and code-native: semantic HTML, print CSS, and SVG figures.
No rasterized whiteboard image or remote web asset is required to build it.

## Build

From the repository root:

```bash
./paper/build.sh
```

The default output is `paper/paper.pdf`. To write elsewhere:

```bash
./paper/build.sh /tmp/rebis-paper.pdf
```

The script prefers `WEASYPRINT` when set, then a `weasyprint` on `PATH`, then
the local research-environment renderer at
`/home/facu/code/ra/.venv/bin/weasyprint`. Chromium and Google Chrome are
fallbacks. The known local renderer is WeasyPrint 66.0.

To force a renderer:

```bash
WEASYPRINT=/path/to/weasyprint ./paper/build.sh
```

## Sources

- `paper.html` — semantic paper source and formal specification narrative.
- `paper.css` — screen and A4 print styles, running headings, and page layout.
- `assets/05_agent_architecture.svg` — scouts, mediator, and arrow judge.
- `assets/06_mandala_editor.svg` — integrated editor and mandala projection.
- `paper.pdf` — derived print artifact; regenerate after source changes.

## Claim discipline

The paper distinguishes software guarantees from planned measurements. It does
not invent benchmark results. Worked set calculations are derivations from the
listed finite fixtures; any host transcript that cannot yet be produced by the
reference binary is explicitly labeled illustrative.

The normative names are Rebis, `rebis_lang`, and the `rebis` command. “Mirror”
appears only for the compatible KAOS command and myth gate spelling.

## Validation

The build script checks that every source asset exists and that the renderer
produces a non-empty PDF. Repository checks should additionally include:

```bash
cargo test --all-features
python3 - <<'PY'
import xml.etree.ElementTree as ET
from pathlib import Path
for svg in sorted(Path("paper/assets").glob("*.svg")):
    ET.parse(svg)
    print("valid XML:", svg)
PY
```

HTML and CSS are the source of truth even when renderer metadata makes two PDF
files differ byte-for-byte.
