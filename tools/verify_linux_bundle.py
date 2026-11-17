#!/usr/bin/env python3
"""Move/install a bundle, then exercise private engines offline in Ubuntu 22.04.

Uses the already-installed mongo:6.0 image only as a Linux filesystem; no mongod
is run. No network, host project, credentials, keyring or source tree is mounted.
"""
import argparse,hashlib,json,os,shutil,subprocess,tempfile
import portable_browser_acceptance
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def run(args,**kwargs):return subprocess.run([str(a)for a in args],check=True,capture_output=True,text=True,timeout=kwargs.pop('timeout',180),**kwargs)
def verify_icons(bundle):
    metadata=json.loads((bundle/'BUILD.json').read_text())['application_icons']
    provenance_path=bundle/metadata['provenance']
    assert provenance_path.resolve(strict=True).is_relative_to(bundle.resolve())
    assert hashlib.sha256(provenance_path.read_bytes()).hexdigest()==metadata['provenance_sha256']
    provenance=json.loads(provenance_path.read_text())
    assert metadata['sizes']==provenance['sizes']and 256 in metadata['sizes']
    sources={record['path']:record for record in provenance['files']}
    assert sorted(file['size']for file in metadata['files'])==sorted(metadata['sizes'])
    for file in metadata['files']:
        size=file['size'];path=bundle/file['path']
        assert file['path']==f'share/codisseu/icons/{size}.png'
        assert path.resolve(strict=True).is_relative_to(bundle.resolve())
        raw=path.read_bytes()
        assert raw[:8]==b'\x89PNG\r\n\x1a\n'and raw[12:16]==b'IHDR'
        assert int.from_bytes(raw[16:20],'big')==size==int.from_bytes(raw[20:24],'big')
        assert hashlib.sha256(raw).hexdigest()==file['sha256']==sources[f'assets/icons/io.codisseu.Codisseu-{size}.png']['sha256']
    return {'sizes':metadata['sizes'],'provenance_sha256':metadata['provenance_sha256'],'all_png_dimensions_and_hashes_verified':True}
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--bundle',type=Path,required=True);parser.add_argument('--test-binary',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--rust-test-binary',type=Path);parser.add_argument('--browser-test-binary',type=Path);args=parser.parse_args()
    out=args.output.absolute();out.mkdir(parents=True,exist_ok=True)
    bundle=args.bundle.resolve(strict=True)
    moved=bundle.parent/(bundle.name+' moved installation')
    assert not moved.exists();bundle.rename(moved)
    report={'moved_from':str(bundle),'moved_to':str(moved),'network':'none','personal_data_mounted':False}
    try:
        report['container_image_id']=run(['docker','image','inspect','--format','{{.Id}}','mongo:6.0']).stdout.strip()
        report['application_icons']=verify_icons(moved)
        report['host_runtime_info']=json.loads(run([moved/'bin/codisseu','--runtime-info']).stdout)
        for role in ['php','node','php-debug','js-debug','php-formatter','php-formatter-helper']:
            assert report['host_runtime_info']['tools'][role].startswith(str(moved))
        for p in report['host_runtime_info']['providers']:assert p['program'].startswith(str(moved))
        has_rust=any(p['language']=='Rust' for p in report['host_runtime_info']['providers'])
        has_python=any(p['language']=='Python' for p in report['host_runtime_info']['providers'])
        has_browser='browser-engine'in report['host_runtime_info']['tools']
        if has_browser:
            assert args.browser_test_binary is not None,'A Browser bundle requires --browser-test-binary'
            assert len(report['host_runtime_info']['tools'])==19 and len(report['host_runtime_info']['providers'])==4
            for role in ['browser-engine','browser-runtime']:assert report['host_runtime_info']['tools'][role].startswith(str(moved))
            build=json.loads((moved/'BUILD.json').read_text());browser=build['browser']
            raw=moved/'share/codisseu/runtime/browser/bin/codisseu-browser-engine.bin'
            assert hashlib.sha256(raw.read_bytes()).hexdigest()==browser['engine']['engine_sha256']
            assert hashlib.sha256((moved/'libexec/codisseu-bin').read_bytes()).hexdigest()==build['binary_sha256']
            assert not(moved/'share/codisseu/runtime/browser/cef/chrome-sandbox').exists()
            assert browser['protocol']==2 and not browser['cef_linked_into_ide'] and not browser['downloads_required']
            notice=moved/'share/codisseu/notices/browser'
            assert json.loads((notice/'helper-cargo-notices.json').read_text())['complete']
            assert (notice/'CEF-CREDITS.html').is_file() and (notice/'cef-rs/DevTools-LICENSE.txt').is_file()
            assert (notice/'devtools-embedding.diff').is_file()
            report['browser_metadata_notices_verified']=True
            guide=build['documentation']['browser_usage']
            guide_path=moved/guide['path']
            assert guide_path.resolve(strict=True).is_relative_to(moved.resolve())
            with guide_path.open('rb')as stream:assert hashlib.file_digest(stream,'sha256').hexdigest()==guide['sha256']
            assert 'share/codisseu/docs/browser-usage.md'in(moved/'README.txt').read_text()
            report['browser_usage_guide_verified']=guide
        if has_python:
            for role in ['python','python-debug','ruff','python-test-runner']:
                assert report['host_runtime_info']['tools'][role].startswith(str(moved))
        if has_rust:
            assert args.rust_test_binary is not None,'A Rust bundle requires --rust-test-binary for native Rust acceptance'
            for role in ['cargo','rustc','rustdoc','rustfmt','rust-analyzer','rust-debug','rust-linker']:
                assert report['host_runtime_info']['tools'][role].startswith(str(moved))
        profile=out/'install-fixture';profile.mkdir()
        bins=profile/'bin';bins.mkdir();data=profile/'data';data.mkdir()
        (profile/'home').mkdir()
        env={**os.environ,'HOME':str(profile/'home'),'XDG_DATA_HOME':str(data),'XDG_CONFIG_HOME':str(profile/'config'),'CODISSEU_BIN_DIR':str(bins)}
        installed=profile/'Installed Codisseu with spaces'
        foreign=bins/'codisseu';foreign.write_text('foreign fixture; preserve me\n');foreign.chmod(0o755)
        rejected=subprocess.run([str(moved/'install.sh'),'--prefix',str(installed)],env=env,capture_output=True,text=True)
        assert rejected.returncode!=0 and foreign.read_text()=='foreign fixture; preserve me\n'and not installed.exists()
        foreign.unlink()
        run([moved/'install.sh','--prefix',installed],env=env)
        run([moved/'install.sh','--prefix',installed],env=env)
        info=json.loads(run([bins/'codisseu','--runtime-info'],env=env).stdout)
        assert info['resources'].startswith(str(installed));report['install_and_idempotence']=True;report['foreign_launcher_preserved']=True
        assert verify_icons(installed)==report['application_icons']
        desktop=(data/'applications/io.codisseu.Codisseu.desktop').read_text()
        assert [line for line in desktop.splitlines()if line.startswith('Icon=')]==['Icon='+str(installed/'share/codisseu/icons/256.png')]
        report['application_icons']['desktop_points_to_installed_256_png']=True
        command=['docker','run','--rm','--pull','never','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--pids-limit','256','--memory','1536m','--cpus','2','--user',f'{os.getuid()}:{os.getgid()}','--tmpfs',f'/tmp:rw,exec,nosuid,size=768m,uid={os.getuid()},gid={os.getgid()},mode=1777','--mount',f'type=bind,src={moved},dst=/opt/Codisseu moved,readonly','--mount',f'type=bind,src={args.test_binary.resolve()},dst=/test-engines,readonly','--mount',f'type=bind,src={out},dst=/evidence','-e','HOME=/tmp','-e','XDG_CACHE_HOME=/tmp/cache','-e','XDG_CONFIG_HOME=/tmp/config','-e','CODISSEU_RESOURCE_DIR=/opt/Codisseu moved/share/codisseu','-e','CODISSEU_PORTABLE_TEST_REPORT=/evidence/offline-engines.json','--entrypoint','/bin/sh','mongo:6.0','-ec',r'''
if command -v php; then echo "Unexpected host PHP"; exit 1; fi
if command -v node; then echo "Unexpected host Node"; exit 1; fi
cat /etc/os-release > /evidence/container-os-release.txt
"/opt/Codisseu moved/bin/codisseu" --runtime-info > /evidence/container-runtime-info.json
"/opt/Codisseu moved/share/codisseu/runtime/php/bin/php" --version
"/opt/Codisseu moved/share/codisseu/runtime/php/bin/php" -r 'echo json_encode(["version" => phpversion("xdebug"), "modes" => xdebug_info("mode")]);' > /evidence/xdebug-default.json
"/opt/Codisseu moved/share/codisseu/runtime/php/bin/php" -d xdebug.mode=debug -d xdebug.start_with_request=no -r 'echo json_encode(["version" => phpversion("xdebug"), "modes" => xdebug_info("mode")]);' > /evidence/xdebug-debug.json
"/opt/Codisseu moved/share/codisseu/runtime/node/bin/node" --version
printf '<?php echo "portable";\n' >/tmp/portable.php
"/opt/Codisseu moved/bin/codisseu" --check-file /tmp/portable.php
"/opt/Codisseu moved/lib/ld-linux-x86-64.so.2" --library-path "/opt/Codisseu moved/lib" /test-engines --ignored --test-threads=1 --nocapture
''']
        if has_rust:
            extra=['--security-opt','seccomp=unconfined','--mount',f'type=bind,src={args.rust_test_binary.resolve()},dst=/test-rust,readonly','-e','CODISSEU_PORTABLE_RUST_TEST_REPORT=/evidence/offline-rust.json']
            command[command.index('--entrypoint'):command.index('--entrypoint')]=extra
            command[-1]+='\nif command -v rustc || command -v cargo || command -v cc; then echo Unexpected-host-Rust-or-cc; exit 1; fi\n"/opt/Codisseu moved/lib/ld-linux-x86-64.so.2" --library-path "/opt/Codisseu moved/lib" /test-rust --ignored --test-threads=1 --nocapture\n'
        if has_python:
            extra=['--mount',f'type=bind,src={ROOT/"tools/portable_python_acceptance.py"},dst=/test-python.py,readonly','-e','CODISSEU_PORTABLE_PYTHON_TEST_REPORT=/evidence/offline-python.json']
            command[command.index('--entrypoint'):command.index('--entrypoint')]=extra
            command[-1]+='\nif command -v python || command -v python3; then echo Unexpected-host-Python; exit 1; fi\n"/opt/Codisseu moved/share/codisseu/runtime/python/cpython/bin/python3" -I -B /test-python.py\n'
        with (out/'offline-container.log').open('w')as log:result=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,timeout=300)
        report['container_exit']=result.returncode;report['container_image']='mongo:6.0 filesystem; Ubuntu22.04, no Mongo server launched';report['no_host_php_node']=True
        report['offline_test']=json.loads((out/'offline-engines.json').read_text())if(out/'offline-engines.json').exists()else None
        if result.returncode==0:
            os_release=(out/'container-os-release.txt').read_text()
            assert 'ID=ubuntu\n'in os_release and 'VERSION_ID="22.04"'in os_release
            report['container_os_release_verified']=True
            assert set(report['offline_test']['offline_formatters'])=={'format.php','format.sql','format.js','format.ts'}
            assert report['offline_test']['formatter_paths_confined']
            for check in ['php_launch_default_private_runtime','php_launch_verified_breakpoint','php_launch_evaluate','php_launch_stop','typescript_private_compiler','typescript_source_map_breakpoint','typescript_evaluate','typescript_stop']:
                assert report['offline_test'][check],f'Private debugger check failed: {check}'
            xdebug_default=json.loads((out/'xdebug-default.json').read_text())
            xdebug_debug=json.loads((out/'xdebug-debug.json').read_text())
            assert str(xdebug_default['version']).startswith('3.') and xdebug_default['modes']==[]
            assert xdebug_debug['version']==xdebug_default['version'] and 'debug' in xdebug_debug['modes']
            build=json.loads((moved/'BUILD.json').read_text())
            assert build['xdebug']['version']==xdebug_debug['version'] and build['xdebug']['default_mode']=='off'
            assert hashlib.sha256((moved/build['xdebug']['extension']).read_bytes()).hexdigest()==build['xdebug']['sha256']
            assert hashlib.sha256((moved/'share/codisseu/notices/Xdebug-COPYRIGHT.txt').read_bytes()).hexdigest()==build['xdebug']['copyright_sha256']
            assert 'This product includes Xdebug' in (moved/'share/codisseu/notices/Xdebug-ACKNOWLEDGMENTS.txt').read_text()
            report['private_xdebug']={'default':xdebug_default,'explicit_debug':xdebug_debug,'metadata_version_matches':True}
        if has_rust and result.returncode==0:
            grammar=build['rust']['syntax_grammar']
            assert grammar['name']=='tree-sitter-rust'and grammar['version']=='0.24.2'and grammar['license']=='MIT'
            grammar_notice=moved/'share/codisseu/notices/rust'/grammar['notice']
            assert hashlib.sha256(grammar_notice.read_bytes()).hexdigest()==grammar['notice_sha256']
            assert 'Copyright (c) 2017 Maxim Sokolov'in grammar_notice.read_text()
            assert 'rust-toolchain.toml'in build['rust']['toolchain_selection']
            report['rust_grammar_notice_verified']=True
            rust=json.loads((out/'offline-rust.json').read_text())
            for check in ['explicit_toolchain_configuration','cargo_check','cargo_build','clippy','rustdoc','rustfmt','analyzer_hover','stdlib_definition','quick_fix_preview_apply_undo','real_inlay_type_parameter','rust_rename_preview_apply_undo','unsupported_macro_rename_rejected','rust_native_breakpoint','rust_evaluate','rust_continue_exit0','sources_unchanged']:
                assert rust[check],f'Private Rust check failed: {check}'
            assert 'result=84' in rust['cargo_run'] and '1 passed' in rust['cargo_test']
            report['offline_rust']=rust
            report['no_host_rust_or_cc']=True
            report['ptrace_scope']='Unconfined seccomp only in the disposable no-network native-debug test container; cap-drop ALL retained'
        if has_python and result.returncode==0:
            icon_notices=moved/'share/codisseu/notices/public-ide-icons'
            python_icon=next(icon for icon in json.loads((icon_notices/'sources.json').read_text())['icons']if icon['name']=='file-python')
            assert python_icon['license']=='Apache-2.0'
            assert hashlib.sha256((icon_notices/'file-python.svg').read_bytes()).hexdigest()==python_icon['svg_sha256']
            assert (icon_notices/'intellij-community-LICENSE.txt').is_file()and(icon_notices/'intellij-community-NOTICE.txt').is_file()
            report['python_icon_notice_verified']=True
            compiled=build['python']['compiled_dependencies']
            assert {(d['name'],d['version'])for d in compiled}=={('tree-sitter-python','0.25.0'),('unicode-normalization','0.1.26')}
            for dependency in compiled:
                for file in dependency['files']:
                    path=moved/'share/codisseu/notices/python/compiled-dependencies'/file['path']
                    assert path.resolve(strict=True).is_relative_to(moved)
                    assert hashlib.sha256(path.read_bytes()).hexdigest()==file['sha256']
            report['python_compiled_notices_verified']=True
            python=json.loads((out/'offline-python.json').read_text())
            assert python['result']=='passed'and not python['network_required']
            for check in ['private_stdlib_ssl_sqlite_ctypes','venv_pip','venv_without_debugpy','offline_own_dependency','run_unicode_literal_arguments','pytest_pass_fail_skip','unittest_pass_fail_skip','structured_runner_both_frameworks_and_selected','all_case_source_paths_and_lines','ruff_format_buffer_preserved','ruff_safe_fix','sources_unchanged']:
                assert python[check],f'Private Python check failed: {check}'
            assert all(python['basedpyright'].values())and all(python['debugpy'].values())
            report['offline_python']=python;report['no_host_python']=True
        if has_browser and result.returncode==0:
            report['offline_browser']=portable_browser_acceptance.verify(moved,args.browser_test_binary,out/'offline-browser')
        report['result']='passed'if result.returncode==0 else'failed'
        assert result.returncode==0,'Offline engine test failed; see offline-container.log'
    finally:
        (out/'verification.json').write_text(json.dumps(report,indent=2)+'\n')
    print(moved)
if __name__=='__main__':main()
