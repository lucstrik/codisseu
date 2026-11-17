#!/usr/bin/env python3
"""Build a relocatable, offline Linux x86_64 Codisseu bundle from audited tools.

Never reads user profiles, OS vault, projects or .secrets. No project hooks,
network downloads, npm/composer installation, or global runtime changes.
"""
import argparse, hashlib, json, os, platform, re, shlex, shutil, subprocess, tarfile
from pathlib import Path
import portable_rust
import portable_python
import portable_browser
ROOT=Path(__file__).resolve().parents[1]

def command(args):
    return subprocess.run([str(a)for a in args],check=True,text=True,capture_output=True,timeout=60).stdout

def digest(path):
    with path.open('rb')as stream:return hashlib.file_digest(stream,'sha256').hexdigest()

def package_icons(resources):
    provenance=ROOT/'assets/icons/provenance.json'
    source=json.loads(provenance.read_text())
    sizes=source.get('sizes')
    if not isinstance(sizes,list)or not sizes or any(type(size)is not int or not 16<=size<=4096 for size in sizes)or len(sizes)!=len(set(sizes))or 256 not in sizes:
        raise RuntimeError('Application icon provenance must declare unique PNG sizes including256')
    records={record['path']:record for record in source['files']}
    directory=resources/'icons';directory.mkdir()
    files=[]
    for size in sizes:
        relative=f'assets/icons/io.codisseu.Codisseu-{size}.png'
        original=ROOT/relative;header=original.read_bytes()[:24]
        if header[:8]!=b'\x89PNG\r\n\x1a\n'or header[12:16]!=b'IHDR'or int.from_bytes(header[16:20],'big')!=size or int.from_bytes(header[20:24],'big')!=size:
            raise RuntimeError('Invalid application PNG dimensions: '+relative)
        sha=digest(original)
        if records.get(relative,{}).get('sha256')!=sha:raise RuntimeError('Application icon provenance/hash mismatch: '+relative)
        destination=directory/f'{size}.png';shutil.copy2(original,destination)
        files.append({'size':size,'path':f'share/codisseu/icons/{size}.png','sha256':sha})
    shutil.copy2(provenance,directory/'provenance.json')
    return {'sizes':sizes,'files':files,'provenance':'share/codisseu/icons/provenance.json','provenance_sha256':digest(provenance)}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,default=ROOT/'target/portable-release/release/codisseu')
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--archive',action='store_true')
    args=parser.parse_args()
    assert platform.system()=='Linux'and platform.machine()=='x86_64','This bundle builder is Linux x86_64 only'
    binary=args.binary.resolve(strict=True);out=args.output.absolute()
    assert binary.is_file()and os.access(binary,os.X_OK)
    # Fail before producing an incomplete package. These resources are prepared
    # on the build machine; installation and first launch never download them.
    fixer=ROOT/'.tools/php-formatting/php-cs-fixer.phar'
    assert fixer.is_file() and digest(fixer)=='47231757eefdd08778e2bd047d98a63dfaad881006c8c1d1bc870d63be7f6a9b','Prepare the build with tools/install_php_formatter.py'
    for file in ['tools/php_format.php','reference/public/php-formatting/LICENSE','.tools/node_modules/typescript/lib/typescript.js','.tools/node_modules/typescript-language-server/lib/cli.mjs']:
        assert (ROOT/file).is_file(),f'Missing installation resource: {file}'
    php=Path(shutil.which('php')or'/usr/bin/php').resolve(strict=True)
    extension_root=Path(command([php,'-r','echo ini_get("extension_dir");']).strip())
    xdebug=extension_root/'xdebug.so'
    if not xdebug.is_file():raise RuntimeError('The build PHP requires an installed Xdebug 3 extension; no download or system change was attempted')
    xdebug_info=json.loads(command([php,'-n','-d',f'zend_extension={xdebug}','-d','xdebug.mode=off','-r','echo json_encode(["version" => phpversion("xdebug")]);']))
    if not str(xdebug_info['version']).startswith('3.'):raise RuntimeError('The build PHP requires a compatible Xdebug 3 extension')
    xdebug_owner=command(['dpkg-query','-S',xdebug]).strip().split(': ',1)[0]
    xdebug_copyright=Path('/usr/share/doc')/xdebug_owner.split(':')[0]/'copyright'
    if not xdebug_copyright.is_file():raise RuntimeError('Missing installed Xdebug redistribution copyright/license')
    xdebug_package=command(['dpkg-query','-W','-f','${binary:Package}\t${Version}\t${source:Package}\t${source:Version}',xdebug_owner]).strip().split('\t')
    if out.exists():raise RuntimeError('Output already exists; existing bundle preserved')
    out.mkdir(parents=True)
    resources=out/'share/codisseu';resources.mkdir(parents=True)
    (out/'libexec').mkdir();(out/'lib').mkdir();(out/'bin').mkdir()
    notices=resources/'notices';notices.mkdir()
    native={};owners={};native_owner_queries=set()
    def native_copy(source,destination):
        source=Path(source).resolve(strict=True);destination.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(source,destination)
        # ldd lists the complete transitive dependency graph of these trusted,
        # installed binaries/extensions. Never inspect executables from a project.
        for line in command(['ldd',source]).splitlines():
            match=re.search(r'=> (/\S+) \(',line)or re.match(r'\s*(/\S+) \(',line)
            if not match:continue
            dependency=Path(match[1]);name=dependency.name;target=out/'lib'/name
            if name in native and digest(dependency)!=native[name]['sha256']:raise RuntimeError('Conflicting native ABI: '+name)
            if not target.exists():shutil.copy2(dependency.resolve(),target)
            native[name]={'sha256':digest(target),'source':str(dependency.resolve())}
        for value in [source,*[Path(v['source'])for v in native.values()]]:
            if value in native_owner_queries:continue
            native_owner_queries.add(value)
            for query in [value,Path(str(value).replace('/usr/lib/','/lib/',1)),Path(str(value).replace('/usr/bin/','/bin/',1))]:
                result=subprocess.run(['dpkg-query','-S',str(query)],capture_output=True,text=True)
                if result.returncode==0:
                    for line in result.stdout.splitlines():owners[line.split(': ',1)[0]]=True
                    break
    native_copy(binary,out/'libexec/codisseu-bin')
    native_copy(php,out/'libexec/php')
    node=ROOT/'.tools/node-v24.22.0-linux-x64'
    native_copy(node/'bin/node',out/'libexec/node')
    php_root=resources/'runtime/php';(php_root/'extensions').mkdir(parents=True)
    extensions=['ctype','dom','fileinfo','iconv','mbstring','phar','posix','simplexml','tokenizer','xml','xmlreader','xmlwriter']
    for extension in extensions:native_copy(extension_root/f'{extension}.so',php_root/'extensions'/f'{extension}.so')
    native_copy(xdebug,php_root/'extensions/xdebug.so')
    shutil.copy2(xdebug_copyright,notices/'Xdebug-COPYRIGHT.txt')
    (notices/'Xdebug-ACKNOWLEDGMENTS.txt').write_text('This product includes Xdebug, freely available from <http://xdebug.org/>\nThis product includes software developed by Vulcan Logic (http://www.vl-srm.net/).\n')
    runtime=resources/'runtime'
    shutil.copytree(ROOT/'.tools/phpactor-2026.10.07.0',runtime/'phpactor')
    # Keep Phpactor's subprocesses on the private PHP wrapper too. PHP_BINARY
    # otherwise names the raw ELF and loses its private loader/extensions.
    php_patches=[]
    for source in sorted((runtime/'phpactor/lib').rglob('*.php')):
        text=source.read_text()
        if 'PHP_BINARY' not in text:continue
        relative=source.relative_to(runtime/'phpactor')
        original=notices/'phpactor-runtime-originals'/relative
        original.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(source,original)
        before=digest(source)
        text=text.replace('PHP_BINARY',"(\\getenv('CODISSEU_PHP_BINARY') ?: PHP_BINARY)")
        source.write_text(text)
        php_patches.append({'file':str(relative),'upstream_sha256':before,'patched_sha256':digest(source),'change':'Use installation wrapper for PHP subprocesses; original source and MIT license preserved'})
    (notices/'phpactor-portable-patch.json').write_text(json.dumps(php_patches,indent=2)+'\n')
    for name in ['typescript','typescript-language-server']:shutil.copytree(ROOT/'.tools/node_modules'/name,runtime/'node_modules'/name)
    for name in ['php','js']:shutil.copytree(ROOT/'.tools/debug'/name,runtime/'debug'/name)
    shutil.copy2(node/'LICENSE',notices/'Node-LICENSE.txt')
    shutil.copy2(ROOT/'tools/debug-adapters.json',notices/'debug-adapters-sources.json')
    shutil.copy2(ROOT/'tools/providers/npm-sources.json',notices/'npm-sources.json')
    shutil.copy2(ROOT/'.tools/manifest.json',notices/'provider-sources.json')
    shutil.copytree('/usr/share/common-licenses',notices/'debian-common-licenses')
    rust_roles,rust_provider,rust_info=portable_rust.prepare(ROOT,resources,out,notices,native_copy,owners)
    python_roles,python_provider,python_info=portable_python.prepare(ROOT,resources,notices)
    browser_roles,browser_info=portable_browser.prepare(ROOT,resources,out,notices,native_copy)
    packages=[]
    for package in sorted(owners):
        info=command(['dpkg-query','-W','-f','${binary:Package}\t${Version}\t${source:Package}\t${source:Version}\n',package]).strip().split('\t')
        packages.append(info)
        copyright=Path('/usr/share/doc')/package.split(':')[0]/'copyright'
        if copyright.is_file():shutil.copy2(copyright,notices/('debian-'+package.replace(':','-')+'-copyright.txt'))
    (notices/'debian-native-packages.json').write_text(json.dumps(packages,indent=2)+'\n')
    installed=json.loads(command([php,'-n','-r','echo json_encode(require $argv[1]);',runtime/'phpactor/vendor/composer/installed.php']))
    for entry in installed.get('versions',{}).values():entry.pop('install_path',None)
    installed.get('root',{}).pop('install_path',None)
    (notices/'phpactor-composer-packages.json').write_text(json.dumps(installed,indent=2)+'\n')
    audited=ROOT/'reference/public/portable-runtime'
    if audited.exists():shutil.copytree(audited,notices/'phpactor-audit')
    shutil.copytree(ROOT/'reference/public/ide-icons',notices/'public-ide-icons')
    shutil.copy2(ROOT/'crates/codisseu-db/certificates/source.json',notices/'aws-ca-source.json')
    icon_info=package_icons(resources)
    loader=next(name for name in native if name.startswith('ld-linux-'))
    def tool_wrapper(role,extra):
        directory=runtime/role/'bin';directory.mkdir(parents=True,exist_ok=True)
        script='''#!/bin/sh
set -eu
codisseu_resources=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
codisseu_root=$(CDPATH= cd -- "$codisseu_resources/../.." && pwd)
'''
        if role=='php':script+='export CODISSEU_PHP_BINARY="$codisseu_resources/runtime/php/bin/php"\nexport PHP_BINARY="$CODISSEU_PHP_BINARY"\n'
        if role=='node':script+='exec "$codisseu_root/libexec/node" "$@"\n'
        else:script+=f'exec "$codisseu_root/lib/{loader}" --library-path "$codisseu_root/lib" "$codisseu_root/libexec/{role}" '+extra+' "$@"\n'
        (directory/role).write_text(script);(directory/role).chmod(0o755)
    php_args='-n -d "extension_dir=$codisseu_resources/runtime/php/extensions" -d "zend_extension=$codisseu_resources/runtime/php/extensions/xdebug.so" '+' '.join('-d extension='+x for x in extensions)+' -d xdebug.mode=off'
    tool_wrapper('php',php_args);tool_wrapper('node','')
    original=json.loads((ROOT/'.tools/providers.json').read_text())
    extra=next(x for x in original[0]['args']if x.startswith('--config-extra='))
    manifest={'schema_version':1,'tools':{'php':'runtime/php/bin/php','node':'runtime/node/bin/node','php-debug':'runtime/debug/php/extension/out/phpDebug.js','js-debug':'runtime/debug/js/js-debug/src/dapDebugServer.js'},'providers':[{'language':'Php','program':'runtime/php/bin/php','args':['-d','xdebug.mode=off','@/runtime/phpactor/bin/phpactor',extra,'language-server']},{'language':'TypeScript','program':'runtime/node/bin/node','args':['@/runtime/node_modules/typescript-language-server/lib/cli.mjs','--stdio']}]}
    formatter=runtime/'php-formatting';formatter.mkdir()
    fixer=ROOT/'.tools/php-formatting/php-cs-fixer.phar'
    assert digest(fixer)=='47231757eefdd08778e2bd047d98a63dfaad881006c8c1d1bc870d63be7f6a9b','Run tools/install_php_formatter.py'
    shutil.copy2(fixer,formatter/'php-cs-fixer.phar')
    shutil.copy2(ROOT/'tools/php_format.php',formatter/'php_format.php')
    shutil.copytree(ROOT/'reference/public/php-formatting',notices/'php-formatting')
    manifest['tools'].update({'php-formatter':'runtime/php-formatting/php-cs-fixer.phar','php-formatter-helper':'runtime/php-formatting/php_format.php'})
    manifest['tools'].update(rust_roles)
    manifest['providers'].append(rust_provider)
    manifest['tools'].update(python_roles)
    manifest['providers'].append(python_provider)
    manifest['tools'].update(browser_roles)
    (resources/'runtime.json').write_text(json.dumps(manifest,indent=2)+'\n')
    launcher='''#!/bin/sh
set -eu
codisseu_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export CODISSEU_RESOURCE_DIR="$codisseu_root/share/codisseu"
'''+f'exec "$codisseu_root/lib/{loader}" --library-path "$codisseu_root/lib" "$codisseu_root/libexec/codisseu-bin" "$@"\n'
    (out/'bin/codisseu').write_text(launcher);(out/'bin/codisseu').chmod(0o755)
    shutil.copy2(ROOT/'tools/portable_install.sh',out/'install.sh');(out/'install.sh').chmod(0o755)
    shutil.copy2(ROOT/'tools/install_command_path.sh',out/'install-command-path.sh');(out/'install-command-path.sh').chmod(0o755)
    (out/'BUNDLE-ID').write_text('codisseu-0.1.0-linux-x86_64-'+digest(out/'libexec/codisseu-bin')[:12]+'\n')
    (out/'BUILD.json').write_text(json.dumps({'platform':'linux-x86_64','binary_sha256':digest(out/'libexec/codisseu-bin'),'php':command([php,'-n','-v']).splitlines()[0],'node':command([node/'bin/node','--version']).strip(),'extensions':extensions,'native_libraries':native,'private_installation':True,'host_requirements':['Linux x86_64 kernel compatible with bundled glibc','glibc2.28+ and compatible libstdc++ for the official private Node runtime','desktop Wayland/X11 and working graphics driver','OS credential service for remembered passwords'],'redistribution':'Licenses and package/source versions are included. Public redistribution requires corresponding-source fulfillment for LGPL components and complete vendor notice audit.'},indent=2)+'\n')
    build=json.loads((out/'BUILD.json').read_text())
    build['xdebug']={'version':xdebug_info['version'],'extension':'share/codisseu/runtime/php/extensions/xdebug.so','sha256':digest(php_root/'extensions/xdebug.so'),'source':str(xdebug.resolve()),'package':xdebug_package,'upstream':'https://xdebug.org/','provenance':'Locally installed Debian package; original copyright and package/source versions included','license':'Xdebug License 1.01 and notices for included components; see Xdebug-COPYRIGHT.txt','copyright_sha256':digest(notices/'Xdebug-COPYRIGHT.txt'),'default_mode':'off','debug_mode':'Enabled only by explicit runtime arguments or XDEBUG_MODE','downloads_required':False}
    build['formatters']={'php':{'version':'3.95.27','sha256':digest(fixer),'helper_sha256':digest(formatter/'php_format.php')},'sql':{'engine':'sqlformat0.5.0 compiled into Codisseu'},'javascript_typescript':{'typescript':json.loads((runtime/'node_modules/typescript/package.json').read_text())['version'],'language_server':json.loads((runtime/'node_modules/typescript-language-server/package.json').read_text())['version']}}
    build['rust']=rust_info
    build['python']=python_info
    build['browser']=browser_info
    build['application_icons']=icon_info
    build['downloads_on_install_or_first_use']=False
    build['command_line']={'command':'codisseu','positional_paths':True,'terminal_cwd':True,'empty_without_path':True,'detached_by_default':True,'wait_option':True,'automatic_user_path':True,'profile':'Independent window session; desktop restore unchanged'}
    (out/'BUILD.json').write_text(json.dumps(build,indent=2)+'\n')
    with (out/'README.txt').open('w') as readme:
        readme.write('Codisseu Linux x86_64\n\nUse bin/codisseu ou install.sh. A pasta completa e necessária; não copie\nsomente o executável. PHP, Node, Phpactor, TypeScript, servidor JS/TS,\nPHP CS Fixer, Xdebug 3, Rust1.97/Cargo/rustfmt/Clippy/rust-analyzer/rust-src,\nCodeLLDB e linker privado GNU std estão incluídos. Formatação SQL está\ncompilada na IDE. Instalação e primeiro uso não baixam ferramentas.\nCtrl+Alt+L formata apenas o arquivo ativo, mediante ação explícita.\nProjetos pedem confiança antes de iniciar motores; depois iniciam\nautomaticamente conforme a configuração existente.\n\nPerfis, credenciais e projetos pessoais não fazem parte do pacote.\nGit, Docker, versões/extensões específicas da aplicação e drivers gráficos\ncontinuam sendo ferramentas/requisitos do ambiente, conforme BUILD.json.\nLicenças e fontes de origem em share/codisseu/notices; limites de\nredistribuição registrados em BUILD.json. Para .deb, preservar esta árvore\ne apontar o launcher do sistema a bin/codisseu; não executar instaladores\nde dependências ou npm/composer em postinst.\n')
    with (out/'README.txt').open('a')as readme:
        readme.write('\nTerminal: o instalador registra codisseu no PATH do usuario. Abra um novo\nterminal se a pasta de comandos ainda nao estava no PATH. Use codisseu .\npara abrir a pasta atual, codisseu para janela sem projeto, ou codisseu\n"/caminho/com espacos". O terminal fica livre; --wait aguarda a janela\nencerrar. --help e --version funcionam sem interface grafica.\n')
        readme.write('\nPython privado3.13.18: stdlib/pip/venv, Basedpyright1.40.2, Ruff0.17.3,\ndebugpy1.8.22 e pytest9.2.0 estao incluidos. Outros interpretadores/venvs\nconfigurados explicitamente prevalecem. Dependencias do projeto podem\nexigir instalacao explicita; nenhuma e instalada ao abrir o projeto.\nVenvs existentes podem precisar ser recriados ao mover/remover Python base.\nPacote exercitado em Ubuntu22 x86_64; debugpy requer glibc2.34+.\n')
        readme.write('\nRust: a toolchain privada padrao e1.97.0 GNU x86_64; Cargo/rustfmt/Clippy\ne rust-analyzer funcionam sem rustup no host. Configuracoes explicitas\ndo projeto continuam prevalecendo. rust-toolchain.toml sozinho nao alterna\nversao: prepare/configure explicitamente outra toolchain quando necessario.\nO linker privado permite build/run/test\nde programas std sem cc do host e usa loader/glibc do prefixo atual;\nreconstrua artefatos depois de mover/remover a instalacao. Compilar C/C++\nem build scripts e targets/libs nativas especificas requer suas ferramentas.\nAbrir/analisar nao executa build scripts/proc macros nem baixa dependencias.\n')
    with (out/'README.txt').open('a')as readme:
        readme.write('\nInspetor frontend nativo: use o botao Inspecionar pagina (alvo) na barra\ndo navegador. O painel abaixo da pagina oferece Rede, Componentes / DOM,\nArmazenamento, Variaveis e Tokens; arraste o divisor para ajustar a altura.\nRecarregue apos abrir para capturar novas requests. Historico de rede\nlimitado a200entradas; corpos ate1MiB sao obtidos sob demanda, sem repetir\na request. Valores ficam ocultos por padrao: use Revelar valores para ver\ne os botoes de copia para enviar ao clipboard. Editar armazenamento altera\ncookies/localStorage/sessionStorage reais da origem ativa. Avaliar executa\nJavaScript na pagina ou no frame pausado. JWT e decodificado localmente,\nsem verificar assinatura/autenticidade. F12 continua abrindo o DevTools\noriginal; inspetor nativo, DevTools e debug do editor sao modos exclusivos\npor aba. Fechar/trocar de modo retoma JavaScript pausado e libera referencias.\nEm areas estreitas, a barra ocupa duas linhas e as abas usam DOM/Dados/JS.\nGuia completo de uso/limites: share/codisseu/docs/browser-usage.md.\n')
    with (out/'README.txt').open('a')as readme:
        readme.write('\nNavegador Chromium/CEF156 nativo incluido em processo Rust separado, com\nDevTools original embutido (DOM/Console/Network/Sources) e depuracao JS/TS\nmediante gesto explicito. O renderer exige namespaces de usuario/PID e\nseccomp Linux; ambientes que bloqueiam a sandbox falham com clareza.\nNenhuma opcao --no-sandbox e usada. Perfis sao privados e temporarios;\nCEF nao e carregado no processo da IDE. Versoes, licencas, fontes e o\nadaptador minimo de inicializacao DevTools constam em notices/browser.\n')
    # Bundle the current owned user guide, without repository-only evidence links.
    guide_source=ROOT/'docs/browser-usage.md'
    guide_text=guide_source.read_text().split('\nReferências e confiança:',1)[0].rstrip()+'\n'
    guide=resources/'docs/browser-usage.md'
    guide.parent.mkdir(parents=True,exist_ok=True)
    guide.write_text(guide_text)
    build['documentation']={'browser_usage':{'path':guide.relative_to(out).as_posix(),'sha256':digest(guide),'source':'docs/browser-usage.md','source_sha256':digest(guide_source),'adaptation':'Repository-only evidence/reference footer omitted; user instructions unchanged'}}
    (out/'BUILD.json').write_text(json.dumps(build,indent=2)+'\n')
    sums=[]
    for file in sorted(out.rglob('*')):
        if file.is_file():
            relative=file.relative_to(out).as_posix()
            if '\n'in relative or '\\'in relative:raise RuntimeError('Unexpected filename in pinned resources')
            sums.append(digest(file)+'  '+relative)
    (out/'MANIFEST.sha256').write_text('\n'.join(sums)+'\n')
    if args.archive:
        archive=out.with_suffix('.tar.xz')
        if archive.exists():raise RuntimeError('Archive already exists')
        with tarfile.open(archive,'w:xz',preset=1)as tar:tar.add(out,arcname=out.name)
        print(json.dumps({'bundle':str(out),'archive':str(archive),'sha256':digest(archive)}))
    else:print(out)
if __name__=='__main__':main()
