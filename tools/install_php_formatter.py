#!/usr/bin/env python3
"""Install a pinned Codisseu-owned formatter; no project or Composer hooks."""
import hashlib
from pathlib import Path
import urllib.request
VERSION = '3.95.27'
SHA256 = '47231757eefdd08778e2bd047d98a63dfaad881006c8c1d1bc870d63be7f6a9b'
URL = f'https://github.com/PHP-CS-Fixer/PHP-CS-Fixer/releases/download/v{VERSION}/php-cs-fixer.phar'
ROOT = Path(__file__).resolve().parents[1]
def main():
    target = ROOT/'.tools/php-formatting/php-cs-fixer.phar'
    if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() == SHA256:
        print(f'PHP formatter {VERSION} already verified')
        return
    with urllib.request.urlopen(URL, timeout=60) as response:
        data = response.read(8*1024*1024+1)
    assert len(data) <= 8*1024*1024 and hashlib.sha256(data).hexdigest() == SHA256
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_suffix('.download')
    temporary.write_bytes(data)
    temporary.replace(target)
    print(f'PHP formatter {VERSION} installed and verified')
if __name__ == '__main__':
    main()
