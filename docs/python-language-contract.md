# Python — provider e refatoração,02/11/2026

Contrato do incremento `python-support-contract.md`. Mesmo editor/transportes
PHP/JS/TS/Rust; Python é `Language::Python` para `.py`, `.pyi`, `.pyw`.
Basedpyright1.40.2 MIT executa somente após Start/autorização do projeto;
nenhum Python/projeto/dependência é executado para reconhecer um arquivo.

`LanguageCommand::PythonEnvironment { python:Option<PathBuf> }` recebe caminho
absoluto declarado (preserva symlink do venv), até4096bytes, ou None. Guarda
configuração mesmo antes do Start; responde `workspace/configuration` por seção
e publica `workspace/didChangeConfiguration` com `python.pythonPath`. Não lê
módulos nem executa o interpretador no Codisseu. Troca cancela/invalida consultas,
ações/hints e diagnósticos Python; PHP/TS/Rust conservam seus slots. Aplicação
reinicia o provider após mudança aprovada para renovar a análise do ambiente.
Push sem versão conserva a limitação de causalidade do contrato LSP comum.

Configuração Basedpyright ativa autoimports/hints e análise de arquivos abertos;
desativa autoFormatStrings (não anunciado como edição implícita), baseline writes
(baselineMode=discard) e OrganizeImports, atendido pelo Ruff. Demais opções do
projeto continuam sob configuração do servidor. InlayHints usa negociação,
UTF16, versão/ID/epoch, timeout/cancelamento e limites do transporte existente.
Label e tooltip são texto inerte; metadados executáveis são descartados. O nome
público rust_capabilities permanece compatível, agora chaveado também por Python.

Rename recebe exclusivamente as edições de Basedpyright. Antes do preview e no
Apply, Tree-sitter Python0.25.0 audita identificadores exatos e árvore concreta
antes/depois. String/comentário não podem ser editados; expressões identificadoras
em f-string permanecem código. Candidato Unicode XID é validado na gramática e
exige forma NFKC para evitar aliases de identificadores Python. Nome novo já
existente (incluindo equivalente NFKC) em qualquer identificador do corpus causa
Rejected conservador. Projeto limitado64fontes/8MiB/512entradas/profundidade8/2s;
venv/caches não integram corpus. Limite/cancelamento/sintaxe inválida bloqueiam
Apply. Nenhuma edição é sintetizada por nome. Escopo PythonAnalyzerBindings
combina bindings do provider e prova estrutural local; não certifica imports
arbitrários, monkey patching, introspecção ou completude global.

Navegação externa permite somente fontes py/pyi/pyw sob stdlib/typeshed privados
e `lib/python3.N`/Lib do interpretador declarado/base, com caminhos canônicos
confinados. Não segue entradas .pth arbitrárias nem amplia acesso a todo HOME.
Bibliotecas abrem somente leitura inclusive quando venv está dentro do projeto;
rename/edição/Save recusados. Bibliotecas Rust mantêm suas guardas existentes.

Migração estende apenas vetores reconhecidos como gerenciados. Vetores explícitos
permanecem exatos; approvals antigos mantêm seus comandos, sem transferir confiança
a Python ou a executável alterado. Limite passa a4providers únicos.
Templates Python são boilerplate original do Codisseu, separados do completion
semântico. Âncoras AST de busca apresentam classes/funções, sem inferir bindings.

Fontes primárias: [configuração Basedpyright](https://docs.basedpyright.com/latest/configuration/language-server-settings/),
[gramática Tree-sitter Python0.25.0](https://github.com/tree-sitter/tree-sitter-python/tree/v0.25.0).
Versões, corpus, comandos e resultados efetivos em
`artifacts/python-integration-20261102/language`; compilação não substitui aceitação.

## Evidência executada

Corpus `language/results.json`:196 testes comuns de linguagem passaram (27
ignorados) e254 de sintaxe passaram; os4 testes novos de transporte exercitam
cancelamento/ambiente, resposta hostil e isolamento PHP/Rust. Aceitação real:
Basedpyright com completion/autoimport/hover/signature/def/ref/rename/hints e
3 cenários Python reais no total incluindo transação de rename e bibliotecas
readonly. Duas regressões reais Rust passaram; políticas e regressões existentes
de núcleo constam separadamente, sem contar testes ignorados como aprovação.
Clippy all-targets dos3crates passou com -D warnings. Hashes/versões e logs no
corpus; não representa execução de toda a suíte do workspace.

Corpus `ui/results.json`:2 testes reais ignorados executados explicitamente,
ambos em100%/125%, passaram em31,61s. Widgets de completion/autoimport, hover,
rename, templates, hints/copy/caret, Ruff format/Undo/Redo/Save e duas ações Ruff
com preview/primeiro clique/Undo.20 capturas WGPU, prévias de correção e imports
inspecionadas nas duas escalas. Não é validação de input nativo do SO. Atalhos
Python omitidos inicialmente e viewport da prévia pequena foram encontrados na
QA e corrigidos pelo núcleo/UI antes da repetição final. Execução/testes/debug e
portátil têm corpus próprio sob responsabilidade dos respectivos módulos.
