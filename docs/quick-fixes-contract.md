# Correções PHP/JS/TS — 30/10/2026

Cenário: Alt+Enter sobre um problema PHP, JavaScript/JSX ou TypeScript/TSX consulta
o provider da raiz ativa, apresenta ações contextualizadas, prévia e aplicação
explícita nos buffers. Rust mantém o mesmo serviço e seu corpus. Sem salvar,
instalar dependências ou executar comandos do servidor implicitamente.

## Transporte e integridade

- Negociar CodeAction literal/data/disabled/preferred/resolve edit em PHP/TS/Rust.
  Inlay hints continuam exclusivos de Rust. Resolver somente token privado atual.
- Encaminhar diagnósticos originais validados da versão sincronizada que cruzam
  a faixa consultada, inclusive code/source/data; limite200/1MiB por documento.
  Sync/Close invalidam cache da fonte, respostas e propostas obsoletas. No
  Phpactor instalado, ações de variável sem diagnóstico vinculado são filtradas
  pelas faixas concretas retornadas, pois esse provider ignora a faixa solicitada.
- Aceitar WorkspaceEdit de arquivos existentes da mesma linguagem do provider
  (JS/JSX/TS/TSX pertencem ao provider TS). Recusar operações de recursos,
  commands executáveis, caminhos externos/symlinks, versões incorretas, intervalos UTF-16
  inválidos e sobreposições. Ações command-only ficam desabilitadas com motivo;
  não simular execução nem transformar metadata do servidor em shell. O adapter
  TypeScript auditado pode descartar somente o wrapper auxiliar
  `_typescript.applyCodeActionCommand` quando já há edit e `action.commands`
  ausente/vazio: a implementação OSS instalada comprova que esse wrapper é no-op.
- Aplicar pela transação existente do núcleo, com snapshots privados, nova
  validação no worker, Undo/Redo agrupados e Save explícito. Apresentação pública
  não é autoridade para modificar conteúdo. Alterar buffers/disco invalida prévia.
- Respostas vazias, provider ausente/sem capacidade, timeout/cancel/crash devem
  terminar em estado explícito. Limites existentes128ações/32arquivos/1024edits/
  4MiB mantidos. Não anunciar todas as intentions proprietárias como implementadas.

## Interface

Referência funcional e visual: PhpStorm2026.2 Help/intention-actions, confiança
`documented`; imagens oficiais com origem/hash no corpus. Build instalado
2026.2.3/PS262.10968.76 é metadado observado; tema/escala runtime atuais unknown.
Perfil isolado com bloqueio de acordo anterior não deve ser reaberto/contornado.

Lista compacta sem escurecimento da janela, seleção/ícones e prévia de código
junto das ações, baseada na imagem oficial. Usar fonte/paleta/tokens já definidos,
texto da linguagem ativa, setas/Enter/Escape, clique fora e foco de retorno ao
editor. Prévia usa o highlighter da extensão correta. Aplicação explícita somente
após a prévia correspondente estar disponível. Rust preserva o fluxo já testado.
Igualdade integral de pixels só pode ser declarada com comparação equivalente;
não inferir medidas locais a partir de imagem pública de DPI desconhecido.

## Aceitação

Providers reais: corrigir variável PHP, importar símbolo e/ou corrigir nome JS/TS,
incluindo JSX/TSX, Unicode antes da faixa e CRLF; prévia/Apply/Undo/Redo/Save.
Corpus negativo: command/resource/outside/overlap/surrogate/stale/readonly,
crash/cancel/timeout; regressão Rust e navegação/formatters. GUI WGPU real em
100%/125%, interação por Alt+Enter, teclado e mouse, screenshots inspecionados.
Compilar não substitui validação funcional. Resultados e limites em status.md.

Prévia limitada a14linhas/16KiB no trecho da primeira edição de cada arquivo,
começando uma linha antes para manter a alteração imediatamente visível. Fonte
longa usa rolagem horizontal. A prévia não é um diff integral de arquivos grandes.
