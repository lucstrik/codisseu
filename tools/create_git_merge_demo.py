#!/usr/bin/env python3
"""Create an isolated, repeatable Git merge exercise; never overwrite a directory."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap

DEMO_ID = "codisseu-git-merge-demo-v1"
DEFAULT_DESTINATION = Path.home() / "Projetos/codisseu-conflitos-demo"
CONFLICT_PATHS = {"src/Checkout.php", "web/etapas.js", "sql/clientes.sql"}

PHP_BASE = """<?php

declare(strict_types=1);

function tituloCheckout(): string
{
    return 'Entrega padrão';
}

// Contexto comum: estes comentários não devem desaparecer na resolução.
// O pedido usa somente dados fictícios deste laboratório.
// As funções ficam separadas para exercitar escolhas independentes.
// Não há serviço remoto, banco ou biblioteca externa.
function codigoPedido(int $id): string
{
    return 'DEMO-' . $id;
}

function calcularFrete(): float
{
    return 19.90;
}

// Contexto preservado entre frete e configuração de timeout.
// Esta função continua a mesma nos dois branches.
function moeda(): string
{
    return 'BRL';
}

function prazoTimeout(): int
{
    return 30;
}

// Outra região comum, separada das alterações acima.
// Esta mensagem também precisa sobreviver ao merge.
function mensagemConfirmacao(): string
{
    return 'Pedido de teste confirmado';
}

function registrarAuditoria(): bool
{
    return false;
}
"""
JS_BASE = """// Aceitar os dois trechos neste arquivo produz duas etapas válidas.
export const etapas = [
    'Recebido',
];

