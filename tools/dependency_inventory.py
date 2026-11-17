#!/usr/bin/env python3
"""Report declared Cargo licenses; never claims to audit source-file licensing.

The default writes Markdown to stdout. --write replaces only the generated block
in docs/third-party-inventory.md; --check verifies that block without writing.
Cargo metadata uses the existing lockfile and does not compile/run project code.
"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
DOCUMENT = ROOT / "docs" / "third-party-inventory.md"
START = "<!-- BEGIN CARGO METADATA INVENTORY -->"
END = "<!-- END CARGO METADATA INVENTORY -->"


def cell(value):
    return str(value if value is not None else "não declarado").replace("|", "\\|").replace("\n", " ")


def inventory():
    lock = ROOT / "Cargo.lock"
    if not lock.is_file():
        raise ValueError("Cargo.lock ausente: resolva dependências explicitamente antes do inventário")
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        timeout=120,
    )
    metadata = json.loads(result.stdout)
    packages = sorted(
        (p for p in metadata["packages"] if p["source"] is not None),
        key=lambda p: (p["name"], p["version"]),
    )
    missing = [p for p in packages if not p["license"]]
    special = [
        p for p in packages
        if any(token in (p["license"] or "") for token in
               ("OFL", "Ubuntu", "Unicode", "GPL", "MPL", "LicenseRef"))
    ]
    lines = [
        START,
        "",
        "## Inventário gerado dos metadados Cargo",
        "",
        f"Cargo.lock SHA-256: `{hashlib.sha256(lock.read_bytes()).hexdigest()}`.",
        f"Pacotes terceiros: **{len(packages)}**. Campo `license` ausente: **{len(missing)}**.",
        "",
        "Origem: `cargo metadata --locked --format-version 1`, workspace completo, sem filtro de plataforma. Inclui dependências de testes, build, outros alvos e pacotes resolvidos que podem não estar no executável Linux. Não é uma lista de componentes comprovadamente distribuídos nem auditoria manual de todos os arquivos. Checksums dos pacotes estão no Cargo.lock.",
        "",
        "Expressões com fontes, Unicode, copyleft ou licença customizada requerem atenção específica:",
        "",
    ]
    lines.extend(f"- `{p['name']} {p['version']}`: `{cell(p['license'])}`." for p in special)
    lines.extend(f"- LICENÇA AUSENTE: `{p['name']} {p['version']}`, license_file: `{cell(p['license_file'])}`." for p in missing)
    lines.extend([
        "",
        "A presença de `OR GPL`/`OR LGPL` em uma expressão com alternativa permissiva não implica escolher GPL/LGPL. Expressões legadas com `/` são reproduzidas sem reinterpretá-las. A seleção e os avisos necessários devem ser registrados ao empacotar.",
        "",
        "| Pacote / origem da versão | Licença declarada | Rust mínimo declarado |",
        "|---|---|---|",
    ])
    for package in packages:
        name, version = package["name"], package["version"]
        if package["source"].startswith("registry+"):
            origin = f"[{name} {version}](https://crates.io/crates/{name}/{version})"
        else:
            origin = f"`{cell(name)} {cell(version)}` ({cell(package['source'])})"
        lines.append(f"| {origin} | `{cell(package['license'])}` | {cell(package['rust_version'])} |")
    lines.extend(["", END])
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        generated = inventory()
        if not (args.write or args.check):
            print(generated)
            return 0
        original = DOCUMENT.read_text(encoding="utf-8")
        if original.count(START) != 1 or original.count(END) != 1:
            raise ValueError("O documento precisa de exatamente um par de marcadores do inventário")
        begin = original.index(START)
        end = original.index(END) + len(END)
        if begin >= end:
            raise ValueError("Marcadores do inventário estão fora de ordem")
        updated = original[:begin] + generated + original[end:]
        if args.check:
            if original != updated:
                print("Inventário desatualizado; revise as alterações e execute --write", file=sys.stderr)
                return 1
            print("Inventário confere com Cargo.lock e cargo metadata")
        else:
            DOCUMENT.write_text(updated, encoding="utf-8")
            print(f"Atualizado: {DOCUMENT}")
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Falha no inventário: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
