#!/usr/bin/env python3
"""Prepare a reviewed, history-free public source snapshot (dry-run by default).

Reads only an explicit source allowlist. Never copies the working Git history,
profiles or local engines. --status-file supplies a new public status summary;
the private historical docs/status.md is never read. --write creates a new
output directory only after every file and link has been checked.

The scanner reports paths, kinds and line numbers, never matching values. It is
a conservative aid for review, not a guarantee that arbitrary data is public.
"""

from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass, field
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import tempfile
from urllib.parse import unquote, urlsplit


ROOT_FILES = frozenset({
    "README.md", "CONTRIBUTING.md", "SECURITY.md", "CODE_OF_CONDUCT.md", "AGENTS.md", "STORM_PLAN.md",
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".gitignore", ".gitattributes", "LICENSE",
    "LICENSE.md", "NOTICE", "NOTICE.md", "THIRD_PARTY_NOTICES.md",
})
SOURCE_TREES = (
    "apps", "crates", "assets", "vendor", "fixtures", "design", "reference/public", ".github",
)
TOOL_FILES = frozenset("""
build_deb.py test_deb_package.py release_notices.py verify_deb.py native_core_release.py
export_public_source.py test_export_public_source.py
build_linux_bundle.py verify_linux_bundle.py verify_cli_bundle.py
build_browser_engine.py install_browser_runtime.py browser-engine.lock.json
install_command_path.sh install_desktop.py update_desktop_binary.py
install_providers.py install_php_formatter.py install_python_tools.py
install_debug_adapters.py install_rust_debugger.py install_copilot.py
debug-adapters.json python-tools.lock.json python_test_runner.py php_format.php
portable_install.sh portable_notices.py portable_browser.py portable_python.py
portable_rust.py portable_rust_linker.rs dependency_inventory.py
test_cli_install.py test_provider_storage.py probe_provider_storage.py
db_metadata_fixtures.py db_sql_fixtures.py docker_fixture.py
grid_inspection_fixtures.py mongo_aggregate_fixture.py mongo_options_fixture.py
multi_project_mysql_fixture.py query_execution_fixtures.py
create_git_merge_demo.py verify_transfer_exports.py
native_smoke.py native_m2.py native_controls.py private_native_run.py native_drawings.py native_themes.py
prepare_app_icons.py prepare_ide_icons.py prepare_excalidraw_icons.py
""".split())
TOOL_TREES = (
    "tools/providers", "tools/browser_notices", "tools/portable_python_notices",
    "tools/portable_rust_notices",
)

