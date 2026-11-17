#!/usr/bin/env python3
"""Fresh-user portable CLI on cached Ubuntu 22.04, offline and without host runtimes."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import uuid


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--bundle',type=Path,required=True)
    p.add_argument('--out',type=Path,required=True)
    args=p.parse_args()
    bundle=args.bundle.resolve(strict=True);out=args.out.resolve();out.mkdir(parents=True,exist_ok=False)
    home=out/'home with spaces';home.mkdir()
    token=uuid.uuid4().hex
    name='codisseu-cli-offline-'+token[:12]
    label='org.codisseu.cli-fixture='+token
    image=subprocess.check_output(['docker','image','inspect','--format','{{.Id}}','mongo:6.0'],text=True,timeout=15).strip()
    command=['docker','run','--rm','--name',name,'--label',label,'--pull','never','--network','none',
             '--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--pids-limit','128',
             '--memory','512m','--cpus','2','--user',f'{os.getuid()}:{os.getgid()}',
             '--tmpfs','/tmp:rw,nosuid,size=64m',
             '--mount',f'type=bind,src={bundle},dst=/opt/Codisseu source,readonly',
             '--mount',f'type=bind,src={home},dst=/home/Codisseu user',
             '-e','HOME=/home/Codisseu user','-e','XDG_DATA_HOME=/home/Codisseu user/data',
             '-e','XDG_CONFIG_HOME=/home/Codisseu user/config','-e','XDG_STATE_HOME=/home/Codisseu user/state',
             '-e','XDG_CACHE_HOME=/home/Codisseu user/cache','-e','SHELL=/bin/bash','-e','PATH=/usr/bin:/bin',
             '--entrypoint','/bin/sh',image,'-ec',r'''
cat /etc/os-release > "$HOME/os-release"
"/opt/Codisseu source/install.sh" --prefix "$HOME/Codisseu moved install"
"/opt/Codisseu source/install.sh" --prefix "$HOME/Codisseu moved install"
. "$HOME/.profile"
command -v codisseu > "$HOME/command-path.txt"
codisseu --version > "$HOME/version.txt"
codisseu --help > "$HOME/help.txt"
codisseu --runtime-info > "$HOME/runtime.json"
mkdir -p "$HOME/Projeto atual"
cd "$HOME/Projeto atual"
printf '<?php // owned portable CLI fixture\n' > probe.php
codisseu --check-file probe.php > "$HOME/check-file.txt"
if codisseu . > "$HOME/no-display.stdout" 2> "$HOME/no-display.stderr"; then exit 81; fi
grep -q 'sessão gráfica' "$HOME/no-display.stderr"
if codisseu absent > "$HOME/missing.stdout" 2> "$HOME/missing.stderr"; then exit 82; fi
grep -q 'Não foi possível abrir' "$HOME/missing.stderr"
''']
    report={'kind':'Relocated install in cached Ubuntu 22.04, no network or personal mounts',
            'image_id':image,'container_name':name,'label':label,'command':command}
    try:
        with (out/'run.log').open('w') as log:
            result=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,timeout=240)
        report['exit']=result.returncode
        assert result.returncode==0,'See offline CLI run.log'
        os_release=(home/'os-release').read_text()
        assert 'ID=ubuntu\n' in os_release and 'VERSION_ID="22.04"' in os_release
        assert (home/'command-path.txt').read_text().strip()=='/home/Codisseu user/.local/bin/codisseu'
        assert 'Codisseu 0.1.0 (linux-x86_64)' in (home/'version.txt').read_text()
        runtime=json.loads((home/'runtime.json').read_text())
        prefix='/home/Codisseu user/Codisseu moved install/'
        assert runtime['resources'].startswith(prefix)
        assert all(v.startswith(prefix) for v in runtime['tools'].values())
        assert all(v['program'].startswith(prefix) for v in runtime['providers'])
        assert 'probe.php:' in (home/'check-file.txt').read_text()
        rc=(home/'.bashrc').read_text();assert rc.count('# >>> Codisseu command PATH >>>')==1
        report['checks']=['Ubuntu22 OS verified','Repeated relocated install','Fresh PATH registration',
                          'Version and help without GUI','All runtime paths confined to moved install',
                          'Existing file opened without host PHP/Node/Python','Headless GUI returns clear error',
                          'Missing relative path rejected','No duplicate shell hook']
        report['host_runtimes_or_user_profiles_mounted']=False
    finally:
        inspect=subprocess.run(['docker','container','inspect','--format','{{json .Config.Labels}}',name],capture_output=True,text=True,timeout=15)
        if inspect.returncode==0:
            key,value=label.split('=',1);assert json.loads(inspect.stdout).get(key)==value
            subprocess.run(['docker','container','rm','--force',name],check=True,capture_output=True,timeout=20)
        absent=subprocess.run(['docker','container','inspect',name],capture_output=True,timeout=15)
        report['owned_container_absent']=absent.returncode!=0
        (out/'manifest.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
    return 0


if __name__=='__main__':raise SystemExit(main())
