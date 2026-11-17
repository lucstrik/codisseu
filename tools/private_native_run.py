#!/usr/bin/env python3
"""Run an existing native corpus in an authenticated, disposable Xvfb session."""
import argparse,os,secrets,subprocess,tempfile,time,json,shutil,re
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--record',type=Path,required=True);p.add_argument('--screen',default='1280x800x24');p.add_argument('--gpu',action='store_true',help='Request installed NVIDIA Vulkan driver; application log confirms actual adapter');p.add_argument('command',nargs=argparse.REMAINDER);a=p.parse_args()
assert a.command
assert re.fullmatch(r'[1-9][0-9]{2,3}x[1-9][0-9]{2,3}x24',a.screen), 'Expected bounded WIDTHxHEIGHTx24'
driver=Path('/usr/share/vulkan/icd.d/nvidia_icd.json' if a.gpu else '/usr/share/vulkan/icd.d/lvp_icd.json')
assert driver.is_file(), 'Requested Vulkan driver is not installed'
root=Path(tempfile.mkdtemp(prefix='storm-private-'));root.chmod(0o700)
a.record.parent.mkdir(parents=True,exist_ok=True)
reservation=None
for n in range(220,300):
 if Path(f'/tmp/.X11-unix/X{n}').exists() or Path(f'/tmp/.X{n}-lock').exists():continue
 try: reservation=Path(f'/tmp/storm-private-display-{n}');reservation.mkdir(mode=0o700);display=f':{n}';break
 except FileExistsError:pass
else:raise RuntimeError('No free private display')
auth=root/'Xauthority';auth.touch(mode=0o600);subprocess.run(['xauth','-f',str(auth),'add',display,'.',secrets.token_hex(16)],check=True)
env=os.environ.copy();env.pop('WAYLAND_DISPLAY',None);env.update(DISPLAY=display,XAUTHORITY=str(auth),WINIT_UNIX_BACKEND='x11',XDG_RUNTIME_DIR=str(root),VK_DRIVER_FILES=str(driver))
xvfb=Path.home()/'.local/share/storm-tools/xvfb-21.1.16-deb13u4/usr/bin/Xvfb'
status=None
with a.record.with_suffix('.xvfb.log').open('w')as log:
 server=subprocess.Popen([str(xvfb),display,'-screen','0',a.screen,'-auth',str(auth),'-nolisten','tcp','-noreset'],env=env,stdout=log,stderr=subprocess.STDOUT)
 try:
  for _ in range(100):
   if Path(f'/tmp/.X11-unix/X{n}').exists():break
   assert server.poll()is None;time.sleep(.05)
  status=subprocess.call(a.command,env=env)
 finally:
  server.terminate()
  try:server.wait(timeout=10)
  except subprocess.TimeoutExpired:server.kill();server.wait()
  auth.unlink(missing_ok=True);shutil.rmtree(root);reservation.rmdir()
a.record.write_text(json.dumps({'display':display,'screen':a.screen,'xvfb_pid':server.pid,'xvfb_exit':server.returncode,'pid_absent':not Path(f'/proc/{server.pid}').exists(),'runtime_removed':not root.exists(),'cookie_removed':not auth.exists(),'command':a.command,'corpus_exit':status,'renderer':'NVIDIA Vulkan requested; confirm actual adapter in application log' if a.gpu else 'llvmpipe software Vulkan; actual eframe/winit X11 window','vulkan_driver':str(driver)},indent=2)+'\n')
raise SystemExit(status)
