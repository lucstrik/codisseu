#!/usr/bin/env python3
"""Exercise native drawing gestures, save/history and project restoration.

Run through private_native_run.py. Each run owns HOME, projects, profile and
executable. Input uses XTest against the verified process; no personal data,
network service, clipboard injection or language provider is used.
"""
import argparse
import ctypes as C
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import traceback

from PIL import ImageGrab

from native_controls import X11


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--scale', type=float, choices=[1., 1.25], default=1.)
    args = parser.parse_args()
    assert Path(os.environ['XAUTHORITY']).parent.name.startswith('storm-private-'), 'Use private_native_run.py'
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    source_binary = args.binary.resolve()
    digest = hashlib.sha256(source_binary.read_bytes()).hexdigest()
    base = Path(tempfile.mkdtemp(prefix='codisseu-native-drawings-'))
    base.chmod(0o700)
    robot = X11()
    app = None
    report = {
        'scenario': 'native Excalidraw gesture, save, undo/redo and project A/B/A restoration',
        'success': False, 'scale': args.scale, 'source_binary': str(source_binary),
        'binary_sha256': digest, 'checks': [], 'runs': [], 'captures': [],
        'transport': 'authenticated private X11; XTest input and WM_DELETE; owned PID verified',
        'limits': ['Linux X11/software Vulkan; no Wayland, Windows or macOS',
                   'Gesture adapter uses fixed logical canvas coordinates at 1280x800',
                   'Does not execute project applications, start providers or contact network services'],
    }

    def wait(predicate, seconds=20):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            assert app is not None and app.poll() is None, f'application exited: {None if app is None else app.returncode}'
            time.sleep(.03)
        raise TimeoutError('Native drawing condition did not become true')

    def scaled(value):
        return round(value * args.scale)

    def capture(name):
        assert robot.pid(robot.window) == app.pid
        # Settling is for a screenshot; assertions below use persisted state.
        time.sleep(.15)
        x, y, width, height = robot.geometry()
        path = out / (name + '.png')
        ImageGrab.grab(xdisplay=os.environ['DISPLAY']).crop((x, y, x + width, y + height)).save(path)
        report['captures'].append({'path': path.name, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                                   'size_px': [width, height], 'source': 'real X11 window pixels'})

    def close_window(name):
        nonlocal app

        class Message(C.Structure):
            _fields_ = [('type', C.c_int), ('serial', C.c_ulong), ('send_event', C.c_int),
                        ('display', C.c_void_p), ('window', C.c_ulong), ('message_type', C.c_ulong),
                        ('format', C.c_int), ('data', C.c_long * 5)]

        class Event(C.Union):
            _fields_ = [('message', Message), ('pad', C.c_long * 24)]

        assert robot.pid(robot.window) == app.pid
        event = Event()
        event.message = Message(33, 0, 1, robot.display, robot.window,
                                robot.x.XInternAtom(robot.display, b'WM_PROTOCOLS', 0), 32,
                                (C.c_long * 5)(robot.x.XInternAtom(robot.display, b'WM_DELETE_WINDOW', 0), 0, 0, 0, 0))
        robot.x.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.POINTER(Event)]
        robot.x.XSendEvent(robot.display, robot.window, 0, 0, C.byref(event))
        robot.x.XFlush(robot.display)
        app.wait(timeout=20)
        assert app.returncode == 0, f'{name}: exit {app.returncode}'
        report['runs'][-1]['exit_code'] = app.returncode
        report['checks'].append(name + ': clean WM_DELETE and session persistence')
        app = None

    def visible_count(scene_path):
        try:
            scene = json.loads(scene_path.read_text())
            return len([element for element in scene['elements'] if not element.get('isDeleted', False)])
        except (OSError, ValueError, KeyError):
            return None

    try:
        binary = base / 'codisseu-under-test'
        # Freeze the complete corpus against concurrent Cargo rebuilds.
        shutil.copy2(source_binary, binary)
        home = base / 'home'
        a, b = base / 'Project A', base / 'Project B'
        for directory in [home, a, b]:
            directory.mkdir()
        drawing = a / 'architecture.excalidraw'
        seed = {'type': 'excalidraw', 'version': 2, 'source': 'codisseu-native-fixture',
                'elements': [], 'appState': {'viewBackgroundColor': '#ffffff'}, 'files': {},
                'nativeFixture': {'preserve': 'owned synthetic field'}}
        drawing.write_text(json.dumps(seed) + '\n')
        profile = home / 'state/profile.json'
        env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home / 'config'),
                   XDG_DATA_HOME=str(home / 'data'), XDG_CACHE_HOME=str(home / 'cache'),
                   XDG_STATE_HOME=str(home / 'state'), WINIT_UNIX_BACKEND='x11',
                   WINIT_X11_SCALE_FACTOR=str(args.scale), LANG='C.UTF-8')
        for key in ['CODISSEU_RESOURCE_DIR', 'CODISSEU_PROVIDER_CONFIG', 'AT_SPI_BUS_ADDRESS']:
            env.pop(key, None)

        def launch(name, project, open_file=None):
            nonlocal app
            session = home / ('state/' + name + '.json')
            ready = out / (name + '-ready.png')
            command = [str(binary), '--no-restore', '--no-auto-providers', '--workspace', str(project),
                       '--session', str(session), '--profile', str(profile), '--capture', str(ready)]
            if open_file is not None:
                command.extend(['--open', str(open_file)])
            with (out / (name + '.log')).open('w') as log:
                app = subprocess.Popen(command, cwd=project, env=env, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            report['runs'].append({'name': name, 'pid': app.pid, 'command': command, 'exit_code': None})
            wait(lambda: ready.with_suffix('.json').is_file(), 90)
            state = json.loads(ready.with_suffix('.json').read_text())
            observed = state['observed']
            assert state['pixels_per_point'] == args.scale, state['pixels_per_point']
            assert observed['providers'] == [], observed['providers']
            assert not observed['provider_startup']['needs_trust'], observed['provider_startup']
            assert observed['provider_startup']['error'] is None, observed['provider_startup']
            paths = [document['path'] for document in observed['documents']]
            report['runs'][-1]['observed_document_paths'] = paths
            assert observed['workspace'] == str(project), observed['workspace']
            wait(lambda: robot.find(app.pid, (scaled(1280), scaled(800))))
            assert hashlib.sha256(Path(f'/proc/{app.pid}/exe').read_bytes()).hexdigest() == digest
            robot.focus()
            return session, paths

        session, paths = launch('draw-and-save', a, drawing)
        assert paths == [str(drawing)], paths
        robot.click(scaled(550), scaled(450))
        robot.key('r')
        time.sleep(.12)
        x, y, width, height = robot.geometry()
        assert robot.pid(robot.window) == app.pid and robot.focused()
        start, end = (450, 330), (650, 450)
        for point in [start, end]:
            assert 0 <= scaled(point[0]) < width and 0 <= scaled(point[1]) < height
        robot.t.XTestFakeMotionEvent(robot.display, -1, x + scaled(start[0]), y + scaled(start[1]), 0)
        robot.t.XTestFakeButtonEvent(robot.display, 1, 1, 0)
        robot.x.XFlush(robot.display)
        time.sleep(.08)
        for step in range(1, 13):
            px = start[0] + (end[0] - start[0]) * step / 12
            py = start[1] + (end[1] - start[1]) * step / 12
            robot.t.XTestFakeMotionEvent(robot.display, -1, x + scaled(px), y + scaled(py), 0)
            robot.x.XFlush(robot.display)
            time.sleep(.025)
        robot.t.XTestFakeButtonEvent(robot.display, 1, 0, 0)
        robot.x.XFlush(robot.display)
        time.sleep(.15)
        robot.key('s', ('Control_L',))
        wait(lambda: visible_count(drawing) == 1)
        saved = json.loads(drawing.read_text())
        element = next(element for element in saved['elements'] if not element.get('isDeleted', False))
        assert element['type'] == 'rectangle', element
        assert all(math.isfinite(element[key]) for key in ['x', 'y', 'width', 'height']), element
        assert abs(element['width'] - 200) < 2 and abs(element['height'] - 120) < 2, element
        assert saved['nativeFixture'] == seed['nativeFixture']
        assert saved['files'] == seed['files']
        report['rectangle'] = {key: element[key] for key in ['id', 'type', 'x', 'y', 'width', 'height']}
        report['checks'].extend(['XTest R + primary drag creates rectangle with measured dimensions',
                                 'Ctrl+S persists valid Excalidraw scene and unknown source fields'])
        capture('rectangle-saved')
        robot.key('z', ('Control_L',))
        time.sleep(.1)
        robot.key('s', ('Control_L',))
        wait(lambda: visible_count(drawing) == 0)
        report['checks'].append('Ctrl+Z and Ctrl+S persist an empty scene')
        capture('rectangle-undone')
        robot.key('z', ('Control_L', 'Shift_L'))
        time.sleep(.1)
        robot.key('s', ('Control_L',))
        wait(lambda: visible_count(drawing) == 1)
        assert json.loads(drawing.read_text())['elements'] == saved['elements']
        report['checks'].append('Ctrl+Shift+Z and Ctrl+S restore exact element data')
        capture('rectangle-redone')
        # Closing the tab must not remove the project link.
        robot.key('w', ('Control_L',))
        time.sleep(.15)
        close_window('draw-and-save')
        persisted = json.loads(session.read_text())
        assert str(drawing) not in persisted['tabs'], persisted['tabs']
        catalog_files = list(profile.with_suffix('.drawings').glob('*.json'))
        assert len(catalog_files) == 1, catalog_files
        catalog = json.loads(catalog_files[0].read_text())
        assert catalog == {'version': 1, 'project': str(a), 'paths': [str(drawing)]}, catalog
        report['checks'].append('Closing drawing tab preserves its project catalog entry')
        for name, project, expected in [('restore-a', a, [str(drawing)]),
                                         ('open-b', b, []), ('return-a', a, [str(drawing)])]:
            session, paths = launch(name, project)
            assert paths == expected, (name, paths, expected)
            capture(name + '-workspace')
            close_window(name)
            persisted = json.loads(session.read_text())
            assert persisted['tabs'] == expected, (name, persisted['tabs'], expected)
            report['checks'].append(name + ': correct drawings restored from shared project profile without --open')
        assert json.loads(drawing.read_text())['elements'] == saved['elements']
        report['success'] = True
    except Exception as error:
        report['failure'] = {'kind': type(error).__name__, 'message': str(error),
                             'traceback': traceback.format_exc()}
        if app is not None and app.poll() is None and robot.window:
            try:
                capture('failure')
            except Exception:
                pass
        raise
    finally:
        if app is not None and app.poll() is None:
            os.killpg(app.pid, signal.SIGTERM)
            try:
                app.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(app.pid, signal.SIGKILL)
                app.wait(timeout=5)
        robot.x.XCloseDisplay(robot.display)
        shutil.rmtree(base)
        report['private_profile_removed'] = not base.exists()
        (out / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(f'Native drawing checks passed: {len(report["checks"])}', flush=True)


if __name__ == '__main__':
    main()
