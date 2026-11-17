#!/usr/bin/env python3
"""Verify the core package with/without host tools on an owned X11 display.

Run through private_native_run.py. Uses disposable HOME/projects and real XTest
typing; never opens a normal profile or executes a project application.
"""
import argparse
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time

from PIL import ImageGrab

from native_controls import X11


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--resources', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--scale', type=float, choices=[1., 1.25], default=1.)
    args = parser.parse_args()
    assert Path(os.environ['XAUTHORITY']).parent.name.startswith('storm-private-')
    resources = args.resources.resolve()
    manifest = json.loads((resources/'runtime.json').read_text())
    assert manifest == {'schema_version': 1, 'tools': {}, 'providers': []}, manifest
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    source_binary = args.binary.resolve()
    digest = hashlib.sha256(source_binary.read_bytes()).hexdigest()
    report = {'scenario': 'public core native editor with and without host tools',
              'binary_sha256': digest, 'source_binary': str(source_binary),
              'scale': args.scale, 'success': False, 'checks': [], 'runs': [],
              'limits': ['Linux X11/software Vulkan; no Wayland or other OS',
                         'Does not execute user applications or install tools']}
    base = Path(tempfile.mkdtemp(prefix='codisseu-core-release-'))
    base.chmod(0o700)
    robot = X11()
    app = None

    def close_window():
        class Message(C.Structure):
            _fields_ = [('type', C.c_int), ('serial', C.c_ulong), ('send_event', C.c_int),
                        ('display', C.c_void_p), ('window', C.c_ulong), ('message_type', C.c_ulong),
                        ('format', C.c_int), ('data', C.c_long*5)]

        class Event(C.Union):
            _fields_ = [('message', Message), ('pad', C.c_long*24)]

        assert robot.pid(robot.window) == app.pid
        event = Event()
        event.message = Message(33, 0, 1, robot.display, robot.window,
                                robot.x.XInternAtom(robot.display, b'WM_PROTOCOLS', 0),
                                32, (C.c_long*5)(robot.x.XInternAtom(robot.display, b'WM_DELETE_WINDOW', 0), 0, 0, 0, 0))
        robot.x.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.POINTER(Event)]
        robot.x.XSendEvent(robot.display, robot.window, 0, 0, C.byref(event))
        robot.x.XFlush(robot.display)
    try:
        # Freeze the tested executable against concurrent rebuilds.
        binary = base/'codisseu'
        shutil.copy2(source_binary, binary)
        host_path = os.environ.get('PATH', '/usr/bin:/bin')
        empty_bin = base/'empty-bin'
        empty_bin.mkdir()
        for name, search_path in [('missing-tools', str(empty_bin)), ('host-tools', host_path)]:
            home = base/name
            project = home/'project'
            project.mkdir(parents=True)
            source = project/'index.php'
            original = b"<?php\n// disposable core release fixture\n"
            source.write_bytes(original)
            session = home/'state/session.json'
            ready = out/(name+'-ready.png')
            env = dict(os.environ, HOME=str(home), PATH=search_path,
                       XDG_CONFIG_HOME=str(home/'config'), XDG_DATA_HOME=str(home/'data'),
                       XDG_CACHE_HOME=str(home/'cache'), XDG_STATE_HOME=str(home/'state'),
                       CODISSEU_RESOURCE_DIR=str(resources), WINIT_UNIX_BACKEND='x11',
                       WINIT_X11_SCALE_FACTOR=str(args.scale), LANG='C.UTF-8')
            for key in ['CARGO_HOME', 'RUSTUP_HOME', 'RUSTUP_TOOLCHAIN']:
                env.pop(key, None)
            available = {tool: shutil.which(tool, path=search_path)
                         for tool in ['php', 'node', 'python3', 'rustc', 'git']}
            if name == 'missing-tools':
                assert not any(available.values()), available
            runtime = subprocess.run([str(binary), '--runtime-info'], env=env,
                                     capture_output=True, text=True, check=True, timeout=10)
            runtime_info = json.loads(runtime.stdout)
            assert runtime_info['tools'] == {} and runtime_info['providers'] == []
            command = [str(binary), '--no-restore', '--workspace', str(project),
                       '--open', str(source), '--session', str(session), '--capture', str(ready)]
            with (out/(name+'.log')).open('w') as log:
                app = subprocess.Popen(command, cwd=project, env=env, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)

                def wait(condition, seconds=20):
                    deadline = time.monotonic()+seconds
                    while time.monotonic() < deadline:
                        if condition():
                            return
                        assert app.poll() is None, f'{name}: application exited {app.returncode}'
                        time.sleep(.03)
                    raise TimeoutError(f'{name}: native condition did not finish')

                wait(lambda: ready.with_suffix('.json').is_file(), 90)
                state = json.loads(ready.with_suffix('.json').read_text())
                observed = state['observed']
                assert observed['providers'] == [], observed['providers']
                assert not observed['provider_startup']['needs_trust'], observed['provider_startup']
                assert observed['provider_startup']['error'] is None, observed['provider_startup']
                assert state['pixels_per_point'] == args.scale
                wait(lambda: robot.find(app.pid, (round(1280*args.scale), round(800*args.scale))))
                assert hashlib.sha256(Path(f'/proc/{app.pid}/exe').read_bytes()).hexdigest() == digest
                robot.focus()
                # Focus the known editor region, then use semantic keyboard navigation.
                robot.click(round(500*args.scale), round(190*args.scale))
                robot.key('End', ('Control_L',))
                robot.text('// verified\n')
                robot.key('s', ('Control_L',))
                expected = original+b'// verified\n'
                wait(lambda: source.read_bytes() == expected)
                time.sleep(.15)
                screenshot = out/(name+'-saved.png')
                x, y, width, height = robot.geometry()
                ImageGrab.grab(xdisplay=os.environ['DISPLAY']).crop((x, y, x+width, y+height)).save(screenshot)
                close_window()
                app.wait(timeout=15)
                assert app.returncode == 0, app.returncode
                persisted = json.loads(session.read_text())
                assert persisted['provider_configs'] == [], persisted.get('provider_configs')
                assert persisted['active'] == str(source), persisted
                assert source.read_bytes() == expected
                report['runs'].append({'name': name, 'pid': app.pid, 'exit_code': app.returncode,
                                       'host_tools': available, 'providers': observed['providers'],
                                       'provider_startup': observed['provider_startup'],
                                       'screenshot': screenshot.name,
                                       'screenshot_sha256': hashlib.sha256(screenshot.read_bytes()).hexdigest()})
                report['checks'].extend([name+': empty installed manifest accepted',
                                         name+': no checkout tools or trust prompt',
                                         name+': real keyboard edit and Ctrl+S exact bytes',
                                         name+': clean WM_DELETE and session persisted'])
                app = None
        report['success'] = True
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
        (out/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
    print(f'Core native release checks passed: {len(report["checks"])}', flush=True)


if __name__ == '__main__':
    main()
