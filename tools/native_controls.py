#!/usr/bin/env python3
"""Real X11 input/captures against a newly launched Storm and disposable files only."""
import argparse
import ctypes as C
import ctypes.util
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def wait_for(predicate, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    raise TimeoutError("Native condition did not become true")


class X11:
    def __init__(self):
        self.x = C.CDLL(ctypes.util.find_library('X11'))
        self.t = C.CDLL(ctypes.util.find_library('Xtst'))
        ptr, window, uint, ulong = C.c_void_p, C.c_ulong, C.c_uint, C.c_ulong
        declarations = {
            'XOpenDisplay': ([C.c_char_p], ptr),
            'XDefaultRootWindow': ([ptr], window),
            'XInternAtom': ([ptr, C.c_char_p, C.c_int], ulong),
            'XQueryTree': ([ptr, window, C.POINTER(window), C.POINTER(window), C.POINTER(C.POINTER(window)), C.POINTER(uint)], C.c_int),
            'XGetWindowProperty': ([ptr, window, ulong, C.c_long, C.c_long, C.c_int, ulong, C.POINTER(ulong), C.POINTER(C.c_int), C.POINTER(ulong), C.POINTER(ulong), C.POINTER(C.POINTER(C.c_ubyte))], C.c_int),
            'XGetGeometry': ([ptr, window, C.POINTER(window), C.POINTER(C.c_int), C.POINTER(C.c_int), C.POINTER(uint), C.POINTER(uint), C.POINTER(uint), C.POINTER(uint)], C.c_int),
            'XTranslateCoordinates': ([ptr, window, window, C.c_int, C.c_int, C.POINTER(C.c_int), C.POINTER(C.c_int), C.POINTER(window)], C.c_int),
            'XGetInputFocus': ([ptr, C.POINTER(window), C.POINTER(C.c_int)], C.c_int),
            'XSetInputFocus': ([ptr, window, C.c_int, ulong], C.c_int),
            'XRaiseWindow': ([ptr, window], C.c_int),
            'XKeysymToKeycode': ([ptr, ulong], C.c_ubyte),
            'XStringToKeysym': ([C.c_char_p], ulong),
            'XFlush': ([ptr], C.c_int),
            'XFree': ([ptr], C.c_int),
            'XCloseDisplay': ([ptr], C.c_int),
        }
        for name, (args, result) in declarations.items():
            fn = getattr(self.x, name); fn.argtypes = args; fn.restype = result
        self.t.XTestFakeKeyEvent.argtypes = [ptr, uint, C.c_int, ulong]
        self.t.XTestFakeButtonEvent.argtypes = [ptr, uint, C.c_int, ulong]
        self.t.XTestFakeMotionEvent.argtypes = [ptr, C.c_int, C.c_int, C.c_int, ulong]
        self.display = self.x.XOpenDisplay(None)
        assert self.display, 'X11 display unavailable'
        self.root = self.x.XDefaultRootWindow(self.display)
        self.pid_atom = self.x.XInternAtom(self.display, b'_NET_WM_PID', 0)
        self.window = None
        self.target_pid = None

    def children(self, window):
        root, parent, count = C.c_ulong(), C.c_ulong(), C.c_uint()
        children = C.POINTER(C.c_ulong)()
        if not self.x.XQueryTree(self.display, window, C.byref(root), C.byref(parent), C.byref(children), C.byref(count)):
            return []
        result = list(children[:count.value])
        if children: self.x.XFree(children)
        return result

    def pid(self, window):
        kind, count, after, fmt = C.c_ulong(), C.c_ulong(), C.c_ulong(), C.c_int()
        data = C.POINTER(C.c_ubyte)()
        self.x.XGetWindowProperty(self.display, window, self.pid_atom, 0, 1, 0, 0,
                                 C.byref(kind), C.byref(fmt), C.byref(count), C.byref(after), C.byref(data))
        result = C.cast(data, C.POINTER(C.c_ulong))[0] if data and count.value and fmt.value == 32 else None
        if data: self.x.XFree(data)
        return result

    def find(self, pid, size=(1280, 800)):
        pending = [self.root]
        while pending:
            window = pending.pop()
            if self.pid(window) == pid and self.geometry(window)[2:] == size:
                self.window = window
                self.target_pid = pid
                return window
            pending.extend(self.children(window))
        return None

    def geometry(self, window=None):
        window = window or self.window
        root, child = C.c_ulong(), C.c_ulong()
        x, y, sx, sy = C.c_int(), C.c_int(), C.c_int(), C.c_int()
        width, height, border, depth = C.c_uint(), C.c_uint(), C.c_uint(), C.c_uint()
        assert self.x.XGetGeometry(self.display, window, C.byref(root), C.byref(x), C.byref(y), C.byref(width), C.byref(height), C.byref(border), C.byref(depth))
        self.x.XTranslateCoordinates(self.display, window, self.root, 0, 0, C.byref(sx), C.byref(sy), C.byref(child))
        return sx.value, sy.value, width.value, height.value

    def focus(self):
        self.x.XRaiseWindow(self.display, self.window)
        self.x.XSetInputFocus(self.display, self.window, 1, 0)
        self.x.XFlush(self.display)
        wait_for(self.focused)

    def focused(self):
        focused, revert = C.c_ulong(), C.c_int()
        self.x.XGetInputFocus(self.display, C.byref(focused), C.byref(revert))
        return focused.value == self.window and self.pid(self.window) == self.target_pid

    def key(self, name, modifiers=()):
        assert self.focused(), 'Focus moved away from our Storm window; input aborted'
        codes = [self.x.XKeysymToKeycode(self.display, self.x.XStringToKeysym(m.encode())) for m in modifiers]
        symbol = ord(name) if len(name) == 1 else self.x.XStringToKeysym(name.encode())
        code = self.x.XKeysymToKeycode(self.display, symbol)
        assert code and all(codes), f'Key unavailable: {name}'
        for keycode in codes: self.t.XTestFakeKeyEvent(self.display, keycode, 1, 0)
        self.x.XFlush(self.display)
        time.sleep(0.025)
        self.t.XTestFakeKeyEvent(self.display, code, 1, 0)
        self.x.XFlush(self.display)
        time.sleep(0.025)
        self.t.XTestFakeKeyEvent(self.display, code, 0, 0)
        for keycode in reversed(codes): self.t.XTestFakeKeyEvent(self.display, keycode, 0, 0)
        self.x.XFlush(self.display)

    def text(self, value):
        assert value.isascii(), 'Only deliberate ASCII keystrokes; no clipboard rewriting'
        for char in value:
            self.key('Return' if char == '\n' else char, ('Shift_L',) if char.isupper() else ())

    def click(self, x, y):
        assert self.focused(), 'Focus moved away from our Storm window; click aborted'
        sx, sy, width, height = self.geometry()
        assert 0 <= x < width and 0 <= y < height
        self.t.XTestFakeMotionEvent(self.display, -1, sx + x, sy + y, 0)
        self.x.XFlush(self.display)
        time.sleep(0.025)
        self.t.XTestFakeButtonEvent(self.display, 1, 1, 0)
        self.x.XFlush(self.display)
        time.sleep(0.035)
        self.t.XTestFakeButtonEvent(self.display, 1, 0, 0)
        self.x.XFlush(self.display)


def run():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(); out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    workspace = out / 'workspace'; workspace.mkdir()
    a, b = workspace / 'initial.php', workspace / 'opened.ts'
    a.write_text("<?php echo 'ação 🌧';\n")
    b.write_text("export const chuva = '🌧';\n")
    original = {str(p): p.read_bytes() for p in [a, b]}
    session, ready = out / 'session.json', out / 'native-ready.png'
    binary = ROOT / 'target/debug/codisseu'
    command = [str(binary), '--no-restore', '--workspace', str(workspace), '--open', str(a), '--session', str(session), '--capture', str(ready)]
    env = os.environ.copy(); env.pop('WAYLAND_DISPLAY', None); env['WINIT_UNIX_BACKEND'] = 'x11'
    robot = X11(); records = []
    def capture(name):
        path = out / f'{name}.png'
        # Input settling is not a state assertion; disk/session assertions below prove effects.
        time.sleep(0.25)
        subprocess.run(['magick', 'import', '-window', str(robot.window), str(path)], check=True, timeout=15)
        data = path.read_bytes(); size = struct.unpack('>II', data[16:24])
        assert size == (1280, 800)
        records.append({'name': name, 'sha256': hashlib.sha256(data).hexdigest(), 'size_px': size})
    with (out / 'storm.log').open('w') as log:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
        try:
            binary_sha256 = hashlib.sha256(Path(f'/proc/{process.pid}/exe').read_bytes()).hexdigest()
            wait_for(lambda: ready.with_suffix('.json').exists(), 30)
            wait_for(lambda: robot.find(process.pid)); robot.focus()
            capture('shell')
            robot.click(100, 20); capture('menu')
            robot.key('Escape'); time.sleep(0.15)
            robot.key('o', ('Control_L',)); capture('open-dialog')
            robot.key('a', ('Control_L',)); time.sleep(0.12)
            robot.text(str(b)); capture('path-filled')
            robot.key('Return'); capture('opened-file')
            time.sleep(0.3)
            robot.key('End', ('Control_L',)); time.sleep(0.12)
            robot.text('// saved by real x11 input\n'); capture('edited-file')
            robot.key('s', ('Control_L',))
            expected = original[str(b)] + b'// saved by real x11 input\n'
            wait_for(lambda: b.read_bytes() == expected)
            assert a.read_bytes() == original[str(a)]
            capture('saved-file')
            robot.click(100, 20); time.sleep(0.15)
            robot.click(100, 160)
            process.wait(timeout=15)
            assert process.returncode == 0
            persisted = json.loads(session.read_text())
            assert persisted['tabs'] == [str(a), str(b)]
            assert persisted['active'] == str(b)
            manifest = {'kind': 'native XWayland window; OS input through XTest; own PID verified before input',
                        'binary_sha256': binary_sha256, 'binary_hash_source': '/proc/<own-pid>/exe immediately after launch', 'command': command,
                        'pid': process.pid, 'exit_code': process.returncode, 'geometry': [1280, 800],
                        'checks': ['open via Ctrl+O/Enter', 'type via XTest', 'save via Ctrl+S and exact disk bytes',
                                   'other file preserved', 'normal File menu Exit via XTest clicks', 'session persisted'],
                        'screens': records, 'session': persisted}
            (out / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
            print(f'Native keyboard, disk and session verified: {out}', flush=True)
        except Exception as error:
            (out / 'failure.json').write_text(json.dumps({'error_type': type(error).__name__, 'error': str(error), 'screens': records}, indent=2) + '\n')
            raise
        finally:
            if process.poll() is None:
                process.terminate()
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired: process.kill(); process.wait()
            robot.x.XCloseDisplay(robot.display)


if __name__ == '__main__':
    run()
