#!/usr/bin/env python3
"""Owned Docker fixtures from installed BusyBox; no pull, mounts or external network.

The wrapper used by native tests limits inventory to our label and mutations to
our captured IDs. It delegates to the real Docker executable; it is not a mock.
"""
import argparse,hashlib,io,json,os,re,subprocess,tarfile,uuid
from pathlib import Path
DOCKER='/usr/bin/docker'

def cli(context,*args,**kwargs):
    return subprocess.run([DOCKER,'--context',context,*args],check=True,capture_output=True,timeout=45,**kwargs)

def create(path):
    assert not path.exists()
    context=subprocess.check_output([DOCKER,'context','show'],text=True,timeout=10).strip()
    token=uuid.uuid4().hex[:12]
    label='org.codisseu.fixture='+token
    image='codisseu-fixture/docker:'+token
    contents=io.BytesIO(); provenance=[]
    with tarfile.open(fileobj=contents,mode='w') as archive:
        files=[Path('/usr/bin/busybox')]
        dependencies=subprocess.check_output(['ldd',str(files[0])],text=True)
        files += [Path(p) for p in re.findall(r'(/[^\s()]+)',dependencies)]
        for source in dict.fromkeys(files):
            data=source.read_bytes();info=tarfile.TarInfo(str(source).lstrip('/'));info.mode=0o755;info.size=len(data);archive.addfile(info,io.BytesIO(data))
            provenance.append({'source':str(source),'sha256':hashlib.sha256(data).hexdigest()})
        for name in ['sh','sleep','ls','cat','stty','mkdir','rm','touch','wc']:
            info=tarfile.TarInfo('bin/'+name);info.type=tarfile.SYMTYPE;info.linkname='/usr/bin/busybox';info.mode=0o777;archive.addfile(info)
        for name in ['tmp','work']:
            info=tarfile.TarInfo(name);info.type=tarfile.DIRTYPE;info.mode=0o777 if name=='tmp' else 0o755;archive.addfile(info)
    ids=[]
    try:
        image_id=cli(context,'image','import','--change','LABEL '+label,'-',image,input=contents.getvalue()).stdout.decode().strip()
        for role in ['api','worker','remove']:
            name='codisseu-docker-test-'+role+'-'+token
            result=cli(context,'container','create','--label',label,'--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--pids-limit','64','--memory','64m','--tmpfs','/tmp:rw,noexec,nosuid,size=8m','--name',name,image,'sh','-c',"trap 'exit 0' TERM; printf 'Codisseu Docker fixture ready\\n'; while :; do sleep 1; done")
            ids.append({'id':result.stdout.decode().strip(),'name':name,'role':role})
        record={'context':context,'label':label,'image':image,'image_id':image_id,'containers':ids,'provenance':provenance}
        path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(record,indent=2)+'\n');path.chmod(0o600)
        return record
    except Exception:
        for item in ids:
            subprocess.run([DOCKER,'--context',context,'container','rm','-f',item['id']],capture_output=True,timeout=15)
        subprocess.run([DOCKER,'--context',context,'image','rm',image],capture_output=True,timeout=15)
        raise

def cleanup(path):
    record=json.loads(path.read_text());context=record['context'];results=[]
    for item in record['containers']:
        # Only an ID in this owned manifest, with its matching fixture label, can be removed.
        result=subprocess.run([DOCKER,'--context',context,'container','inspect','--format','{{json .Config.Labels}}',item['id']],capture_output=True,text=True,timeout=15)
        if result.returncode:results.append({'id':item['id'],'already_absent':True});continue
        labels=json.loads(result.stdout);key,value=record['label'].split('=',1);assert labels.get(key)==value
        cli(context,'container','rm','-f',item['id']);results.append({'id':item['id'],'removed':True})
    result=cli(context,'image','inspect','--format','{{json .Config.Labels}}',record['image']);labels=json.loads(result.stdout);key,value=record['label'].split('=',1);assert labels.get(key)==value
    cli(context,'image','rm',record['image'])
    return {'containers':results,'image_removed':True}

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('action',choices=['create','cleanup']);p.add_argument('manifest',type=Path);a=p.parse_args()
    print(json.dumps(create(a.manifest) if a.action=='create' else cleanup(a.manifest),indent=2))