# Only these reviewed classes of product documents can be followed. Links never
# authorize copying arbitrary docs, reports, receipts or reference evidence.
DOC_FILES = frozenset("""
architecture.md dependencies.md development.md optional-tools.md install-linux.md
cli-usage.md cli-launch-contract-20261111.md editor-shortcuts.md themes.md
laravel.md php-composer.md php-tests-usage.md php-tests-contract.md
js-tests-usage.md js-tests-contract.md rust-usage.md rust-support-contract.md
python-usage.md python-support-contract.md python-language-contract.md
python-runtime-contract.md python-tests-contract.md
m4-database-usage.md m4-data-transfer-contract.md m4-sql-workflow-contract.md
debugging.md m5-debug-contract.md m5-debug-backend.md
ai-chat-usage.md ai-chat-concurrency-contract.md ai-chat-access-contract.md
ai-chat-vscode-contract.md ai-chat-workspace-scope-contract.md
api-client-usage.md api-client-contract.md excalidraw-usage.md excalidraw-contract.md browser-usage.md browser-contract.md
browser-inspector-contract.md browser-frontend-inspector-contract.md
jira-usage.md jira-contract.md jira-timesheet-contract.md
quick-fixes-usage.md quick-fixes-contract.md local-history-usage.md
multi-project-workspace-usage.md multi-project-workspace-contract.md
providers.md provider-storage.md portable-runtime-contract.md runtime-release-contract.md
public-release-contract.md public-release-status.md debian-release-contract.md new-window-contract.md project-session-contract-20261113.md
project-database-scope-contract-20261113.md project-ai-http-scope-contract-20261113.md
contracts/database.md contracts/documents.md contracts/language.md contracts/ui.md
""".split())
DOC_SEEDS = frozenset({
    "architecture.md", "development.md", "optional-tools.md", "install-linux.md",
    "runtime-release-contract.md", "public-release-contract.md", "public-release-status.md", "debian-release-contract.md",
    "contracts/database.md", "contracts/documents.md", "contracts/language.md", "contracts/ui.md",
    "excalidraw-usage.md", "excalidraw-contract.md",
})
EXCLUDED_PARTS = {
    ".git", ".tools", "target", "artifacts", "dist", "node_modules", "__pycache__",
    ".secrets", ".ssh", ".gnupg", ".aws", ".config", ".cache", ".idea", ".vscode",
}
EXCLUDED_SUFFIXES = {
    ".pyc", ".pyo", ".key", ".p12", ".pfx", ".sqlite", ".sqlite3", ".db", ".log",
    ".zip", ".7z", ".xz", ".gz", ".deb", ".so", ".dll", ".exe", ".jar", ".jks",
}
PUBLIC_CERTIFICATES = {"crates/codisseu-db/certificates/aws-rds-roots.pem"}
INLINE_LINK = re.compile(r"(?P<image>!)?\[(?P<label>[^\]\n]*)\]\((?P<target>[^)\n]+)\)")
HTML_LINK = re.compile(r'(?P<prefix>\b(?:href|src)\s*=\s*["\'])(?P<target>[^"\']+)(?P<suffix>["\'])')
REFERENCE_LINK = re.compile(r"(?m)^\[[^\]\n]+\]:\s*<?([^\s>]+)>?")
CORPORATE_REPLACEMENTS = (
    ("Total" + " Express", "Empresa Exemplo", "replace-company-display-name"),
    ("Total" + "Express", "ExampleGroup", "replace-company-camel-name"),
    ("total" + "express", "example_company", "replace-company-identifier"),
    ("PSI" + "GEST", "EXAMPLE_DB", "replace-project-label"),
)
SECRET_PATTERNS = (
    ("private-key", re.compile(r"-----BEGIN (?:[A-Z0-9]+ )?PRIVATE KEY-----")),
    ("github-token", re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})\b")),
    ("aws-access-key", re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b")),
    ("slack-token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{20,}\b")),
    ("google-api-key", re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b")),
)
BASIC_AUTH = re.compile(r"\b[a-zA-Z][a-zA-Z0-9+.-]*://[^\s/@:'\"<>]+:[^\s/@'\"<>]+@")
PERSONAL_HOME = re.compile(r"/(?:home|Users)/lucas(?=/|\b)")
LOCAL_EMAIL = re.compile(r"(?i)\b[A-Z0-9._%+-]+@(?:" + "total" + "express|psi" + r"gest)\.[A-Z0-9.-]+\b")
MAX_FILE_BYTES = 24 * 1024 * 1024


class ExportError(ValueError):
    """A safe diagnostic containing only a kind and a relative public path."""

    def __init__(self, kind: str, path: str):
        self.kind = kind
        self.path = path
        super().__init__(f"{kind}: {path}")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def allowed_path(path: str) -> bool:
    parts = PurePosixPath(path).parts
    if not parts or ".." in parts or PurePosixPath(path).is_absolute():
        return False
    if any(p in EXCLUDED_PARTS for p in parts):
        return False
    name = parts[-1]
    lower = name.lower()
    if lower == ".env" or lower.startswith(".env."):
        return False
    if lower in {"credentials", "credentials.json", "secrets.json", "id_rsa", "id_ed25519"}:
        return False
    if lower.startswith("session.") and lower.endswith((".json", ".json.bak")):
        return False
    if Path(name).suffix.lower() in EXCLUDED_SUFFIXES:
        return False
    if Path(name).suffix.lower() == ".pem" and path not in PUBLIC_CERTIFICATES:
        return False
    return True


def read_regular(root: Path, relative: str) -> tuple[bytes, int]:
    path = root / relative
    current = root
    for part in PurePosixPath(relative).parts:
        current = current / part
        if current.is_symlink():
            raise ExportError("symlink-refused", relative)
    try:
        info = path.lstat()
        if not stat.S_ISREG(info.st_mode):
            raise ExportError("non-regular-file", relative)
        if info.st_size > MAX_FILE_BYTES:
            raise ExportError("file-too-large", relative)
        # O_NOFOLLOW also closes the common file-level symlink substitution race.
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        with os.fdopen(fd, "rb") as source:
            data = source.read(MAX_FILE_BYTES + 1)
        if len(data) > MAX_FILE_BYTES:
            raise ExportError("file-too-large", relative)
        return data, 0o755 if info.st_mode & 0o111 else 0o644
    except OSError as error:
        raise ExportError("file-read-failed", relative) from error


