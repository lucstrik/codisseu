# Workspace com vários projetos —27/10/2026

Pedido: abrir uma pasta agregadora de microserviços, preservando encoding,
configurações de linguagem e repositórios independentes na mesma janela Rust/egui.
Identidade visual e ferramentas existentes são mantidas.

## Contrato e cenários

- MP-01: cada documento tem raiz de projeto, resolvida no worker por ancestrais
  com `.git`, `composer.json`, `package.json` ou `tsconfig.json`. Dependências
  `vendor`, `node_modules` e `target` não se tornam projetos por seus manifests.
  Preferências explícitas de pasta também definem raízes. Sem marcador, a raiz
  aberta é o fallback. Não executar manifests, hooks, Composer ou npm.
- MP-02: UTF-8, Latin-1 verdadeiro e Windows-1252 coexistem. Preferência do
  projeto/pasta mais específico prevalece; preferência da agregadora não mascara
  `.editorconfig` de outro projeto. EditorConfig é resolvido por arquivo, com
  precedência de seção/ancestral e `root=true`. Charset desconhecido ou padrão
  não suportado falha explicitamente, sem assumir UTF-8.
- MP-03: o rodapé mostra encoding efetivo; seu menu identifica a pasta afetada.
  Alterar encoding recarrega somente documentos limpos desse projeto; buffers
  dirty de irmãos permanecem intactos. Arquivos/BOM/CRLF não são convertidos.
  Validação e persistência atômicas precedem aplicação de qualquer buffer.
- MP-04: restauração, arquivo novo, watcher, busca/leitura semântica e Git usam a
  mesma política de bytes. Trocar de aba/projeto não salva ou conecta ferramentas.
- MP-05: servidores de linguagem são iniciados sob demanda com raiz/configuração
  do serviço; processos/cache têm limite e cleanup. Respostas/rename de outro
  projeto não são aplicadas por coincidência de nomes.
- MP-06: Git descobre os repositórios filhos e worktrees, identifica caminhos
  relativos e mantém alterações/índice/commit/rascunho por repositório. Um filho
  quebrado não oculta os outros; limites e erros são explícitos.
- MP-07: PTYs por diretório, Docker por contexto, APIs organizadas em collections
  do workspace e banco por datasource conservam sua identidade ao alternar
  arquivos de serviços. Validar efeitos reais em fixtures próprias; não usar
  containers ou bancos empresariais para testes.

## Validação

Corpus inclui três repositórios descartáveis com encodings distintos, nomes
Unicode, `.editorconfig`, roots aninhadas e homônimos; Save/Undo/restore/dirty,
charset inválido, symlink e falha atômica; Git real com HEAD/índice/bytes dos
irmãos verificados; LSP real quando disponível; PTY Linux, HTTP loopback,
SQLite e container próprio. Screenshots devem vir do aplicativo real.

Não promete todos os encodings, toda sintaxe EditorConfig, suporte irrestrito
a monorepos, paridade universal VS Code ou plataforma não exercitada.
