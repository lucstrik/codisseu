"""Prepare local, audited Rust resources for the Linux personal bundle.

No network, user profiles, Cargo cache, rustup configuration or project hooks.
"""
import hashlib,json,os,shutil,subprocess
from pathlib import Path
VERSION='1.97.0'
HOST='x86_64-unknown-linux-gnu'
TOOLS=['cargo','rustc','rustdoc','rustfmt','cargo-fmt','cargo-clippy','clippy-driver','rust-analyzer']

def command(args):return subprocess.run([str(x)for x in args],check=True,capture_output=True,text=True,timeout=120).stdout

def digest(path):
    with Path(path).open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()

def source_toolchain():
    rustup=shutil.which('rustup')
    if not rustup:raise RuntimeError('Prepare Rust1.97.0 with rust-analyzer/rust-src/rustfmt/clippy on the build machine first')
    root=Path(command([rustup,'run',VERSION,'rustc','--print','sysroot']).strip()).resolve(strict=True)
    version=command([root/'bin/rustc','-vV'])
    if f'release: {VERSION}\n'not in version or f'host: {HOST}\n'not in version:raise RuntimeError('Pinned Rust build toolchain mismatch')
    for name in TOOLS:
        if not(root/'bin'/name).is_file():raise RuntimeError('Missing Rust component: '+name)
    if not(root/'lib/rustlib/src/rust/library/core/src/lib.rs').is_file():raise RuntimeError('Install the pinned rust-src component on the build machine')
    return root,version