def tree_files(root: Path, tree: str):
    start = root / tree
    if not start.exists() and not start.is_symlink():
        return
    if start.is_symlink() or not start.is_dir():
        raise ExportError("symlink-or-invalid-tree", tree)
    for directory, dirs, files in os.walk(start, followlinks=False):
        base = Path(directory)
        for name in sorted(dirs):
            relative = (base / name).relative_to(root).as_posix()
            if not allowed_path(relative):
                dirs.remove(name)
            elif (base / name).is_symlink():
                raise ExportError("symlink-refused", relative)
        for name in sorted(files):
            relative = (base / name).relative_to(root).as_posix()
            if allowed_path(relative):
                yield relative


def local_target(origin: str, target: str) -> str | None:
    target = target.strip().strip("<>")
    parsed = urlsplit(target)
    if parsed.scheme or parsed.netloc or target.startswith(("#", "//")):
        return None
    raw = unquote(parsed.path)
    if not raw:
        return None
    if raw.startswith("/") or "\\" in raw:
        return "__outside_snapshot__"
    parts = list(PurePosixPath(origin).parent.parts)
    for part in PurePosixPath(raw).parts:
        if part == "..":
            if not parts:
                return "__outside_snapshot__"
            parts.pop()
        elif part != ".":
            parts.append(part)
    return "/".join(parts)


def link_targets(path: str, text: str):
    for match in INLINE_LINK.finditer(text):
        yield local_target(path, match["target"])
    for match in HTML_LINK.finditer(text):
        yield local_target(path, match["target"])
    for match in REFERENCE_LINK.finditer(text):
        yield local_target(path, match[1])


def scan_text(path: str, text: str) -> list[dict]:
    findings = []
    for kind, pattern in SECRET_PATTERNS:
        for match in pattern.finditer(text):
            finding_kind = kind
            severity = "block"
            # This exact invalid PEM is an inspected rejection test, not key
            # material. Any changed body or another occurrence still blocks.
            sentinel = "-----BEGIN " + "PRIVATE KEY-----\\nsecret\\n-----END " + "PRIVATE KEY-----"
            if kind == "private-key" and path == "crates/codisseu-db/src/tls_roots.rs" and text.startswith(sentinel, match.start()):
                finding_kind, severity = "private-key-test-sentinel-review", "review"
            findings.append({"path": path, "kind": finding_kind, "line": text.count("\n", 0, match.start()) + 1, "severity": severity})
    for match in BASIC_AUTH.finditer(text):
        synthetic_context = "/tests/" in path or path.startswith(("fixtures/", "tools/test_")) or path.endswith("_tests.rs")
        findings.append({"path": path, "kind": "url-userinfo-synthetic-review" if synthetic_context else "url-userinfo-review", "line": text.count("\n", 0, match.start()) + 1, "severity": "review"})
    for kind, pattern in (("personal-home-unredacted", PERSONAL_HOME), ("corporate-email", LOCAL_EMAIL)):
        if pattern.search(text):
            findings.append({"path": path, "kind": kind, "severity": "block"})
    return findings


def sanitize_text(path: str, text: str) -> tuple[str, list[dict]]:
    changes = []
    text, count = PERSONAL_HOME.subn("/home/usuario", text)
    if count:
        changes.append({"kind": "replace-personal-home", "count": count})
    for old, new, kind in CORPORATE_REPLACEMENTS:
        count = text.count(old)
        if count:
            text = text.replace(old, new)
            changes.append({"kind": kind, "count": count})
    return text, changes


def strip_unpublished_links(path: str, text: str, files: set[str]) -> tuple[str, list[dict]]:
    count = 0

    def inline(match):
        nonlocal count
        target = local_target(path, match["target"])
        if target is not None and target not in files:
            count += 1
            return match["label"]
        return match[0]

    def html(match):
        nonlocal count
        target = local_target(path, match["target"])
        if target is not None and target not in files:
            count += 1
            return match["prefix"] + "#" + match["suffix"]
        return match[0]

    def reference(match):
        nonlocal count
        target = local_target(path, match[1])
        if target is not None and target not in files:
            count += 1
            return match[0].replace(match[1], "#", 1)
        return match[0]

    text = INLINE_LINK.sub(inline, text)
    text = HTML_LINK.sub(html, text)
    text = REFERENCE_LINK.sub(reference, text)
    changes = [{"kind": "unlink-private-or-unpublished-local-target", "count": count}] if count else []
    return text, changes