export function mostrarEtapas() {
    return etapas.join(' → ');
}
"""
SQL_BASE = """-- Exemplo de leitura, sem conexão nem execução automática.
SELECT id, nome
FROM clientes
WHERE ativo = 1
ORDER BY nome;
"""

NEXT_ROUND = r'''#!/usr/bin/env python3
"""Create another conflicting branch without deleting previous attempts."""
import json, os, subprocess
from pathlib import Path

root = Path(__file__).resolve().parent
metadata = root / '.git/codisseu-demo.json'
if not (root / '.git').is_dir() or (root / '.git').is_symlink() or not metadata.is_file():
    raise SystemExit('Execute somente dentro da demo Codisseu original.')
if json.loads(metadata.read_text()).get('id') != 'codisseu-git-merge-demo-v1':
    raise SystemExit('Esta pasta não possui a identificação da demo.')
env = dict({key: value for key, value in os.environ.items() if not key.startswith('GIT_')}, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null',
           GIT_TERMINAL_PROMPT='0', GIT_OPTIONAL_LOCKS='0')
def git(*args):
    return subprocess.run(['git', '-C', str(root), '-c', 'core.hooksPath=/dev/null',
                           '-c', 'commit.gpgsign=false', *args], env=env,
                          capture_output=True, text=True, timeout=30)
for name in ['MERGE_HEAD', 'REBASE_HEAD', 'CHERRY_PICK_HEAD', 'REVERT_HEAD',
             'rebase-merge', 'rebase-apply', 'sequencer']:
    if (root / '.git' / name).exists():
        raise SystemExit('Conclua ou aborte a operação atual antes de criar outra rodada.')
status = git('status', '--porcelain=v1', '--untracked-files=all')
if status.returncode or status.stdout:
    raise SystemExit('A árvore precisa estar limpa. Salve e faça commit da rodada atual primeiro.')
for number in range(1, 10001):
    branch = f'rodada-{number:03d}'
    exists = git('show-ref', '--verify', '--quiet', 'refs/heads/' + branch)
    if exists.returncode == 1:
        break
    if exists.returncode:
        raise SystemExit(exists.stderr)
else:
    raise SystemExit('Limite de dez mil rodadas alcançado.')
created = git('switch', '-c', branch, 'minha-versao-inicial')
if created.returncode:
    raise SystemExit(created.stderr)
merged = git('merge', '--no-commit', '--no-ff', 'equipe')
paths = git('diff', '--name-only', '--diff-filter=U')
expected = {'src/Checkout.php', 'web/etapas.js', 'sql/clientes.sql'}
if merged.returncode != 1 or paths.returncode or set(paths.stdout.splitlines()) != expected:
    raise SystemExit('Resultado inesperado; confira git status. Nenhum reset ou limpeza foi executado.\n'
                     + merged.stdout + merged.stderr)
print(f'Conflitos criados no branch {branch}. As rodadas anteriores foram preservadas.')
print(paths.stdout, end='')
print('Abra Source Control no Codisseu e dê duplo clique nos arquivos em conflito.')
'''


def git_env() -> dict[str, str]:
    return dict({key: value for key, value in os.environ.items() if not key.startswith("GIT_")}, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null",
                GIT_TERMINAL_PROMPT="0", GIT_OPTIONAL_LOCKS="0", LC_ALL="C")


def git(root: Path, *args: str, expected: tuple[int, ...] = (0,), date: str | None = None) -> subprocess.CompletedProcess[str]:
    env = git_env()
    if date:
        env.update(GIT_AUTHOR_DATE=date, GIT_COMMITTER_DATE=date)
    result = subprocess.run(["git", "-C", str(root), *args], env=env,
                            capture_output=True, text=True, timeout=30)
    if result.returncode not in expected:
        raise RuntimeError(f"Git {' '.join(args)}: {result.stderr.strip()}")
    return result


def guide(root: Path) -> str:
    return textwrap.dedent(f"""\
    # Demo de conflitos do Codisseu

    Pasta: `{root}`. Este repositório é fictício, local e sem remoto.
    Ele já abre com um merge pendente: sua versão (`main`) recebe `equipe`.
    Não execute os arquivos SQL contra um banco: são somente textos de teste.

    ## Testar pela interface

    1. Reabra o Codisseu atualizado e use **Abrir projeto** nesta pasta.
    2. Abra **Source Control** (`Ctrl+Shift+G`). Expanda o grupo de conflitos.
    3. Dê duplo clique em `src/Checkout.php`. À esquerda está sua versão;
       à direita, a versão da equipe; no centro fica o resultado.
    4. Cada lado tem decisões independentes: a seta inclui seu trecho no
       resultado e o X ignora aquele lado. No primeiro conflito PHP, aceite
       `Entrega expressa` pela seta da esquerda e ignore a direita pelo X.
       No segundo, aceite `12.90` pela seta da direita e ignore a esquerda.
       **Ctrl+clique na seta** é o atalho para escolher exclusivamente um lado.
    5. Na toolbar **Apply Non-Conflicting**, use **All**. Isso conserva timeout
       `45` da sua versão e auditoria `true` da equipe. Use **F7** e **Shift+F7**
       para navegar entre trechos. **Merge Settings → Undo Merge** e **Merge Settings → Redo Merge** desfazem e
       refazem escolhas no resultado; os fontes dos lados são preservados.
    6. Confira o centro e mantenha **Merge Settings → Stage as resolved** marcado, como vem por
       padrão. Use **Apply Changes**, confira a prévia e confirme **Apply**.
       O arquivo sai dos conflitos e a IDE abre o próximo arquivo conflitante.
       Para salvar só este e voltar à lista, use **Save and Close** e depois
       confirme **Apply** na prévia.
    7. Resolva `sql/clientes.sql`: escolha um lado com Ctrl+clique na seta,
       ou use **Merge Settings → Editar resultado completo** para escrever uma condição única.
       Aceitar os dois blocos WHERE sem editar produz SQL inválido. Após
       revisar uma edição manual, marque **Marcar resultado como resolvido**;
       editar novamente desmarca essa confirmação. **Merge Settings → Undo Merge** permite
       voltar às escolhas, preservando os fontes dos lados.
    8. Resolva `web/etapas.js`: aceite as duas setas do mesmo trecho, uma de
       cada lado. O resultado deve conter `Em separação` e `Conferido`, cada
       um em sua própria linha da lista. Aplique **All** se houver mudanças
       independentes pendentes. Confirme a resolução como no passo6.
    9. Depois de resolver os três arquivos, use **Continue** do merge se a IDE
       o oferecer. Se o fluxo pedir mensagem/commit, confirme um commit local.
       Alternativa no terminal: `git commit -m "Demo: conflitos resolvidos"`.
       O trabalho não é enviado para lugar nenhum.

    Os textos dos controles seguem o contrato da implementação Codisseu e podem
    aparecer em inglês. Não é uma alegação de que o PhpStorm instalado foi medido.

    ## Conferir o resultado

    No terminal desta pasta:

    ```bash
    git status
    git diff --name-only --diff-filter=U
    git diff --cached
    ```

    A segunda consulta deve ficar vazia quando todos os arquivos estiverem
    marcados como resolvidos. Antes do commit, `git diff --cached` mostra as
    escolhas que você preparou. Após o commit, `git status` deve indicar árvore
    limpa. Em `src/Checkout.php`, a solução sugerida tem:

    ```php
    return 'Entrega expressa'; // tituloCheckout
    return 12.90;             // calcularFrete
    return 45;                // prazoTimeout
    return true;              // registrarAuditoria
    ```

    Não substitua o arquivo inteiro por esse resumo: cada linha pertence à sua
    função. Os comentários e funções comuns também precisam permanecer.

    ## Gerar os mesmos conflitos outra vez

    Primeiro termine a rodada atual e faça seu commit. Depois execute:

    ```bash
    cd {root}
    python3 gerar-novo-conflito.py
    ```

    O script cria um branch novo `rodada-001`, depois `rodada-002` etc., sempre
    da tag `minha-versao-inicial`, e tenta juntar `equipe`. O erro de merge é
    esperado: ele deixa os três arquivos novamente em conflito para você testar.
    As rodadas resolvidas continuam nos branches anteriores. Não há reset,
    clean, exclusão de branch nem remoto. O script recusa árvore suja ou operação
    pendente para conservar o que você estiver fazendo.

    Para entender como o conflito nasce, consulte:

    ```bash
    git log --oneline --graph --all
    git show minha-versao-inicial:src/Checkout.php
    git show equipe:src/Checkout.php
    ```

    As duas versões modificam as mesmas linhas de um ancestral comum. Esse é
    o motivo do conflito; o Git não consegue escolher sozinho entre os valores.

    ## Cancelar uma rodada

    Somente nesta pasta de demonstração, para abandonar o merge ainda pendente:

    ```bash
    git merge --abort
    ```

    Isso descarta as resoluções ainda não concluídas dessa rodada e volta à
    versão anterior ao merge. Depois, `python3 gerar-novo-conflito.py` abre
    outra rodada. Se quiser guardar uma tentativa incompleta, copie os arquivos
    para outra pasta antes de abortar. Não use esse comando em outro projeto
    por causa deste tutorial.

    ## Cenários e valores

    | Arquivo | Base | Sua versão | Equipe |
    | --- | --- | --- | --- |
    | PHP: título | Entrega padrão | Entrega expressa | Entrega econômica |
    | PHP: frete | 19.90 | 24.90 | 12.90 |
    | PHP: timeout, sem conflito | 30 | 45 | 30 |
    | PHP: auditoria, sem conflito | false | false | true |
    | JS: item da lista | Recebido | Em separação | Conferido |
    | SQL: condição | ativo = 1 | ativo = 1 AND cidade = 'Recife' | ativo = 1 AND vip = 1 |

    Todos os fontes e a história Git foram gerados pelo Codisseu para este teste.
    Não há dependências para instalar, hooks de projeto, credenciais ou dados reais.
    """)


def write_sources(root: Path, php: str, js: str, sql: str) -> None:
    for name, source in {"src/Checkout.php": php, "web/etapas.js": js, "sql/clientes.sql": sql}.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source, encoding="utf-8", newline="")


def create(destination: Path) -> dict:
    destination = destination.expanduser().absolute()
    # mkdir is exclusive: even an existing empty directory is preserved.
    destination.mkdir(parents=True, exist_ok=False)
    git(destination, "-c", "init.templateDir=", "init", "-b", "main")
    for name, value in {"user.name": "Codisseu Demo", "user.email": "demo@example.invalid",
                        "commit.gpgsign": "false", "core.hooksPath": "/dev/null",
                        "core.autocrlf": "false", "core.quotePath": "false",
                        "merge.conflictStyle": "diff3", "protocol.allow": "never"}.items():
        git(destination, "config", "--local", name, value)
    write_sources(destination, PHP_BASE, JS_BASE, SQL_BASE)
    (destination / "README.md").write_text(guide(destination), encoding="utf-8")
    runner = destination / "gerar-novo-conflito.py"
    runner.write_text(NEXT_ROUND, encoding="utf-8")
    runner.chmod(0o755)
    git(destination, "add", "--", ".")
    git(destination, "commit", "-m", "Base comum: checkout, etapas e consulta fictícios", date="2026-11-04T12:00:00-03:00")
    git(destination, "tag", "base-comum")
    git(destination, "branch", "equipe")
    write_sources(destination,
                  PHP_BASE.replace("Entrega padrão", "Entrega expressa").replace("19.90", "24.90").replace("return 30;", "return 45;"),
                  JS_BASE.replace("'Recebido'", "'Em separação'"),
                  SQL_BASE.replace("WHERE ativo = 1", "WHERE ativo = 1 AND cidade = 'Recife'"))
    git(destination, "add", "--", ".")
    git(destination, "commit", "-m", "Sua versão: expressa, frete local, timeout e etapa", date="2026-11-04T12:01:00-03:00")
    git(destination, "tag", "minha-versao-inicial")
    git(destination, "switch", "equipe")
    write_sources(destination,
                  PHP_BASE.replace("Entrega padrão", "Entrega econômica").replace("19.90", "12.90").replace("return false;", "return true;"),
                  JS_BASE.replace("'Recebido'", "'Conferido'"),
                  SQL_BASE.replace("WHERE ativo = 1", "WHERE ativo = 1 AND vip = 1"))
    git(destination, "add", "--", ".")
    git(destination, "commit", "-m", "Equipe: econômica, desconto, auditoria e conferência", date="2026-11-04T12:02:00-03:00")
    git(destination, "switch", "main")
    metadata = {"id": DEMO_ID, "schema": 1, "path": str(destination),
                "base": git(destination, "rev-parse", "base-comum").stdout.strip(),
                "ours": git(destination, "rev-parse", "minha-versao-inicial").stdout.strip(),
                "theirs": git(destination, "rev-parse", "equipe").stdout.strip(),
                "conflict_paths": sorted(CONFLICT_PATHS)}
    (destination / ".git/codisseu-demo.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n")
    git(destination, "merge", "--no-commit", "--no-ff", "equipe", expected=(1,))
    actual = set(git(destination, "diff", "--name-only", "--diff-filter=U").stdout.splitlines())
    if actual != CONFLICT_PATHS:
        raise RuntimeError(f"Conflitos inesperados em {destination}: {actual}")
    metadata["unmerged_entries"] = len(git(destination, "ls-files", "-u").stdout.splitlines())
    metadata["php_conflicts"] = (destination / "src/Checkout.php").read_text().count("<<<<<<< ")
    if metadata["php_conflicts"] != 2 or metadata["unmerged_entries"] != 9:
        raise RuntimeError("A fixture não produziu os dois conflitos PHP e nove estágios esperados")
    return metadata


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="codisseu-merge-generator-test-") as directory:
        root = Path(directory) / "demo"
        report = create(root)
        assert report["php_conflicts"] == 2
        assert not git(root, "remote").stdout
        assert git(root, "show", ":2:src/Checkout.php").stdout.count("Entrega expressa") == 1
        assert git(root, "show", ":3:src/Checkout.php").stdout.count("Entrega econômica") == 1
        assert "return 45;" in (root / "src/Checkout.php").read_text()
        assert "return true;" in (root / "src/Checkout.php").read_text()
        untouched = (root / ".git/index").read_bytes()
        try:
            create(root)
        except FileExistsError:
            pass
        else:
            raise AssertionError("Existing directory was not refused")
        assert (root / ".git/index").read_bytes() == untouched
        pending = subprocess.run(["python3", str(root / "gerar-novo-conflito.py")], capture_output=True, text=True, env=git_env())
        assert pending.returncode != 0 and "Conclua ou aborte" in pending.stderr
        # Resolve only this disposable repository, preserving a completed attempt.
        for name in sorted(CONFLICT_PATHS):
            (root / name).write_text(git(root, "show", f":2:{name}").stdout, encoding="utf-8")
        git(root, "add", "--", ".")
        git(root, "commit", "-m", "Tentativa resolvida preservada")
        completed = git(root, "rev-parse", "main").stdout
        new_round = subprocess.run(["python3", str(root / "gerar-novo-conflito.py")], capture_output=True, text=True, env=git_env())
        assert new_round.returncode == 0, new_round.stderr
        assert git(root, "branch", "--show-current").stdout.strip() == "rodada-001"
        assert git(root, "rev-parse", "main").stdout == completed
        assert set(git(root, "diff", "--name-only", "--diff-filter=U").stdout.splitlines()) == CONFLICT_PATHS
        git(root, "merge", "--abort")
        (root / "nao-apagar.txt").write_text("preservar\n")
        dirty = subprocess.run(["python3", str(root / "gerar-novo-conflito.py")], capture_output=True, text=True, env=git_env())
        assert dirty.returncode != 0 and "árvore precisa estar limpa" in dirty.stderr
        assert (root / "nao-apagar.txt").read_text() == "preservar\n"
    print("Self-test passed: real conflicts/stages, refused overwrite/pending/dirty, preserved completed round, no remote")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path, default=DEFAULT_DESTINATION)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--record", type=Path)
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    try:
        report = create(args.destination)
    except FileExistsError:
        parser.error(f"Destino já existe e foi preservado: {args.destination}. Para outra rodada da demo use gerar-novo-conflito.py.")
    if args.record:
        args.record.parent.mkdir(parents=True, exist_ok=True)
        args.record.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
