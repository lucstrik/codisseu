"""Copy verified installation-owned Python resources; no network or project hooks."""
import json,shutil,subprocess
from pathlib import Path

def prepare(repository,resources,notices):
    subprocess.run(['python3',str(repository/'tools/install_python_tools.py'),'--verify'],check=True,capture_output=True,text=True,timeout=120)
    source=repository/'.tools/python';target=resources/'runtime/python'
    shutil.copytree(source,target,symlinks=True,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
    runner=repository/'tools/python_test_runner.py'
    if not runner.is_file():raise RuntimeError('Python test runner missing from build source')
    shutil.copy2(runner,target/'python_test_runner.py')
    for path in target.rglob('*'):
        if path.is_symlink()and not path.resolve(strict=True).is_relative_to(target.resolve()):raise RuntimeError('Python resource symlink escapes installation')
    shutil.copytree(source/'NOTICES',notices/'python')
    shutil.copytree(repository/'tools/portable_python_notices',notices/'python/compiled-dependencies')
    shutil.copy2(repository/'tools/install_python_tools.py',notices/'python/install_python_tools.py')
    info=json.loads((source/'codisseu-install.json').read_text())
    roles={'python':'runtime/python/cpython/bin/python3','python-debug':'runtime/python/cpython/lib/python3.13/site-packages/debugpy/adapter/__main__.py','ruff':'runtime/python/cpython/bin/ruff'}
    roles['python-test-runner']='runtime/python/python_test_runner.py'
    provider={'language':'Python','program':'runtime/node/bin/node','args':['@/runtime/python/node_modules/basedpyright/langserver.index.js','--stdio']}
    compiled=json.loads((repository/'tools/portable_python_notices/sources.json').read_text())
    return roles,provider,{'versions':info['versions'],'lock_sha256':info['lock_sha256'],'sources':info['sources'],'compiled_dependencies':compiled,'private_installation':True,'downloads_on_install_or_first_use':False,'target_interpreter':'Explicit project interpreter/venv preserved. Bundled default does not select versions from .python-version or pyproject alone.','virtual_environments':'Create after installing/moving the Python base; existing venvs may need recreation after moving/removing it. pip/wheels included, installations only upon explicit user action.','host_requirement':'Linux x86_64; debugpy CPython3.13 wheel requires glibc2.34+. Ubuntu22offline exercised; no guarantee for all distributions/native project dependencies.'}
