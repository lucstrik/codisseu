#!/usr/bin/env python3
"""Exercise real cell editing through PID-guarded XTest in private X11.

Run under private_native_run.py. Uses a disposable SQLite fixture and independent
connections to verify staging, Submit, Commit and Rollback. AT-SPI supplies
semantic targets and values; no application state or database response is faked.
"""
import argparse
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import traceback

from native_controls import ROOT, X11


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--resources', type=Path,
                        help='Extracted core share/codisseu with authoritative empty runtime.json')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--scale', type=float, choices=[1., 1.25], default=1.)
    args = parser.parse_args()
    assert Path(os.environ.get('XAUTHORITY', '/missing')).parent.name.startswith('storm-private-'), 'Use private_native_run.py'
    if os.environ.get('CODISSEU_EDIT_PRIVATE_BUS') != '1':
        access = Path.home()/'.local/share/storm-tools/editor-native-access-20261022/root/usr/lib/x86_64-linux-gnu'
        env = dict(os.environ, CODISSEU_EDIT_PRIVATE_BUS='1',
                   GI_TYPELIB_PATH=str(access/'girepository-1.0'), LD_LIBRARY_PATH=str(access))
        env.pop('AT_SPI_BUS_ADDRESS', None)
        with tempfile.TemporaryDirectory(prefix='codisseu-edit-bus-') as bus_home:
            env.update(HOME=bus_home, XDG_CONFIG_HOME=str(Path(bus_home)/'config'),
                       XDG_DATA_HOME=str(Path(bus_home)/'data'),
                       XDG_STATE_HOME=str(Path(bus_home)/'state'),
                       XDG_CACHE_HOME=str(Path(bus_home)/'cache'), GSETTINGS_BACKEND='memory')
            result = subprocess.call(['dbus-run-session', '--', sys.executable,
                                      str(Path(__file__).resolve()), *sys.argv[1:]], env=env)
        report_path = args.out.resolve()/'report.json'
        if report_path.is_file():
            record = json.loads(report_path.read_text())
            record['temporary_accessibility_home_removed'] = not Path(bus_home).exists()
            report_path.write_text(json.dumps(record, ensure_ascii=False, indent=2)+'\n')
        return result

    import dbus
    import gi
    gi.require_version('Atspi', '2.0')
    from gi.repository import Atspi
    from PIL import ImageGrab
    bus = dbus.SessionBus()
    status = dbus.Interface(bus.get_object('org.a11y.Bus', '/org/a11y/bus'),
                            'org.freedesktop.DBus.Properties')
    status.Set('org.a11y.Status', 'IsEnabled', dbus.Boolean(True))
    status.Set('org.a11y.Status', 'ScreenReaderEnabled', dbus.Boolean(True))
    Atspi.init()
    Atspi.get_desktop(0)

    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve(strict=True)
    resources = args.resources.resolve(strict=True) if args.resources else None
    base = Path(tempfile.mkdtemp(prefix='codisseu-edit-native-'))
    base.chmod(0o700)
    home, project = base/'home', base/'fixture'
    home.mkdir()
    project.mkdir()
    database, query = project/'own.sqlite', project/'query.sql'
    sql = 'SELECT * FROM users ORDER BY id;\n'
    query.write_text(sql)
    baseline = [(1, 'Ada', 7, 1, 'Own target'), (2, 'Bruno', 99, 0, 'Untouched')]
    with sqlite3.connect(database) as connection:
        connection.execute('CREATE TABLE users(id INTEGER PRIMARY KEY,name TEXT NOT NULL,quantity BIGINT NULL,enabled BOOLEAN,note TEXT)')
        connection.executemany('INSERT INTO users VALUES(?,?,?,?,?)', baseline)
    env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home/'config'),
               XDG_DATA_HOME=str(home/'data'), XDG_STATE_HOME=str(home/'state'),
               XDG_CACHE_HOME=str(home/'cache'), WINIT_UNIX_BACKEND='x11',
               WINIT_X11_SCALE_FACTOR=str(args.scale), LANG='C.UTF-8')
    for name in ['WAYLAND_DISPLAY', 'CODISSEU_RESOURCE_DIR', 'CODISSEU_PROVIDER_CONFIG']:
        env.pop(name, None)
    if resources:
        manifest = json.loads((resources/'runtime.json').read_text())
        assert manifest == {'schema_version': 1, 'tools': {}, 'providers': []}, manifest
        env['CODISSEU_RESOURCE_DIR'] = str(resources)
    robot, app = X11(), None
    report = {'scenario': 'native SQLite typed result editing, pending, Submit, Commit and Rollback',
              'success': False, 'scale': args.scale, 'binary': str(binary),
              'binary_sha256': hashlib.file_digest(binary.open('rb'), 'sha256').hexdigest(),
              'resources': str(resources) if resources else None, 'checks': [],
              'captures': [], 'native_targets': [], 'independent_reads': [],
              'limits': ['Linux X11 only; native scenario uses SQLite, not remote protocols',
                         'No claim of full DataGrip parity; other engines require separate integration tests']}

    def wait(predicate, seconds=20):
        deadline = time.monotonic()+seconds
        while time.monotonic() < deadline:
            result = predicate()
            if result:
                return result
            assert app is not None and app.poll() is None, 'Application exited before expected state'
            time.sleep(.05)
        raise TimeoutError('Native grid-edit condition did not become true')

    def nodes():
        desktop, pending = Atspi.get_desktop(0), []
        for i in range(desktop.get_child_count()):
            try:
                node = desktop.get_child_at_index(i)
                if node and app and node.get_process_id() == app.pid:
                    pending.append(node)
            except Exception:
                pass
        visited = 0
        while pending:
            node = pending.pop()
            visited += 1
            assert visited < 20000, 'Unexpected accessibility tree size'
            if node is None:
                continue
            yield node
            try:
                pending.extend(node.get_child_at_index(i) for i in range(node.get_child_count()))
            except Exception:
                pass

    def find(label, prefix=False):
        fallback = None
        for node in nodes():
            try:
                name = node.get_name().rstrip()
                if (name.startswith(label) if prefix else name == label):
                    rect = node.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
                    if rect.width > 0 and rect.height > 0:
                        if node.get_role_name() not in ['label', 'static', 'static text']:
                            return node
                        fallback = node
            except Exception:
                pass
        return fallback

    def enabled(label):
        node = find(label)
        return node if node and node.get_state_set().contains(Atspi.StateType.ENABLED) else None

    def click(label, double=False):
        node = wait(lambda: find(label))
        rect = node.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
        x, y, width, height = robot.geometry()
        px, py = round(rect.x-x+rect.width/2), round(rect.y-y+rect.height/2)
        assert 0 <= px < width and 0 <= py < height, 'Offscreen target: '+label
        assert robot.pid(robot.window) == app.pid
        report['native_targets'].append({'label': label, 'double_click': double,
                                          'role': node.get_role_name(), 'window_px': [px, py]})
        robot.click(px, py)
        if double:
            time.sleep(.045)
            robot.click(px, py)
        time.sleep(.12)

    def key(name, modifiers=()):
        robot.key(name, modifiers)
        time.sleep(.10)

    def edit(label, column, value):
        click(label, double=True)
        wait(lambda: find('Editar '+column))
        key('a', ('Control_L',))
        robot.text(value)
        wait(lambda: input_text(column) == value)

    def input_text(column):
        node = find('Editar '+column)
        if not node:
            return None
        try:
            value = Atspi.Text.get_text(node, 0, -1)
        except Exception as error:
            value = None
            report['last_input_read_error'] = repr(error)
        observation = {'column': column, 'text': value, 'role': node.get_role_name(),
                       'interfaces': node.get_interfaces(), 'attributes': node.get_attributes(),
                       'character_count': Atspi.Text.get_character_count(node),
                       'children': []}
        for index in range(min(node.get_child_count(), 20)):
            child = node.get_child_at_index(index)
            if child:
                observation['children'].append({'name': child.get_name(), 'role': child.get_role_name()})
        if observation != report.get('last_input_observation'):
            report.setdefault('input_observations', []).append(observation)
        report['last_input_observation'] = observation
        return value

    def rows():
        with sqlite3.connect(database, timeout=.25) as connection:
            return connection.execute('SELECT * FROM users ORDER BY id').fetchall()

    def record_rows(name, expected):
        actual = rows()
        assert actual == expected, {'phase': name, 'actual': actual, 'expected': expected}
        report['independent_reads'].append({'phase': name, 'rows': actual})

    def capture(name):
        assert robot.pid(robot.window) == app.pid
        x, y, width, height = robot.geometry()
        path = out/(name+'.png')
        ImageGrab.grab(xdisplay=os.environ['DISPLAY']).crop((x, y, x+width, y+height)).save(path)
        report['captures'].append({'path': path.name, 'size_px': [width, height],
                                   'sha256': hashlib.file_digest(path.open('rb'), 'sha256').hexdigest(),
                                   'source': 'real private X11 application window pixels'})

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
                                robot.x.XInternAtom(robot.display, b'WM_PROTOCOLS', 0), 32,
                                (C.c_long*5)(robot.x.XInternAtom(robot.display, b'WM_DELETE_WINDOW', 0), 0, 0, 0, 0))
        robot.x.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.POINTER(Event)]
        robot.x.XSendEvent(robot.display, robot.window, 0, 0, C.byref(event))
        robot.x.XFlush(robot.display)
        app.wait(timeout=20)
        assert app.returncode == 0, app.returncode

    try:
        if resources:
            runtime = subprocess.run([str(binary), '--runtime-info'], env=env,
                                     capture_output=True, text=True, check=True, timeout=10)
            runtime = json.loads(runtime.stdout)
            assert Path(runtime['resources']).resolve(strict=True) == resources
            assert runtime['tools'] == {} and runtime['providers'] == []
            report['runtime_info'] = runtime
            report['checks'].append('packaged empty core manifest loaded without checkout fallback')
        ready = out/'ready.png'
        command = [str(binary), '--no-restore', '--no-auto-providers', '--workspace', str(project),
                   '--open', str(query), '--database', str(database), '--execute-sql',
                   '--session', str(home/'session.json'), '--capture', str(ready)]
        report['command'] = command
        with (out/'application.log').open('w') as log:
            app = subprocess.Popen(command, cwd=project, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        report['pid'] = app.pid
        wait(lambda: ready.with_suffix('.json').exists(), 90)
        state = json.loads(ready.with_suffix('.json').read_text())
        assert state['pixels_per_point'] == args.scale
        assert state['observed']['providers'] == []
        wait(lambda: robot.find(app.pid, (round(1280*args.scale), round(800*args.scale))))
        assert hashlib.file_digest(Path(f'/proc/{app.pid}/exe').open('rb'), 'sha256').hexdigest() == report['binary_sha256']
        robot.focus()
        wait(lambda: find('Ada'))
        wait(lambda: enabled('Habilitar edição'))
        record_rows('read-only baseline', baseline)
        capture('read-only')
        report['checks'].append('query starts read-only with original real rows')
        click('Habilitar edição')
        wait(lambda: find('Habilitar edição') is None and find('Ada'))
        report['checks'].append('explicit SQLite editing enablement reloads original query')

        edit('Ada', 'name', 'Ada committed')
        capture('inline-name')
        key('Return')
        wait(lambda: find('Editar name') is None and find('Ada committed'))
        record_rows('staged text unchanged independently', baseline)
        capture('pending-name')
        report['checks'].append('double-click uses real inline input; Enter stages without writing')
        click('Commit')
        committed = [(1, 'Ada committed', 7, 1, 'Own target'), baseline[1]]
        wait(lambda: rows() == committed)
        wait(lambda: find('Ada committed') and not enabled('Rollback'))
        record_rows('Commit changes exactly target row', committed)
        capture('committed-name')
        report['checks'].append('Commit directly submits pending edit and confirms exactly target row')

        edit('Ada committed', 'name', 'Ada rollback')
        key('Return')
        wait(lambda: find('Editar name') is None)
        click('Submit')
        wait(lambda: enabled('Rollback') and find('Ada rollback'))
        record_rows('Submit remains invisible independently', committed)
        capture('submitted-uncommitted')
        click('Rollback')
        wait(lambda: find('Ada committed') and not enabled('Rollback'))
        record_rows('Rollback restores original committed row', committed)
        report['checks'].append('Submit stays in same uncommitted session; Rollback restores it')

        click('7', double=True)
        wait(lambda: find('Editar quantity'))
        key('a', ('Control_L',))
        robot.text('letters')
        wait(lambda: input_text('quantity') == '7')
        assert find('quantity exige', prefix=True)
        record_rows('numeric letters rejected without writes', committed)
        capture('numeric-rejected')
        key('a', ('Control_L',))
        robot.text('42')
        wait(lambda: input_text('quantity') == '42')
        key('Return')
        wait(lambda: find('Editar quantity') is None and find('42'))
        record_rows('integer staged only', committed)
        click('Commit')
        numeric = [(1, 'Ada committed', 42, 1, 'Own target'), baseline[1]]
        wait(lambda: rows() == numeric)
        wait(lambda: find('42') and not enabled('Rollback'))
        record_rows('integer committed exactly', numeric)
        report['checks'].append('BIGINT rejects alphabetic typing, accepts integer, persists without touching other row')

        click('42', double=True)
        wait(lambda: find('Editar quantity'))
        click('NULL')
        key('Return')
        wait(lambda: find('Editar quantity') is None)
        record_rows('NULL stays pending independently', numeric)
        capture('pending-null')
        click('Commit')
        nullable = [(1, 'Ada committed', None, 1, 'Own target'), baseline[1]]
        wait(lambda: rows() == nullable)
        wait(lambda: not enabled('Rollback'))
        record_rows('nullable integer commits real SQL NULL', nullable)
        report['checks'].append('nullable numeric column distinguishes SQL NULL and commits it')
        assert query.read_text() == sql
        report['checks'].append('SQL source and unrelated row/columns preserved')
        capture('final-grid')
        close_window()
        report['checks'].append('clean native WM_DELETE exit0')
        report['success'] = True
    except Exception as error:
        report['failure'] = {'type': type(error).__name__, 'message': str(error),
                             'traceback': traceback.format_exc()}
        if app and app.poll() is None and robot.window:
            try:
                capture('failure')
                report['accessible_nodes_at_failure'] = [
                    {'name': node.get_name(), 'role': node.get_role_name()} for node in nodes()]
            except Exception:
                pass
        raise
    finally:
        if app and app.poll() is None:
            app.terminate()
            try:
                app.wait(timeout=10)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait(timeout=5)
        report['app_exit'] = app.returncode if app else None
        report['pid_absent'] = not app or not Path(f'/proc/{app.pid}').exists()
        robot.x.XCloseDisplay(robot.display)
        shutil.rmtree(base)
        report['fixture_home_profile_removed'] = not base.exists()
        (out/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps({'success': report['success'], 'checks': len(report['checks']),
                      'windows': 1, 'scale': args.scale}), flush=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
