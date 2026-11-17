# Desenvolver o Codisseu

O workspace usa Rust, egui/eframe e um único editor. A GUI fica em
`crates/codisseu-ui`, o estado e os serviços em `crates/codisseu-core`, o editor
em `crates/codisseu-editor` e o executável em `apps/codisseu`.
Os demais crates cuidam de linguagens, banco, Git, depuração e integrações.

Antes de alterar código, leia [AGENTS.md](../AGENTS.md) e
[STORM_PLAN.md](../STORM_PLAN.md). Os contratos ficam em `docs/`; a arquitetura
está em [architecture.md](architecture.md).

## Compilar e abrir

O arquivo `rust-toolchain.toml` fixa Rust **1.97.0**, com rustfmt e Clippy.
Você precisa da toolchain e de um ambiente Linux com display e bibliotecas
gráficas adequadas. O primeiro build baixa as dependências do Cargo; o lockfile
deve ser preservado.

```sh
cargo run --locked -p codisseu
cargo run --locked -p codisseu -- --workspace /caminho/do/projeto
```

Para experimentar, copie `fixtures/fullstack` para um diretório descartável e
abra essa cópia. Você pode isolar a sessão com `--session /tmp/codisseu-teste/session.json`
e iniciar sem restaurar abas com `--no-restore`.

O primeiro `.deb` público contém o núcleo da IDE. Os motores de linguagem,
depuradores e o navegador têm preparação própria e explícita, tanto no build
de desenvolvimento quanto nessa distribuição.
Consulte [providers.md](providers.md), [debugging.md](debugging.md),
[browser-usage.md](browser-usage.md) e [install-linux.md](install-linux.md).
Não é preciso ter todos eles para abrir o editor.

## Verificar uma mudança

Escolha os testes do módulo afetado e execute as verificações adequadas:

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --lib --locked --jobs 1 -- --test-threads=2
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 tools/dependency_inventory.py --check
```

Alguns testes de integração são opt-in porque exigem processos reais, ferramentas
preparadas, display ou servidores de teste. A suíte comum não comprova esses
fluxos. Exemplo para os providers locais já preparados:

```sh
CODISSEU_PROVIDER_CONFIG="$PWD/.tools/providers.json" \
  cargo test -p codisseu-language --test providers --locked -- --ignored --nocapture
```

Um build aprovado comprova compilação. Para uma mudança funcional, exercite o
comportamento e suas falhas. Para uma mudança visual, inspecione a interface
real; screenshots antigas não validam a nova versão. `cargo xtask` ainda não é
um comando implementado neste workspace.

## Preparar uma distribuição

O guia de [instalação Linux](install-linux.md) descreve o pacote e seus requisitos.
O builder deve levar apenas os arquivos conhecidos do produto e os componentes
redistribuíveis necessários. Não copie perfis, credenciais, bancos, `.env`,
projetos pessoais ou evidências privadas para o pacote ou para o repositório.

Confira origem, licenças, notices, hashes e as obrigações de código-fonte dos
componentes redistribuídos antes de publicar. Os registros estão no
[inventário de terceiros](third-party-inventory.md).