def public_inventory(root: Path) -> bytes:
    intro = """# Componentes de terceiros

O Codisseu usa dependências Rust, fontes, ícones e outros componentes com suas
próprias licenças. O Cargo.lock fixa as versões dos pacotes. Os avisos e textos
de licença dos recursos incorporados estão em reference/public e vendor.

O .deb core inclui seu inventário e os avisos correspondentes em
/opt/codisseu/share/codisseu/notices. Motores externos preparados separadamente conservam suas
próprias obrigações; eles não acompanham esta primeira distribuição pública.

Este resumo não republica relatórios de instalações pessoais. O bloco abaixo é
o inventário declarativo dos metadados Cargo, quando disponível; não é uma
declaração de que todo pacote listado integra o executável Linux.

"""
    path = "docs/third-party-inventory.md"
    if (root / path).is_file():
        text = read_regular(root, path)[0].decode("utf-8")
        begin = "<!-- BEGIN CARGO METADATA INVENTORY -->"
        end = "<!-- END CARGO METADATA INVENTORY -->"
        if text.count(begin) == text.count(end) == 1:
            intro += text[text.index(begin):text.index(end) + len(end)] + "\n"
    return intro.encode()


@dataclass
class Snapshot:
    payloads: dict[str, bytes] = field(default_factory=dict)
    modes: dict[str, int] = field(default_factory=dict)
    sources: dict[str, str | None] = field(default_factory=dict)
    transformations: dict[str, list[dict]] = field(default_factory=dict)
    findings: list[dict] = field(default_factory=list)

    def manifest(self) -> dict:
        return {
            "schema_version": 1,
            "kind": "public-source-snapshot",
            "git_history_included": False,
            "profiles_or_private_engines_included": False,
            "files": [{"path": path, "sha256": digest(data), "source_sha256": self.sources[path], "bytes": len(data), "mode": oct(self.modes[path]), "transformations": self.transformations.get(path, [])} for path, data in sorted(self.payloads.items())],
            "scanner_findings": self.findings,
            "scanner_limit": "Pattern checks and explicit allowlists support review; they do not prove absence of every kind of private data.",
        }


def prepare(root: Path, public_status: bytes) -> Snapshot:
    root = root.resolve()
    paths = set()
    for path in ROOT_FILES:
        if (root / path).exists() or (root / path).is_symlink():
            paths.add(path)
    for tree in (*SOURCE_TREES, *TOOL_TREES):
        paths.update(tree_files(root, tree))
    for name in TOOL_FILES:
        path = "tools/" + name
        if (root / path).exists() or (root / path).is_symlink():
            paths.add(path)
    paths.update(tree_files(root, "docs/images"))
    snapshot = Snapshot()
    for path in sorted(paths):
        original, mode = read_regular(root, path)
        snapshot.payloads[path] = original
        snapshot.modes[path] = mode
        snapshot.sources[path] = digest(original)

    pending = list(DOC_SEEDS)
    for path, data in list(snapshot.payloads.items()):
        if path.endswith(".md"):
            for target in link_targets(path, data.decode("utf-8")):
                if target and target.startswith("docs/") and target[5:] in DOC_FILES:
                    pending.append(target[5:])
    while pending:
        name = pending.pop()
        path = "docs/" + name
        if path in snapshot.payloads or name not in DOC_FILES or not (root / path).exists():
            continue
        data, mode = read_regular(root, path)
        snapshot.payloads[path], snapshot.modes[path], snapshot.sources[path] = data, mode, digest(data)
        for target in link_targets(path, data.decode("utf-8")):
            if target and target.startswith("docs/") and target[5:] in DOC_FILES:
                pending.append(target[5:])

    public_milestones = """# Etapa vigente

M7 — estabilidade e distribuição, com uma primeira alpha core para Linux amd64.

Esta publicação é uma fatia de distribuição. Não encerra os marcos globais do
produto nem certifica todos os recursos, plataformas ou referências visuais.
O que foi validado e o que ainda está pendente estão em [status.md](status.md).
O plano de longo prazo permanece em [STORM_PLAN.md](../STORM_PLAN.md).
""".encode()
    public_permissions = """# Permissões e escopo público

Em 16/11/2026 o autor autorizou a preparação e publicação do Codisseu na sua
conta GitHub, incluindo o pacote Linux, documentação e as três imagens que
forneceu para o README. A escolha da licença do código é MIT.

Este snapshot usa a implementação do Codisseu e componentes públicos com
procedência e avisos de licença. Histórico Git anterior, perfis, credenciais,
projetos pessoais e evidências proprietárias de laboratório foram excluídos.

A autorização do autor não altera as licenças e os direitos de terceiros.
Este arquivo não concede permissão geral para redistribuir componentes de
IDEs proprietários. Consulte os avisos do pacote e o inventário de terceiros.
""".encode()
    generated = {
        "docs/status.md": public_status,
        "docs/milestones.md": public_milestones,
        "docs/third-party-inventory.md": public_inventory(root),
        "reference/permissions.md": public_permissions,
    }
    for path, data in generated.items():
        snapshot.payloads[path], snapshot.modes[path], snapshot.sources[path] = data, 0o644, None
        snapshot.transformations[path] = [{"kind": "new-public-summary", "count": 1}]
    selected = set(snapshot.payloads)
    for path, data in list(snapshot.payloads.items()):
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            # Binary assets are only accepted under reviewed source/asset trees.
            continue
        if LOCAL_EMAIL.search(text):
            snapshot.findings.append({"path": path, "kind": "corporate-email-before-sanitization", "severity": "block"})
        text, changes = sanitize_text(path, text)
        if path.endswith(".md"):
            text, link_changes = strip_unpublished_links(path, text, selected)
            changes.extend(link_changes)
        snapshot.payloads[path] = text.encode("utf-8")
        if changes:
            snapshot.transformations.setdefault(path, []).extend(changes)
        snapshot.findings.extend(scan_text(path, text))
    return snapshot


