#!/usr/bin/env python3
"""Explicit private CEF SDK/runtime preparation. No global or setuid installation."""
import argparse,hashlib,json,os,platform,shutil,tarfile,tempfile,urllib.request
from pathlib import Path
from browser_frontend_assets import prepare as prepare_frontend, embedding as prepare_embedding
ROOT=Path(__file__).resolve().parents[1];LOCK=ROOT/'tools/browser-engine.lock.json'
def digest(path):
 with path.open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def verify(base,require_frontend=True):
 record=json.loads((base/'codisseu-install.json').read_text())
 if record['lock_sha256']!=digest(LOCK):raise RuntimeError('CEF pin mismatch')
 for relative,sha in record['files'].items():
  path=base/'cef'/relative
  if not path.resolve(strict=True).is_relative_to((base/'cef').resolve())or digest(path)!=sha:raise RuntimeError('CEF resource modified: '+relative)
 if require_frontend and not(base/'cef/devtools-assets/embedding.json').is_file():raise RuntimeError('Prepare original DevTools assets explicitly: --prepare-frontend')
 if (base/'cef/chrome-sandbox').exists():raise RuntimeError('Private runtime must not install a setuid helper')
 print(json.dumps({'cef':record['source']['cef_version'],'runtime':str(base/'cef'),'verified_files':len(record['files']),'sandbox':'Chromium user namespace sandbox required; never disabled'}))
def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--verify',action='store_true');parser.add_argument('--prepare-frontend',action='store_true');parser.add_argument('--destination',type=Path,default=ROOT/'.tools/browser');args=parser.parse_args();base=args.destination.absolute();source=json.loads(LOCK.read_text())
 if base.is_symlink():raise RuntimeError('Refusing symlink CEF destination')
 if (base/'codisseu-install.json').exists():
  if args.prepare_frontend and (not(base/'cef/devtools-assets/embedding.json').exists()or json.loads((base/'cef/devtools-assets/embedding.json').read_text()).get('adapter_version')!=4):
   verify(base,require_frontend=False)
   if not(base/'cef/devtools-assets').exists():prepare_frontend(base/'cef',base/'sdk-flat/include/cef_pack_resources.h')
   else:prepare_embedding(base/'cef/devtools-assets')
   record=json.loads((base/'codisseu-install.json').read_text());record['files'].update({str(p.relative_to(base/'cef')):digest(p)for p in sorted((base/'cef/devtools-assets').rglob('*'))if p.is_file()});temporary=base/'.codisseu-install.json.new';temporary.write_text(json.dumps(record,indent=2)+'\n');os.replace(temporary,base/'codisseu-install.json')
  verify(base);return
 if args.verify:raise RuntimeError('Prepare private CEF runtime explicitly first')
 if platform.system()!='Linux'or platform.machine()!='x86_64':raise RuntimeError('Pinned CEF target is Linux x86_64')
 if (base/'cef').exists()or(base/'sdk-flat').exists():raise RuntimeError('Existing unowned CEF tree preserved')
 downloads=base/'downloads';downloads.mkdir(parents=True,exist_ok=True);archive=downloads/source['archive']
 if not archive.exists():
  with urllib.request.urlopen(source['url'],timeout=180)as incoming,archive.open('xb')as output:
   count=0
   while block:=incoming.read(1024*1024):
    count+=len(block)
    if count>source['bytes']:raise RuntimeError('CEF archive exceeds pin')
    output.write(block)
 if archive.stat().st_size!=source['bytes']or digest(archive)!=source['sha256']:raise RuntimeError('CEF archive SHA256 mismatch')
 with tempfile.TemporaryDirectory(prefix='cef-prepare-',dir=base)as temporary:
  work=Path(temporary)
  with tarfile.open(archive,'r:bz2')as tar:
   members=tar.getmembers()
   if len(members)>30000 or sum(m.size for m in members)>4*1024**3:raise RuntimeError('CEF extraction limits exceeded')
   tar.extractall(work,filter='data')
  sdk=work/source['archive'].removesuffix('.tar.bz2');runtime=work/'runtime';runtime.mkdir();build=work/'sdk-flat';build.mkdir()
  for directory in ['Release','Resources']:
   for path in(sdk/directory).iterdir():
    if path.name=='chrome-sandbox':continue
    target=runtime/path.name
    if path.is_dir():shutil.copytree(path,target,symlinks=True)
    else:shutil.copy2(path,target)
  for name in ['LICENSE.txt','CREDITS.html','README.txt']:shutil.copy2(sdk/name,runtime/name)
  prepare_frontend(runtime,sdk/'include/cef_pack_resources.h')
  archive_record={'type':'minimal','name':source['archive'],'sha1':source['official_sha1']};(runtime/'archive.json').write_text(json.dumps(archive_record,indent=2)+'\n')
  # The bindings need the API-version header at build time; only runtime files
  # belong in cef/. SDK symlinks remain private and are not shipped to users.
  for path in runtime.iterdir():os.symlink('../cef/'+path.name,build/path.name,target_is_directory=path.is_dir())
  for name in ['include','cmake','libcef_dll']:shutil.copytree(sdk/name,build/name)
  for name in ['CMakeLists.txt','BUILD.bazel','MODULE.bazel','WORKSPACE']:shutil.copy2(sdk/name,build/name)
  for path in runtime.rglob('*'):
   if path.is_symlink()and not path.resolve().is_relative_to(runtime.resolve()):raise RuntimeError('CEF runtime link escape')
  files={str(p.relative_to(runtime)):digest(p)for p in sorted(runtime.rglob('*'))if p.is_file()}
  runtime.rename(base/'cef');build.rename(base/'sdk-flat');(base/'bin').mkdir(exist_ok=True)
  (base/'codisseu-install.json').write_text(json.dumps({'schema_version':1,'lock_sha256':digest(LOCK),'source':source,'license':'CEF BSD-3-Clause and Chromium third-party credits preserved unmodified','sandbox':'No setuid helper installed; requires Chromium user namespace sandbox, verified during acceptance','files':files},indent=2)+'\n')
 verify(base)
if __name__=='__main__':main()
