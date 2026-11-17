# Trabalhar com Rust no Codisseu

Abra a pasta do projeto Cargo e um arquivo `.rs`. Ele usa o mesmo editor, abas,
Git, busca e terminal dos projetos PHP/JS/TS/SQL. Rust e TOML já têm ícones
licenciados; funções, keywords, macros, strings, lifetimes e comentários recebem
destaque lexical incremental, inclusive sem servidor de linguagem.

O pacote portátil inclui Cargo/rustc, rust-analyzer, rustfmt, Clippy, rust-src
e CodeLLDB privados; não exige RustRover. Configurações em `.cargo/config`,
RUSTC/RUSTFLAGS e linker explícitos continuam tendo precedência.
O padrão privado é Rust1.97.0: `rust-toolchain.toml` sozinho não alterna versão,
pois o pacote não usa rustup. Crates com código C/C++ ou dependências nativas
precisam da toolchain correspondente. Outras versões/targets e dependências
do projeto exigem preparação própria; abrir não baixa ferramentas. Veja
rust-portable-contract.md.

No ambiente de desenvolvimento foram instalados
os componentes `rust-analyzer` e `rust-src` da toolchain1.97.0 já existente;
`rustfmt` e `clippy` já estavam disponíveis. Em outra máquina com rustup, prepare:

```sh
rustup component add rust-analyzer rust-src rustfmt clippy
```

Em **Ferramentas → Providers de linguagem**, revise os executáveis e inicie-os
para confiar nessa configuração. Configurações gerenciadas descobrem Rust em
recursos privados, CARGO_HOME/bin ou PATH no worker. A confiança antiga não é
transferida silenciosamente a um novo executável. Configurações explícitas são
preservadas; para elas acrescente `{"language":"Rust","program":"rust-analyzer",
"args":[]}` ao array JSON (máximo três providers distintos).

O analyzer oferece diagnósticos nativos, completion automático e **Ctrl+Space**,
hover/documentação, definição **F12/Ctrl+clique**, implementação, referências e
assinaturas, com buffers não salvos. **Ctrl+Alt+L** formata o arquivo Rust ativo
via rustfmt, com Undo/Redo; Save continua explícito. **Tab** expande abreviações
como `fn`, `pubfn`, `main`, `struct`, `enum`, `impl`, `test`, `match`, `for`,
`iflet`, `println`; **Ctrl+J** abre o catálogo. Tab/Shift+Tab percorrem os campos;
Escape encerra o template. São21 templates próprios, não extraídos do RustRover.

O analyzer consulta Cargo metadata offline e rustc após a confiança. Não ativa
build scripts, proc macros ou Cargo check ao salvar. Assim, diagnósticos de
compilação/borrow checker e macros dependentes de execução exigem as tarefas
Cargo explícitas. Metadata pode criar Cargo.lock; não baixa dependências nesse
modo. Projetos ainda sem dependências locais precisam de sua preparação normal
pelo desenvolvedor no terminal. Abrir a pasta não executa programas ou testes.

O martelo ao lado de Play/Debug abre **Build** (**Ctrl+F9**) quando o arquivo
ativo é Rust ou Cargo.toml dentro de um projeto Cargo. Ele fica cinza em outros
contextos sem build integrado. Play/Shift+F10 prepara Run para o contexto Rust.

Em **Executar → Rust · Cargo…**, escolha Check, Build, Run, Test, Clippy ou
Documentation. Revise manifesto/cwd, executável, pacote/binário, Release,
Offline, Locked, All features e argumentos do programa/teste (um por linha).
Confirme a confiança e clique **Executar tarefa Cargo**. O terminal nativo
mostra output/código de saída; Ctrl+C interrompe a tarefa. Arquivos modificados
precisam ser salvos antes. Nada é interpolado em um shell ou salvo automaticamente.
O manifesto mais próximo do arquivo ativo é usado; Cargo resolve o workspace.
Um workspace virtual com vários binários exige escolher pacote/binário.

Para depurar, faça **Cargo → Build** e abra **Executar → Debug…**. O arquivo
`.rs` seleciona Rust / CodeLLDB. Escolha o executável compilado em `target/debug`
(ou o caminho de artefato do seu perfil/target), cwd, argumentos e ambiente.
O modo Release precisa de símbolos em Cargo.toml. A revisão não compila nem
executa um `.rs` como script. Marque breakpoints pelo gutter/Ctrl+F8 e autorize
o início. Pilha, variáveis, Evaluate/Alt+F8, watches, F7/F8/Shift+F8, F9 e Stop
usam o painel Debug compartilhado. Condições/logpoints dependem do adaptador;
uma macro pode gerar várias localizações na mesma linha.

CodeLLDB1.12.3 é instalado privadamente com
`python3 tools/install_rust_debugger.py`; `--verify` confere seus hashes. O produto
procura o recurso `rust-debug` do pacote ou a instalação privada de desenvolvimento;
Advanced aceita um executável CodeLLDB explícito. Não requer VS Code ou RustRover.

**Shift+F6** abre Rename Rust. Revise nome, referências e preview; **Refatorar**
aplica a transação aos buffers, com Undo/Redo agrupados e Save explícito.
A autoridade semântica é rust-analyzer, com validação estrutural Tree-sitter.
Funções, tipos, métodos, campos, parâmetros e locais (inclusive Unicode e raw
`r#type`) têm corpus real. Nomes já existentes no corpus são recusados de forma
conservadora; renomear módulos/arquivos, transformações estruturais e tokens de
macros não são suportados. Comentários e strings não são alterados.

**Alt+Enter**, ou **Correções rápidas Rust** no contexto do editor, consulta
ações reais do analyzer. Selecione a correção, revise Antes/Depois com as mesmas cores do editor e clique
**Aplicar correção**. Undo/Redo são agrupados; salvar continua explícito.
Somente edições em fontes `.rs` existentes do projeto são aceitas; comandos do
servidor, criação/remoção de arquivos e edições externas são recusados.

Tipos inferidos e nomes de parâmetros aparecem discretamente na linha após
a análise. As dicas acompanham a região visível e respeitam versões de buffers;
não entram no texto, seleção copiada, Save ou Undo. Em **Preferências**,
**Rust · Mostrar tipos e parâmetros no editor** permite desligá-las. O analyzer
pode omitir hints redundantes; o Codisseu não inventa rótulos.

Limites atuais: Launch Linux foi exercitado; Attach/Listen Rust continuam
recusados. Completion LSP preserva texto/edições adicionais, sem anunciar snippets
do servidor. Semantic tokens, expansão executável de macros e árvore gráfica de testes
ainda precisam de contratos/corpus próprios; não são apresentados como concluídos.
Cores exatas e paridade integral com RustRover não foram certificadas.

“Ir à definição” pode abrir fontes `.rs` do sysroot rustup e do Cargo registry
somente para leitura. O rodapé informa essa proteção. Seleção, cópia, Find e
Back funcionam; as abas externas não são restauradas como editáveis. Checkouts
Git de dependências e sysroots personalizados fora dessas raízes não são
liberados automaticamente; hover/navegação encadeada a partir da aba externa
não sincronizada também têm escopo limitado.