def prepare(repository,resources,out,notices,native_copy,owners):
    source,version=source_toolchain()
    private=resources/'runtime/rust';toolchain=private/'toolchain';(toolchain/'bin').mkdir(parents=True)
    records=[]
    for name in TOOLS:
        original=source/'bin'/name;destination=toolchain/'bin'/name
        shutil.copy2(original,destination);records.append({'path':'bin/'+name,'sha256':digest(original)})
    shutil.copytree(source/'lib',toolchain/'lib',symlinks=True)
    # Rust official libLLVM aliases are internal relative symlinks. Reject any
    # source/runtime link that would turn a private sysroot into an external read.
    for path in toolchain.rglob('*'):
        if path.is_symlink()and(not path.resolve().is_relative_to(toolchain.resolve())or os.path.isabs(os.readlink(path))):raise RuntimeError('Rust toolchain contains an escaping symlink')
    rust_notices=notices/'rust';rust_notices.mkdir()
    syntax_license=repository/'tools/portable_rust_notices/tree-sitter-rust-0.24.2-LICENSE'
    shutil.copy2(syntax_license,rust_notices/syntax_license.name)
    for name in ['COPYRIGHT.html','COPYRIGHT-library.html','README.md']:
        original=source/'share/doc/rust'/name
        if original.is_file():shutil.copy2(original,rust_notices/name)
    shutil.copytree(source/'share/doc/rust/licenses',rust_notices/'licenses')
    for name in ['multirust-channel-manifest.toml','multirust-config.toml','components','rust-installer-version']:
        original=source/'lib/rustlib'/name
        if original.is_file():shutil.copy2(original,rust_notices/name)
    debug_source=repository/'.tools/debug/codelldb'
    command(['python3',repository/'tools/install_rust_debugger.py','--verify'])
    debug=resources/'runtime/debug/codelldb'
    shutil.copytree(debug_source,debug)
    shutil.copy2(repository/'tools/install_rust_debugger.py',rust_notices/'install_rust_debugger.py')
    # Runtime C linking inputs are copied from known installed packages. These
    # are link-time files, not executable dependencies passed to ldd.
    linker=private/'linker';crt=linker/'crt';libs=linker/'lib'
    crt.mkdir(parents=True);libs.mkdir()
    gcc=Path(command(['cc','-print-libgcc-file-name']).strip()).parent
    def link_input(original,destination):
        original=Path(original).resolve(strict=True);shutil.copy2(original,destination)
        result=subprocess.run(['dpkg-query','-S',str(original)],capture_output=True,text=True)
        if result.returncode==0:
            for line in result.stdout.splitlines():owners[line.split(': ',1)[0]]=True
        records.append({'path':str(destination.relative_to(resources)),'sha256':digest(original),'source':str(original)})
    for name in ['Scrt1.o','crti.o','crtn.o']:
        link_input(Path(command(['cc','-print-file-name='+name]).strip()),crt/name)
    for name in ['crtbeginS.o','crtendS.o']:
        link_input(gcc/name,crt/name)
    for name in ['libgcc.a','libgcc_eh.a']:
        link_input(gcc/name,libs/name)
    for name in ['libc_nonshared.a','libutil.a','librt.a','libpthread.a','libdl.a']:
        link_input(Path('/usr/lib/x86_64-linux-gnu')/name,libs/name)
    # Rebase GNU linker scripts, never carry their absolute Debian filesystem
    # paths into an otherwise portable build environment.
    (libs/'libc.so').write_text('/* Codisseu relocation of the installed glibc link group. */\nGROUP ( libc.so.6 libc_nonshared.a AS_NEEDED ( ld-linux-x86-64.so.2 ) )\n')
    (libs/'libgcc_s.so').write_text('/* Codisseu relocation of the installed GCC link group. */\nGROUP ( libgcc_s.so.1 -lgcc )\n')
    (libs/'libm.so').write_text('/* Codisseu relocation of the installed glibc math link group. */\nGROUP ( libm.so.6 AS_NEEDED ( libmvec.so.1 ) )\n')
    for name in ['libc.so.6','libgcc_s.so.1','libm.so.6','libmvec.so.1','ld-linux-x86-64.so.2']:
        if not(out/'lib'/name).exists():native_copy(Path('/usr/lib/x86_64-linux-gnu')/name,out/'lib'/name)
    # The driver is original Rust source shipped with the bundle notices and
    # compiled only on the build machine. It calls the official rust-lld.
    driver_source=repository/'tools/portable_rust_linker.rs';driver=linker/'link-driver';temporary_driver=linker/'.link-driver-build'
    command([source/'bin/rustc','--edition=2024','-O',driver_source,'-o',temporary_driver])
    native_copy(temporary_driver,driver);temporary_driver.unlink()
    shutil.copy2(driver_source,rust_notices/'portable_rust_linker.rs')
    wrappers=private/'bin';wrappers.mkdir()
    prelude='''#!/bin/sh
set -eu
codisseu_rust=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
codisseu_resources=$(CDPATH= cd -- "$codisseu_rust/../.." && pwd)
codisseu_bundle=$(CDPATH= cd -- "$codisseu_resources/../.." && pwd)
export PATH="$codisseu_rust/bin:${PATH:-/usr/bin:/bin}"
'''
    for name in TOOLS:
        script=prelude+f'exec "$codisseu_rust/toolchain/bin/{name}" "$@"\n'
        (wrappers/name).write_text(script);(wrappers/name).chmod(0o755)
    loader=next((out/'lib').glob('ld-linux-*')).name
    script=prelude+'''export CODISSEU_RUST_LINKER_ROOT="$codisseu_rust/linker"
export CODISSEU_RUST_BUNDLE_ROOT="$codisseu_bundle"
'''+f'exec "$codisseu_bundle/lib/{loader}" --library-path "$codisseu_bundle/lib" "$codisseu_rust/linker/link-driver" "$@"\n'
    (wrappers/'cc').write_text(script);(wrappers/'cc').chmod(0o755)
    info={'version':VERSION,'host':HOST,'rustc_vV':version,'source':'Locally installed official rustup component distribution; no rustup HOME/config/cache included','tool_identities':records,'rust_src':str((toolchain/'lib/rustlib/src/rust/library').relative_to(resources)),'default_toolchain':'private1.97.0; explicit Cargo executable/configuration/target linker remains authoritative','native_linking':'Private rust-lld + Debian CRT/GCC runtime/glibc link inputs for std GNU x86_64 dynamic programs. Rebuild generated artifacts after moving/removing the bundle. Native C/C++ compilation and other targets need project toolchains.','codelldb':json.loads((debug/'codisseu-install.json').read_text())}
    info['syntax_grammar']={'name':'tree-sitter-rust','version':'0.24.2','license':'MIT','upstream':'https://github.com/tree-sitter/tree-sitter-rust','distribution':'crates.io','crate_sha256':'439e577dbe07423ec2582ac62c7531120dbfccfa6e5f92406f93dd271a120e45','notice':syntax_license.name,'notice_sha256':digest(syntax_license)}
    info['toolchain_selection']='Direct private Cargo/rustc do not select another version from rust-toolchain.toml alone. Explicit Cargo executable/RUSTC/target linker configuration is preserved; other versions require a separately prepared project toolchain.'
    (rust_notices/'sources.json').write_text(json.dumps(info,indent=2)+'\n')
    roles={name:'runtime/rust/bin/'+name for name in ['cargo','rustc','rustdoc','rustfmt','rust-analyzer']}
    roles['rust-linker']='runtime/rust/bin/cc'
    roles['rust-debug']='runtime/debug/codelldb/extension/adapter/codelldb'
    provider={'language':'Rust','program':'runtime/rust/bin/rust-analyzer','args':[]}
    return roles,provider,info
