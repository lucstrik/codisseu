#!/usr/bin/env python3
"""Verify live application themes through native settings and a disposable Git repo.

Run through private_native_run.py. All input targets the verified private X11
window. Settings are selected by their accessible names; screenshots contain
real window pixels. HOME, profile, session and repository are disposable.
"""
import argparse
from collections import Counter
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import traceback

from PIL import ImageGrab

from native_controls import ROOT, X11


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--resources', type=Path,
                        help='Verified core package share/codisseu directory; requires its empty runtime manifest')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--scale', type=float, choices=[1., 1.25], default=1.)
    parser.add_argument('--night-palette', type=Path, default=ROOT/'design/themes/midnight.json')
    parser.add_argument('--classic-palette', type=Path, default=ROOT/'design/tokens.json')
    args = parser.parse_args()
    assert Path(os.environ['XAUTHORITY']).parent.name.startswith('storm-private-'), 'Use private_native_run.py'

    # Private AT-SPI bus discovers semantic settings names without connecting to
    # applications on the user's desktop. XTest still performs all activation.
    if os.environ.get('CODISSEU_THEME_PRIVATE_BUS') != '1':
        env = dict(os.environ, CODISSEU_THEME_PRIVATE_BUS='1')
        env.pop('AT_SPI_BUS_ADDRESS', None)
        access = Path.home()/'.local/share/storm-tools/editor-native-access-20261022/root/usr/lib/x86_64-linux-gnu'
        env['GI_TYPELIB_PATH'] = str(access/'girepository-1.0')
        env['LD_LIBRARY_PATH'] = str(access)
        with tempfile.TemporaryDirectory(prefix='codisseu-theme-bus-') as bus_home:
            env.update(HOME=bus_home, XDG_CONFIG_HOME=str(Path(bus_home)/'config'),
                       XDG_DATA_HOME=str(Path(bus_home)/'data'),
                       XDG_STATE_HOME=str(Path(bus_home)/'state'),
                       XDG_CACHE_HOME=str(Path(bus_home)/'cache'), GSETTINGS_BACKEND='memory')
            result = subprocess.call(['dbus-run-session', '--', sys.executable,
                                      str(Path(__file__).resolve()), *sys.argv[1:]], env=env)
        record_path = args.out.resolve()/'report.json'
        if record_path.is_file():
            record = json.loads(record_path.read_text())
            record['temporary_accessibility_home_removed'] = not Path(bus_home).exists()
            record_path.write_text(json.dumps(record, ensure_ascii=False, indent=2)+'\n')
        return result

    import dbus
    import gi
    gi.require_version('Atspi', '2.0')
    from gi.repository import Atspi
    bus = dbus.SessionBus()
    status = dbus.Interface(bus.get_object('org.a11y.Bus', '/org/a11y/bus'),
                            'org.freedesktop.DBus.Properties')
    status.Set('org.a11y.Status', 'IsEnabled', dbus.Boolean(True))
    status.Set('org.a11y.Status', 'ScreenReaderEnabled', dbus.Boolean(True))
    Atspi.init()
    Atspi.get_desktop(0)

    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    source_binary = args.binary.resolve(strict=True)
    resources = args.resources.resolve(strict=True) if args.resources is not None else None
    digest = hashlib.file_digest(source_binary.open('rb'), 'sha256').hexdigest()
    classic = json.loads(args.classic_palette.read_text())
    night = dict(classic, **json.loads(args.night_palette.read_text()))
    base = Path(tempfile.mkdtemp(prefix='codisseu-native-themes-'))
    base.chmod(0o700)
    robot = X11()
    app = None
    report = {
        'scenario': 'native global themes, persistence, exact Save and real Git gutter',
        'success': False, 'scale': args.scale, 'source_binary': str(source_binary),
        'binary_sha256': digest, 'resources': str(resources) if resources is not None else None,
        'checks': [], 'runs': [], 'captures': [],
        'transport': 'private authenticated X11 and AT-SPI; PID-guarded XTest and WM_DELETE',
        'limits': ['Linux X11/llvmpipe; Wayland, other GPUs and operating systems not exercised',
                   'Does not start language providers, execute project applications or use network services'],
    }

    def scaled(value):
        return round(value*args.scale)

    def wait(predicate, seconds=20):
        deadline = time.monotonic()+seconds
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            assert app is not None and app.poll() is None, f'application exited: {None if app is None else app.returncode}'
            time.sleep(.05)
        raise TimeoutError('Native theme condition did not become true')

    def nodes():
        desktop = Atspi.get_desktop(0)
        pending = []
        for i in range(desktop.get_child_count()):
            try:
                node = desktop.get_child_at_index(i)
                if node and node.get_process_id() == app.pid:
                    pending.append(node)
            except Exception:
                pass
        visited = 0
        while pending:
            node = pending.pop()
            visited += 1
            assert visited < 20_000, 'Unexpected accessibility tree size'
            if node is None:
                continue
            yield node
            try:
                pending.extend(node.get_child_at_index(i) for i in range(node.get_child_count()))
            except Exception:
                pass

    def find(label):
        fallback = None
        for node in nodes():
            try:
                if node.get_name().rstrip() == label:
                    rect = node.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
                    if rect.width > 0 and rect.height > 0:
                        if node.get_role_name() not in ['label', 'static', 'static text']:
                            return node
                        fallback = node
            except Exception:
                pass
        return fallback

    def click_label(label):
        node = wait(lambda: find(label))
        rect = node.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
        x, y, width, height = robot.geometry()
        px, py = rect.x+rect.width//2-x, rect.y+rect.height//2-y
        assert 0 <= px < width and 0 <= py < height, f'Offscreen setting: {label}'
        report.setdefault('native_click_targets', []).append({
            'label': label, 'role': node.get_role_name(), 'window_px': [px, py],
        })
        robot.click(px, py)
        time.sleep(.15)

    def pixels():
        assert robot.pid(robot.window) == app.pid
        x, y, width, height = robot.geometry()
        return ImageGrab.grab(xdisplay=os.environ['DISPLAY']).crop((x, y, x+width, y+height)).convert('RGB')

    def count(image, color, bounds):
        return Counter(image.crop(tuple(scaled(x) for x in bounds)).getdata())[tuple(color)]

    def palette_counts(image, palette):
        return {
            'editor': count(image, palette['editor'], (350, 120, 950, 700)),
            'project': count(image, palette['panel'], (50, 160, 270, 700)),
            'database': count(image, palette['panel'], (1030, 160, 1225, 700)),
            'toolbar': count(image, palette['bar'], (350, 0, 950, 38)),
        }

    def matches_palette(palette):
        result = palette_counts(pixels(), palette)
        report['last_palette_pixel_counts'] = result
        return result if all(value > scaled(1000) for value in result.values()) else None

    def tab_paint(image, palette):
        # Semantic presence alone cannot detect a later background paint that
        # covers tabs that had already been rendered.
        _, y, _, _ = robot.geometry()
        semantic = False
        for node in nodes():
            try:
                rect = node.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
                if (node.get_name() == 'sample.ts' and rect.width > 0
                        and scaled(44) <= rect.y-y <= scaled(84)):
                    semantic = True
                    break
            except Exception:
                pass
        glyphs = image.crop(tuple(scaled(v) for v in (332, 51, 389, 78))).getdata()
        result = {
            'semantic_tab': semantic,
            'visible_label_pixels': sum(min(pixel) > 50 and max(pixel) > 130 for pixel in glyphs),
            'active_underline_pixels': count(image, palette['accent'], (288, 44, 425, 87)),
        }
        assert result['semantic_tab'] and result['visible_label_pixels'] >= 25 and result['active_underline_pixels'] >= 20, result
        return result

    def capture(name, palette=None, gutter=False):
        # Delay only settles the pointer/animation; concrete assertions follow.
        x, y, width, height = robot.geometry()
        robot.t.XTestFakeMotionEvent(robot.display, -1, x+scaled(1240), y+scaled(740), 0)
        robot.x.XFlush(robot.display)
        time.sleep(.2)
        image = pixels()
        assert image.size == (scaled(1280), scaled(800)), image.size
        path = out/(name+'.png')
        image.save(path)
        record = {'path': path.name, 'sha256': hashlib.file_digest(path.open('rb'), 'sha256').hexdigest(),
                  'size_px': list(image.size), 'source': 'real X11 window pixels'}
        if palette:
            record['palette_pixels'] = palette_counts(image, palette)
            assert all(value > scaled(1000) for value in record['palette_pixels'].values()), record
            record['active_tab_paint'] = tab_paint(image, palette)
        if gutter:
            record['gutter_pixels'] = {
                key: count(image, night['editor_git_'+key], (275, 80, 350, 680))
                for key in ['added', 'modified']
            }
            assert all(value >= 6 for value in record['gutter_pixels'].values()), record
        report['captures'].append(record)

    def set_theme(label):
        robot.key('s', ('Control_L', 'Alt_L'))
        wait(lambda: find('Tema do aplicativo'))
        click_label('Tema do aplicativo')
        click_label(label)
        # Keep the actual settings visible as a reviewable intermediate state.
        capture('settings-'+label.split()[-1].lower()+'-'+str(len(report['captures'])))
        robot.key('Escape')
        time.sleep(.2)

    def type_ascii(value):
        for char in value:
            if char == '\n':
                robot.key('Return')
            else:
                shift = char.isupper() or char in '~!@#$%^&*()_+{}|:"<>?'
                robot.key(char, ('Shift_L',) if shift else ())

    def close_window(name):
        nonlocal app

        class Message(C.Structure):
            _fields_ = [('type', C.c_int), ('serial', C.c_ulong), ('send_event', C.c_int),
                        ('display', C.c_void_p), ('window', C.c_ulong), ('message_type', C.c_ulong),
                        ('format', C.c_int), ('data', C.c_long*5)]

        class Event(C.Union):
            _fields_ = [('message', Message), ('pad', C.c_long*24)]

        assert robot.pid(robot.window) == app.pid
        event = Event()
        event.message = Message(33, 0, 1, robot.display, robot.window,
                                robot.x.XInternAtom(robot.display, b'WM_PROTOCOLS', 0), 32,
                                (C.c_long*5)(robot.x.XInternAtom(robot.display, b'WM_DELETE_WINDOW', 0), 0, 0, 0, 0))
        robot.x.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.POINTER(Event)]
        robot.x.XSendEvent(robot.display, robot.window, 0, 0, C.byref(event))
        robot.x.XFlush(robot.display)
        app.wait(timeout=20)
        assert app.returncode == 0, f'{name}: exit {app.returncode}'
        report['runs'][-1]['exit_code'] = app.returncode
        report['checks'].append(name+': clean WM_DELETE and session persistence')
        app = None

    try:
        binary = base/'codisseu-under-test'
        shutil.copy2(source_binary, binary)
        home = base/'home'
        project = base/'Theme Project'
        home.mkdir()
        project.mkdir()
        source = project/'sample.ts'
        original = ("// owned native theme fixture\nexport const label = 'base';\n"
                    "export function compute(value: number) {\n    return value + 1;\n}\n"+
                    ''.join(f'// preserved context {n}\n' for n in range(10))+
                    'export const footer = true;\n')
        source.write_text(original)
        env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home/'config'),
                   XDG_DATA_HOME=str(home/'data'), XDG_CACHE_HOME=str(home/'cache'),
                   XDG_STATE_HOME=str(home/'state'), WINIT_UNIX_BACKEND='x11',
                   WINIT_X11_SCALE_FACTOR=str(args.scale), LANG='C.UTF-8')
        for key in ['CODISSEU_RESOURCE_DIR', 'CODISSEU_PROVIDER_CONFIG']:
            env.pop(key, None)
        if resources is not None:
            manifest_path = resources/'runtime.json'
            manifest = json.loads(manifest_path.read_text())
            assert manifest == {'schema_version': 1, 'tools': {}, 'providers': []}, manifest
            env['CODISSEU_RESOURCE_DIR'] = str(resources)
            result = subprocess.run([str(binary), '--runtime-info'], env=env, capture_output=True,
                                    text=True, check=True, timeout=10)
            runtime = json.loads(result.stdout)
            assert Path(runtime['resources']).resolve(strict=True) == resources, runtime
            assert runtime['tools'] == {} and runtime['providers'] == [], runtime
            report['runtime_info'] = runtime
            report['runtime_manifest_sha256'] = hashlib.file_digest(manifest_path.open('rb'), 'sha256').hexdigest()
            report['checks'].append('exact packaged empty core runtime manifest loaded without checkout fallback')

        def git(arguments):
            return subprocess.run(['git', '-C', str(project), *arguments],
                                  env=dict(env, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null'),
                                  capture_output=True, check=True).stdout

        for command in [['-c', 'init.templateDir=', 'init', '-q', '-b', 'main'],
                        ['config', 'core.hooksPath', '/dev/null'], ['config', 'user.name', 'Theme Fixture'],
                        ['config', 'user.email', 'fixture@example.invalid'], ['config', 'commit.gpgsign', 'false'],
                        ['add', 'sample.ts'], ['commit', '-qm', 'Synthetic theme baseline']]:
            git(command)
        head = git(['rev-parse', 'HEAD'])
        index_digest = hashlib.file_digest((project/'.git/index').open('rb'), 'sha256').hexdigest()
        session = home/'state/session.json'

        def launch(name, restore=False):
            nonlocal app
            ready = out/(name+'-ready.png')
            command = [str(binary), '--no-auto-providers', '--session', str(session), '--capture', str(ready)]
            if not restore:
                command.extend(['--no-restore', '--workspace', str(project), '--open', str(source)])
            with (out/(name+'.log')).open('w') as log:
                app = subprocess.Popen(command, cwd=project, env=env, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            report['runs'].append({'name': name, 'pid': app.pid, 'command': command, 'exit_code': None})
            wait(lambda: ready.with_suffix('.json').is_file(), 90)
            state = json.loads(ready.with_suffix('.json').read_text())
            assert state['pixels_per_point'] == args.scale, state['pixels_per_point']
            assert state['observed']['providers'] == [], state['observed']['providers']
            assert not state['observed']['provider_startup']['needs_trust']
            wait(lambda: robot.find(app.pid, (scaled(1280), scaled(800))))
            assert hashlib.file_digest(Path(f'/proc/{app.pid}/exe').open('rb'), 'sha256').hexdigest() == digest
            robot.focus()

        launch('default')
        wait(lambda: matches_palette(night))
        capture('default-noturno', night)
        report['checks'].append('fresh profile uses Noturno across editor, Project, Database and toolbar')

        robot.click(scaled(600), scaled(220))
        # Existing indentation and auto-pairs are product behavior. Edit a
        # comment and append another comment instead of injecting a document
        # containing braces through keyboard input.
        robot.key('Home', ('Control_L',))
        robot.key('Home')
        robot.key('End', ('Shift_L',))
        type_ascii('// modified native theme fixture')
        robot.key('End', ('Control_L',))
        type_ascii('// added theme line\n')
        edited = original.replace('// owned native theme fixture', '// modified native theme fixture')+'// added theme line\n'
        robot.key('s', ('Control_L',))
        wait(lambda: source.read_text() == edited)
        wait(lambda: all(count(pixels(), night['editor_git_'+key], (275, 80, 350, 680)) >= 6
                         for key in ['added', 'modified']))
        capture('noturno-saved-git', night, gutter=True)
        assert git(['rev-parse', 'HEAD']) == head
        assert hashlib.file_digest((project/'.git/index').open('rb'), 'sha256').hexdigest() == index_digest
        report['checks'].extend(['native typing and Ctrl+S preserve exact expected bytes',
                                 'real unstaged Git hunks paint green additions and amber modifications',
                                 'Save leaves Git HEAD and index unchanged'])

        set_theme('Codisseu Clássico')
        wait(lambda: matches_palette(classic))
        capture('classic-live', classic)
        assert source.read_text() == edited
        report['checks'].append('settings switch to Classic repaints all four application surfaces live')
        close_window('classic')
        stored = json.loads(session.read_text())
        assert stored['editor_preferences']['theme'] == 'classic', stored['editor_preferences']
        report['checks'].append('Classic preference persisted in session')

        launch('restored-classic', restore=True)
        wait(lambda: matches_palette(classic))
        capture('classic-restored', classic)
        report['checks'].append('reopening restores Classic and the saved document')
        set_theme('Codisseu Noturno')
        wait(lambda: matches_palette(night))
        capture('noturno-live', night, gutter=True)
        close_window('noturno')
        assert json.loads(session.read_text())['editor_preferences']['theme'] == 'midnight'
        report['checks'].append('Noturno preference persisted after native settings change')

        launch('restored-noturno', restore=True)
        wait(lambda: matches_palette(night))
        capture('noturno-restored', night, gutter=True)
        close_window('restored-noturno')
        assert source.read_text() == edited
        assert git(['rev-parse', 'HEAD']) == head
        assert hashlib.file_digest((project/'.git/index').open('rb'), 'sha256').hexdigest() == index_digest
        report['checks'].append('second reopening restores Noturno with exact saved bytes and original Git baseline')
        report['success'] = True
    except Exception as error:
        report['failure'] = {'type': type(error).__name__, 'message': str(error), 'traceback': traceback.format_exc()}
        if app is not None and app.poll() is None:
            try:
                capture('failure')
                report['accessible_nodes_at_failure'] = [
                    {'name': node.get_name(), 'role': node.get_role_name()} for node in nodes()
                ]
            except Exception:
                pass
        raise
    finally:
        if app is not None and app.poll() is None:
            app.terminate()
            try:
                app.wait(timeout=10)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait(timeout=5)
        robot.x.XCloseDisplay(robot.display)
        shutil.rmtree(base)
        report['temporary_home_and_repository_removed'] = not base.exists()
        (out/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps({'success': report['success'], 'checks': len(report['checks']),
                      'windows': len(report['runs']), 'scale': args.scale}), flush=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
