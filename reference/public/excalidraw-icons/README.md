# Excalidraw toolbar artwork

Public source: Excalidraw revision
`2c3dcc06ec807e81ab9437eceb1a8f7a3c590743`, collected on 20/11/2026.
`sources.json` records source URLs, hashes, selected component names and every
SVG/PNG derivative. `upstream-jsx.json` preserves only the selected icon source
snippets, without importing the unrelated Font Awesome icons from that file.

The selected artwork is original Excalidraw or Tabler-derived artwork under MIT.
`LICENSE` preserves Excalidraw's notice; `TABLER-LICENSE` preserves Paweł Kuna's
notice, with its own source revision recorded in the manifest. The save action
uses Excalidraw's MIT `downloadIcon`, rather than its unrelated Font Awesome
floppy-disk icon.

Geometry, viewBoxes, masks, joins and per-path stroke widths are retained.
JSX attribute names were converted to SVG, `currentColor` became white for
alpha-only native tint, and back-layer rotation became an SVG transform. The
wordmark and logo paths come from `ExcalidrawLogo.tsx`; the composed logo uses
the official small-layout proportions. They identify the requested drawing
tool and do not imply endorsement or incorporation of the React application.

Runtime artwork lives in `crates/codisseu-ui/assets/excalidraw/toolbar-*.png`.
Ordinary icons are transparent 64×64 PNGs; wordmark assets preserve their wide
aspect ratio. Rust paints them using a bounded texture cache and native theme
colors. No network, JavaScript, CSS or SVG runtime renderer is involved.

Re-render the stored SVG files with system librsvg/Cairo:

```sh
python3 tools/prepare_excalidraw_icons.py --render
python3 tools/prepare_excalidraw_icons.py --check
```
