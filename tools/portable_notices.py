#!/usr/bin/env python3
"""Audit notices for the pinned, unpacked Phpactor; fetch only public source files."""
import concurrent.futures, hashlib, json, re, subprocess, urllib.request
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
DEST=ROOT/'reference/public/portable-runtime'
PHP=ROOT/'.tools/phpactor-2026.10.07.0'

def fetch(url):
    request=urllib.request.Request(url,headers={'User-Agent':'Codisseu-notice-audit/1'})
    with urllib.request.urlopen(request,timeout=20)as response:
        data=response.read(1024*1024+1)
        if len(data)>1024*1024:raise ValueError('Notice too large')
        return data

def main():
    DEST.mkdir(parents=True,exist_ok=True)
    installed=json.loads(subprocess.run(['php','-n','-r','echo json_encode(require $argv[1]);',str(PHP/'vendor/composer/installed.php')],capture_output=True,text=True,check=True).stdout)
    reference=installed['root']['reference'];url=f'https://raw.githubusercontent.com/phpactor/phpactor/{reference}/composer.lock'
    lock_path=DEST/'phpactor-composer.lock'
    if not lock_path.exists():lock_path.write_bytes(fetch(url))
    lock=json.loads(lock_path.read_text());packages={p['name']:p for p in lock.get('packages',[])+lock.get('packages-dev',[])}
    def audit(item):
        name,info=item
        if not info.get('install_path') or name=='phpactor/phpactor':return None
        assert re.fullmatch('[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+',name)
        metadata=packages.get(name);expected=info.get('reference')
        entry={'name':name,'reference':expected,'version':info.get('pretty_version')}
        if not metadata or metadata.get('source',{}).get('reference')!=expected:return {**entry,'status':'source-reference-not-matched'}
        source=metadata['source']['url'];entry['source']=source;entry['license']=metadata.get('license',[])
        found=[]
        for candidate in ['LICENSE','LICENSE.md','LICENSE.txt','COPYING']:
            local=Path(info['install_path'])/candidate
            target=DEST/'phpactor-notices'/name/candidate
            data=None;origin=None
            if local.is_file():data=local.read_bytes();origin='included in pinned PHAR'
            elif target.is_file():data=target.read_bytes();origin='cached pinned upstream notice'
            elif source.startswith('https://github.com/'):
                repo=source.removeprefix('https://github.com/').removesuffix('.git')
                try:origin=f'https://raw.githubusercontent.com/{repo}/{expected}/{candidate}';data=fetch(origin)
                except Exception:continue
            if data:
                target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(data)
                found.append({'file':str(target.relative_to(DEST)),'sha256':hashlib.sha256(data).hexdigest(),'origin':origin});break
        return {**entry,'status':'notice-preserved'if found else'notice-missing','notices':found}
    with concurrent.futures.ThreadPoolExecutor(max_workers=4)as pool:entries=[e for e in pool.map(audit,installed['versions'].items())if e]
    result={'phpactor_reference':reference,'lock_url':url,'lock_sha256':hashlib.sha256(lock_path.read_bytes()).hexdigest(),'packages':entries,'complete':all(e['status']=='notice-preserved'for e in entries)}
    (DEST/'phpactor-notices.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({'complete':result['complete'],'packages':len(entries),'unresolved':[e['name']for e in entries if e['status']!='notice-preserved']}))
if __name__=='__main__':main()