def write_snapshot(snapshot: Snapshot, output: Path) -> None:
    blocked = [f for f in snapshot.findings if f["severity"] == "block"]
    if blocked:
        raise ExportError("secret-review-required", blocked[0]["path"])
    if output.exists() or output.is_symlink():
        raise ExportError("output-already-exists", "output")
    parent = output.parent
    for path in (parent, *parent.parents):
        if path.is_symlink():
            raise ExportError("output-symlink-parent", "output")
    parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".public-source-staging-", dir=parent))
    try:
        for path, data in sorted(snapshot.payloads.items()):
            destination = staging / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
            destination.chmod(snapshot.modes[path])
        manifest = json.dumps(snapshot.manifest(), ensure_ascii=False, indent=2) + "\n"
        (staging / "PUBLIC_SOURCE_MANIFEST.json").write_text(manifest, encoding="utf-8")
        # A second check avoids replacing any existing user directory.
        if output.exists() or output.is_symlink():
            raise ExportError("output-already-exists", "output")
        staging.rename(output)
    finally:
        if staging.exists():
            shutil.rmtree(staging)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--status-file", type=Path, required=True, help="New reviewed public summary; private docs/status.md is forbidden")
    parser.add_argument("--output", type=Path, default=Path("artifacts/github-publication/source"))
    parser.add_argument("--write", action="store_true", help="Create snapshot; otherwise inspect in memory only")
    args = parser.parse_args(argv)
    try:
        if args.status_file.is_symlink() or args.status_file.resolve() == (args.root / "docs/status.md").resolve():
            raise ExportError("private-or-symlink-status-refused", "docs/status.md")
        if args.status_file.stat().st_size > 128 * 1024:
            raise ExportError("public-status-too-large", "docs/status.md")
        status = args.status_file.read_bytes()
        status.decode("utf-8")
        snapshot = prepare(args.root, status)
        counts = Counter(f["severity"] for f in snapshot.findings)
        result = {"mode": "write" if args.write else "dry-run", "files": len(snapshot.payloads), "bytes": sum(map(len, snapshot.payloads.values())), "transformed_files": len(snapshot.transformations), "scanner_counts": dict(counts), "findings": snapshot.findings}
        print(json.dumps(result, ensure_ascii=False, indent=2))
        if counts["block"]:
            return 2
        if args.write:
            write_snapshot(snapshot, args.output)
        return 0
    except (OSError, ValueError) as error:
        # Generic exceptions may contain sensitive absolute paths or source text.
        diagnostic = str(error) if isinstance(error, ExportError) else type(error).__name__
        print(json.dumps({"error": diagnostic}, ensure_ascii=False))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
