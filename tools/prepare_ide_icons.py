#!/usr/bin/env python3
"""Verify or rerender audited local SVGs using librsvg (no runtime or network dependency)."""
import argparse
import ctypes as C
import ctypes.util
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCES = ROOT / 'reference/public/ide-icons'
OUTPUT = ROOT / 'crates/codisseu-ui/assets/ide-icons'
MANIFEST = SOURCES / 'sources.json'

class Viewport(C.Structure):
    _fields_ = [(key, C.c_double) for key in ('x', 'y', 'width', 'height')]

def render_all(manifest):
    rsvg = C.CDLL(ctypes.util.find_library('rsvg-2'))
    cairo = C.CDLL(ctypes.util.find_library('cairo'))
    gobject = C.CDLL(ctypes.util.find_library('gobject-2.0'))
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
    for icon in manifest['icons']:
        svg = SOURCES / (icon['name'] + '.svg')
        if hashlib.sha256(svg.read_bytes()).hexdigest() != icon['svg_sha256']:
            raise ValueError(f'Source hash changed: {svg}')
        handle = rsvg.rsvg_handle_new_from_file(str(svg).encode(), None)
        if not handle:
            raise ValueError(f'Invalid SVG: {svg}')
        surface = cairo.cairo_image_surface_create(0, 64, 64)
        context = cairo.cairo_create(surface)
        try:
            if not rsvg.rsvg_handle_render_document(handle, context, C.byref(Viewport(0, 0, 64, 64)), None):
                raise ValueError(f'Cannot render: {svg}')
            png = OUTPUT / (icon['name'] + '.png')
            if cairo.cairo_surface_write_to_png(surface, str(png).encode()):
                raise ValueError(f'Cannot save: {png}')
            icon['png_sha256'] = hashlib.sha256(png.read_bytes()).hexdigest()
            if icon.get('upstream_sha256'):
                icon['modification'] = 'currentColor replaced with #c9c9ce; geometry unchanged; rendered to transparent64x64 PNG using librsvg/Cairo'
            elif icon.get('source_status') == 'user-provided reference / Codisseu vector redrawing':
                icon['modification'] = 'Vector redrawing of user attachment, white background omitted, neutral #c9c9ce; rendered to transparent64x64 PNG'
            elif icon.get('source_status') != 'designed':
                icon['modification'] = 'SVG unchanged; rendered to transparent 64x64 PNG using librsvg/Cairo'
        finally:
            cairo.cairo_destroy(context)
            cairo.cairo_surface_destroy(surface)
            gobject.g_object_unref(handle)
    manifest['rasterizer'] = 'librsvg 2.60.0 / Cairo 1.18.4 (system shared libraries, build-time only)'
    MANIFEST.write_text(json.dumps(manifest, indent=2) + '\n')

def check(manifest):
    expected = set()
    for icon in manifest['icons']:
        expected.add(icon['name'])
        for directory, suffix, key in [(SOURCES, '.svg', 'svg_sha256'), (OUTPUT, '.png', 'png_sha256')]:
            path = directory / (icon['name'] + suffix)
            if hashlib.sha256(path.read_bytes()).hexdigest() != icon[key]:
                raise ValueError(f'Hash mismatch: {path}')
    if {p.stem for p in OUTPUT.glob('*.png')} != expected:
        raise ValueError('Runtime asset files differ from audited manifest')
    print(f'{len(expected)} SVG sources and PNG runtime assets verified')

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--render', action='store_true', help='Rerender verified local sources, updating PNG hashes')
    parser.add_argument('--check', action='store_true', help='Verify source and runtime hashes (default)')
    args = parser.parse_args()
    manifest = json.loads(MANIFEST.read_text())
    if args.render:
        render_all(manifest)
    check(manifest)
