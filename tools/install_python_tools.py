#!/usr/bin/env python3
"""Explicitly prepare pinned private Python tools; never use global pip or project hooks."""
import argparse,base64,hashlib,json,os,platform,shutil,subprocess,tarfile,tempfile,urllib.request
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
LOCK=ROOT/'tools/python-tools.lock.json'
def verify_standalone_notices(target):
    root=target/'python';metadata=json.loads((root/'PYTHON.json').read_text());pending=[metadata];count=0
    while pending:
        value=pending.pop()
        if isinstance(value,dict):
            for key,item in value.items():
                if key in ['license_path','license_paths']:
                    for relative in([item]if isinstance(item,str)else item):
                        path=root/relative
                        if not path.resolve(strict=True).is_relative_to(root.resolve())or not path.is_file():raise RuntimeError('Official Python metadata license absent/outside notices')
                        count+=1
                if isinstance(item,(dict,list)):pending.append(item)
        elif isinstance(value,list):pending.extend(value)
    if count<1:raise RuntimeError('Official Python metadata license inventory empty')
    return count
def digest(path):
    with path.open('rb')as stream:return hashlib.file_digest(stream,'sha256').hexdigest()
def verify(destination):
    record=json.loads((destination/'codisseu-install.json').read_text())
    if record['lock_sha256']!=digest(LOCK):raise RuntimeError('Private Python tool versions differ from pinned lock')
    expected={s['name']:s['version']for s in json.loads(LOCK.read_text())['sources']}
    if any(record['versions'].get(name)!=version for name,version in expected.items()):raise RuntimeError('Private Python component identity mismatch')
    for required in ['NOTICES/standalone/python/PYTHON.json','NOTICES/Basedpyright-LICENSE.txt','NOTICES/Basedpyright-typeshed-LICENSE.txt','cpython/lib/python3.13/site-packages/debugpy/adapter/__main__.py','cpython/lib/python3.13/site-packages/pytest/__init__.py','cpython/bin/ruff','node_modules/basedpyright/langserver.index.js']:
        if required not in record['files']:raise RuntimeError('Private Python resource/notice missing: '+required)
    for relative,sha in record['files'].items():
        path=destination/relative
        if not path.resolve(strict=True).is_relative_to(destination.resolve())or digest(path)!=sha:raise RuntimeError('Private Python resource modified: '+relative)
    actual={str(p.relative_to(destination))for p in destination.rglob('*')if p.is_file()and p.name!='codisseu-install.json'and'__pycache__'not in p.parts and p.suffix!='.pyc'}
    if actual!=set(record['files']):raise RuntimeError('Private Python installation contains untracked resources')
    verify_standalone_notices(destination/'NOTICES/standalone')
    print(json.dumps({'destination':str(destination),'verified_files':len(record['files']),'versions':record['versions']}))
def download(source,destination):
    request=urllib.request.Request(source['url'],headers={'User-Agent':'Codisseu-private-tools'})
    with urllib.request.urlopen(request,timeout=120)as incoming,destination.open('wb')as output:
        total=0
        while block:=incoming.read(1024*1024):
            total+=len(block)
            if total>128*1024*1024:raise RuntimeError('Pinned Python download exceeds limit')
            output.write(block)
    if 'bytes'in source and total!=source['bytes']:raise RuntimeError('Pinned Python archive size mismatch')
    if 'sha256'in source and digest(destination)!=source['sha256']:raise RuntimeError('Pinned Python archive SHA mismatch')
    if 'integrity'in source:
        expected=source['integrity'].removeprefix('sha512-')
        if base64.b64encode(hashlib.sha512(destination.read_bytes()).digest()).decode()!=expected:raise RuntimeError('Pinned npm integrity mismatch')
def extract(archive,target):
    with tarfile.open(archive,'r:gz')as tar:
        members=tar.getmembers()
        if len(members)>50000 or sum(m.size for m in members)>1024*1024*1024:raise RuntimeError('Python extraction limits exceeded')
        tar.extractall(target,filter='data')
    for path in target.rglob('*'):
        if path.is_symlink()and(not path.resolve().is_relative_to(target.resolve())or os.path.isabs(os.readlink(path))):raise RuntimeError('Escaping Python symlink')
def extract_notices(archive,target):
    target.mkdir(parents=True,exist_ok=True)
    decoder=subprocess.Popen(['zstd','-dc',str(archive)],stdout=subprocess.PIPE)
    total=0
    with tarfile.open(fileobj=decoder.stdout,mode='r|')as tar:
        for member in tar:
            name=Path(member.name)
            if not member.isfile()or not(name.name=='PYTHON.json'or'license'in name.name.lower()or name.name.lower().startswith('copying')):continue
            if name.is_absolute()or'..'in name.parts or'\\'in member.name:raise RuntimeError('Invalid Python notice path')
            total+=member.size
            if total>32*1024*1024:raise RuntimeError('Python notice limit exceeded')
            destination=target/name;destination.parent.mkdir(parents=True,exist_ok=True)
            with tar.extractfile(member)as incoming,destination.open('wb')as output:shutil.copyfileobj(incoming,output)
    if decoder.wait()!=0:raise RuntimeError('Cannot decode official Python metadata')
    if not(target/'python/PYTHON.json').is_file():raise RuntimeError('Official standalone metadata absent')
    verify_standalone_notices(target)
