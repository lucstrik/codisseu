#!/usr/bin/env python3
"""Render the audited Excalidraw SVG subset with system librsvg/Cairo.

No runtime renderer or download dependency. --check only verifies stored hashes.
The initial JSX extraction is documented in sources.json, with upstream revision
and hashes; SVG geometry remains identical to that licensed source.
"""
import argparse
import ctypes as C
import ctypes.util
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "reference/public/excalidraw-icons"
RUNTIME = ROOT / "crates/codisseu-ui/assets/excalidraw"
MANIFEST = SOURCE / "sources.json"


class Viewport(C.Structure):
    _fields_ = [(key, C.c_double) for key in ("x", "y", "width", "height")]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def render(manifest):
    rsvg = C.CDLL(ctypes.util.find_library("rsvg-2"))
    cairo = C.CDLL(ctypes.util.find_library("cairo"))
    gobject = C.CDLL(ctypes.util.find_library("gobject-2.0"))
    rsvg.rsvg_handle_new_from_file.argtypes = [C.c_char_p, C.c_void_p]
    rsvg.rsvg_handle_new_from_file.restype = C.c_void_p
    rsvg.rsvg_handle_render_document.argtypes = [C.c_void_p, C.c_void_p, C.POINTER(Viewport), C.c_void_p]
    rsvg.rsvg_handle_render_document.restype = C.c_int
    cairo.cairo_image_surface_create.argtypes = [C.c_int, C.c_int, C.c_int]
    cairo.cairo_image_surface_create.restype = C.c_void_p
    cairo.cairo_create.argtypes = [C.c_void_p]
    cairo.cairo_create.restype = C.c_void_p
    cairo.cairo_surface_write_to_png.argtypes = [C.c_void_p, C.c_char_p]
    cairo.cairo_surface_write_to_png.restype = C.c_int
    cairo.cairo_destroy.argtypes = [C.c_void_p]
    cairo.cairo_surface_destroy.argtypes = [C.c_void_p]
    gobject.g_object_unref.argtypes = [C.c_void_p]
    for icon in manifest["icons"]:
        svg = SOURCE / icon["svg"]
        if sha(svg) != icon["svg_sha256"]:
            raise ValueError(f"SVG hash changed: {svg}")
        width, height = icon["runtime_size"]
        handle = rsvg.rsvg_handle_new_from_file(str(svg).encode(), None)
        if not handle:
            raise ValueError(f"Invalid SVG: {svg}")
        surface = cairo.cairo_image_surface_create(0, width, height)
        context = cairo.cairo_create(surface)
        try:
            if not rsvg.rsvg_handle_render_document(handle, context, C.byref(Viewport(0, 0, width, height)), None):
                raise ValueError(f"SVG render failed: {svg}")
            png = ROOT / icon["runtime_png"]
            if cairo.cairo_surface_write_to_png(surface, str(png).encode()):
                raise ValueError(f"PNG write failed: {png}")
            icon["png_sha256"] = sha(png)
        finally:
            cairo.cairo_destroy(context)
            cairo.cairo_surface_destroy(surface)
            gobject.g_object_unref(handle)
    MANIFEST.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")


def check(manifest):
    for icon in manifest["icons"]:
        if sha(SOURCE / icon["svg"]) != icon["svg_sha256"]:
            raise ValueError(f"SVG hash mismatch: {icon['name']}")
        if sha(ROOT / icon["runtime_png"]) != icon["png_sha256"]:
            raise ValueError(f"PNG hash mismatch: {icon['name']}")
    for item in manifest["licenses"]:
        if sha(SOURCE / item["file"]) != item["sha256"]:
            raise ValueError(f"License hash mismatch: {item['file']}")
    print(f"Verified {len(manifest['icons'])} Excalidraw SVG/PNG assets and licenses")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--render", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    manifest = json.loads(MANIFEST.read_text())
    if args.render:
        render(manifest)
    check(manifest)
