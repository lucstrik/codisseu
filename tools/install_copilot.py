#!/usr/bin/env python3
"""Install the pinned official Copilot server outside all project directories."""
import argparse,hashlib,json,os,platform,shutil,stat,tempfile,urllib.request,zipfile
from pathlib import Path
VERSION='1.565.6'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--destination',type=Path,default=Path.home()/'.local/share/codisseu/tools/copilot');a=p.parse_args()
operating={'Linux':'linux','Darwin':'darwin','Windows':'win32'}.get(platform.system());arch={'x86_64':'x64','AMD64':'x64','aarch64':'arm64','arm64':'arm64'}.get(platform.machine());assert operating and arch,'Unsupported platform'
name=f'copilot-language-server-{operating}-{arch}-{VERSION}.zip'
headers={'User-Agent':'Codisseu-Copilot-installer','Accept':'application/vnd.github+json'}
request=urllib.request.Request(f'https://api.github.com/repos/github/copilot-language-server-release/releases/tags/{VERSION}',headers=headers)
with urllib.request.urlopen(request,timeout=30) as response:release=json.load(response)
asset=next(x for x in release['assets'] if x['name']==name);assert asset['digest'].startswith('sha256:')
a.destination=a.destination.expanduser().resolve();a.destination.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='.copilot-install-',dir=a.destination) as temp:
    temp=Path(temp);archive=temp/'server.zip'
    with urllib.request.urlopen(asset['browser_download_url'],timeout=60) as response,archive.open('wb') as output:
        total=0
        while chunk:=response.read(65536):
            total+=len(chunk);assert total<=256*1024*1024,'Archive too large';output.write(chunk)
    digest=hashlib.file_digest(archive.open('rb'),'sha256').hexdigest();assert digest==asset['digest'].split(':',1)[1],'Checksum mismatch'
    files=temp/'files';files.mkdir()
    with zipfile.ZipFile(archive) as source:
        assert sum(f.file_size for f in source.infolist())<=512*1024*1024
        for f in source.infolist():
            target=files/f.filename
            assert target.resolve().is_relative_to(files.resolve()) and not stat.S_ISLNK(f.external_attr>>16),'Unsafe archive entry'
            source.extract(f,files)
    program=files/('copilot-language-server.exe' if operating=='win32' else 'copilot-language-server');assert program.is_file();program.chmod(0o755)
    license_url='https://raw.githubusercontent.com/github/copilot-language-server-release/main/LICENSE'
    with urllib.request.urlopen(license_url,timeout=30) as response:(files/'LICENSE').write_bytes(response.read(65536))
    record={'version':VERSION,'source':asset['browser_download_url'],'archive_sha256':digest,'license_source':license_url}
    (files/'codisseu-install.json').write_text(json.dumps(record,indent=2)+'\n')
    target=a.destination/VERSION
    if target.exists():
        # Existing running versions are retained; only the managed current link changes.
        assert (target/program.name).is_file(),'Existing installation incomplete'
        assert hashlib.file_digest((target/program.name).open('rb'),'sha256').digest()==hashlib.file_digest(program.open('rb'),'sha256').digest(),'Existing version differs; preserve it and choose another destination'
        shutil.copyfile(files/'LICENSE',target/'LICENSE')
        shutil.copyfile(files/'codisseu-install.json',target/'codisseu-install.json')
    else:os.replace(files,target)
    link=temp/'current';link.symlink_to(target,target_is_directory=True);os.replace(link,a.destination/'current')
print(str(a.destination/'current'/program.name))