def console_wrappers(payload):
    for name,module in [('pytest','pytest'),('py.test','pytest'),('pip','pip'),('pip3','pip'),('pip3.13','pip'),('debugpy','debugpy'),('debugpy-adapter','debugpy.adapter'),('pygmentize','pygments')]:
        path=payload/'cpython/bin'/name
        path.write_text('#!/bin/sh\nset -eu\npython_bin=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexec "$python_bin/python3" -B -m '+module+' "$@"\n');path.chmod(0o755)
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--verify',action='store_true');parser.add_argument('--destination',type=Path,default=ROOT/'.tools/python');args=parser.parse_args();destination=args.destination.absolute()
    if destination.is_symlink():raise RuntimeError('Refusing symlink destination')
    if destination.exists():verify(destination);return
    if args.verify:raise RuntimeError('Install private Python tools explicitly first')
    if platform.system()!='Linux'or platform.machine()!='x86_64':raise RuntimeError('Pinned Python tools currently target Linux x86_64')
    sources=json.loads(LOCK.read_text())['sources'];destination.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='codisseu-python-',dir=destination.parent)as work:
        work=Path(work);payload=work/'payload';payload.mkdir();wheels=payload/'wheelhouse';wheels.mkdir();notices=payload/'NOTICES';notices.mkdir()
        for source in sources:
            archive=work/(source.get('filename')or source['name']+'.tgz');download(source,archive)
            if source['kind']=='python':
                extract(archive,work/'cpython');shutil.move(work/'cpython/python',payload/'cpython')
            elif source['kind']=='npm':
                extract(archive,work/'npm');node_modules=payload/'node_modules';node_modules.mkdir();shutil.move(work/'npm/package',node_modules/source['name'])
            elif source['kind']=='notices':extract_notices(archive,notices/'standalone')
            else:shutil.copy2(archive,wheels/archive.name)
        python=payload/'cpython/bin/python3'
        # Inherited PIP_TARGET/PIP_PREFIX/PIP_USER/PYTHONHOME must never redirect
        # this preparation into a user's global or project environment.
        env={'PATH':os.environ.get('PATH','/usr/bin:/bin'),'HOME':str(work),'LANG':'C.UTF-8','LC_ALL':'C.UTF-8','PIP_CONFIG_FILE':'/dev/null','PYTHONNOUSERSITE':'1','PYTHONDONTWRITEBYTECODE':'1'}
        command=[str(python),'-I','-B','-m','pip','install','--no-index','--no-deps','--no-compile','--disable-pip-version-check']+[str(p)for p in sorted(wheels.glob('*.whl'))]
        subprocess.run(command,check=True,env=env,cwd=work,timeout=120)
        console_wrappers(payload)
        shutil.copy2(LOCK,notices/'python-tools.lock.json')
        # Upstream CPython installation contains LICENSE.txt; Python's complete
        # standalone dependency license directory is preserved with the payload.
        for path in(payload/'cpython').rglob('*'):
            if path.is_file()and('license'in path.name.lower()or'copying'in path.name.lower()):
                relative=path.relative_to(payload);target=notices/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(path,target)
        node_license=payload/'node_modules/basedpyright/LICENSE.txt'
        if not node_license.is_file():node_license=payload/'node_modules/basedpyright/LICENSE'
        if not node_license.is_file():raise RuntimeError('Basedpyright license absent')
        shutil.copy2(node_license,notices/'Basedpyright-LICENSE.txt')
        shutil.copy2(payload/'node_modules/basedpyright/dist/typeshed-fallback/LICENSE',notices/'Basedpyright-typeshed-LICENSE.txt')
        versions={s['name']:s['version']for s in sources}
        versions['pip']=subprocess.check_output([str(python),'-I','-B','-c','import pip; print(pip.__version__)'],text=True,env=env,cwd=work).strip()
        for path in payload.rglob('__pycache__'):shutil.rmtree(path)
        files={str(p.relative_to(payload)):digest(p)for p in sorted(payload.rglob('*'))if p.is_file()}
        (payload/'codisseu-install.json').write_text(json.dumps({'schema_version':1,'lock_sha256':digest(LOCK),'versions':versions,'sources':sources,'files':files},indent=2)+'\n')
        payload.rename(destination)
    verify(destination)
if __name__=='__main__':main()
