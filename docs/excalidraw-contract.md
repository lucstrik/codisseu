# Desenhos Excalidraw nativos — 20/11/2026

Pedido: desenhar dentro do Codisseu, com arquivos pertencentes ao projeto e
restauração ao reabrir a pasta, usando a organização existente de ferramentas.

O editor é uma implementação própria Rust/egui do formato aberto `.excalidraw`
JSON v2. Não incorpora a interface React nem executa a extensão VS Code.
Referências documentais: [formato oficial](https://github.com/excalidraw/excalidraw/blob/2c3dcc06ec807e81ab9437eceb1a8f7a3c590743/dev-docs/docs/codebase/json-schema.mdx),
[serialização](https://github.com/excalidraw/excalidraw/blob/2c3dcc06ec807e81ab9437eceb1a8f7a3c590743/packages/excalidraw/data/json.ts)
e [extensão](https://github.com/excalidraw/excalidraw-vscode/blob/06bb63cea9b52d4759fc16606e95f48c0accf14c/README.md),
consultadas em 20/11/2026 (`documented`, revisões upstream fixadas nos links; nenhuma medição
local da extensão). Código e símbolos novos são próprios; nenhum ativo de marca
é incorporado. Compatibilidade não significa paridade de todas as funções.

## Contrato

- Desenhos são documentos do editor comum: mesmas abas, dirty, Save, Undo/Redo,
  conflito externo, encerramento e workers de arquivos. Canvas é uma apresentação
  do JSON do documento; não mantém uma segunda fonte persistente de conteúdo.
- Painel Desenhos permite criar e abrir arquivos reais `.excalidraw` e
  `.excalidraw.json` dentro das raízes declaradas. Criação recusa sobrescrita,
  traversal e saída por symlink. Arquivo vazio abre como uma cena nova.
- Catálogo de caminhos fora do projeto usa a identidade canônica da pasta ou
  `.code-workspace`, dentro do perfil estável. Reabrir restaura desenhos
  registrados existentes, mesmo quando suas abas foram fechadas. Projetos
  diferentes e workspaces vizinhos não compartilham catálogo. Arquivos ausentes
  não são recriados; limite explícito de documentos continua valendo.
- I/O do catálogo/criação ocorre em worker com ACK; troca/saída aguarda operações
  aceitas. Resultados de outro projeto não abrem abas no projeto atual.
- Selecionar, criar formas/texto/linhas/setas/desenho livre, mover/redimensionar,
  excluir/duplicar, ajustar estilo, pan/zoom e enquadrar devem alterar a cena
  real. Uma ação de desenho corresponde a um grupo de Undo do documento.
- Campos desconhecidos, objetos não editáveis e `files` são preservados. A UI
  informa limitações de renderização. JSON inválido não vira cena vazia nem é
  sobrescrito automaticamente. Visualização de código permite corrigir o JSON.
- SVG é exportação explícita, sem execução de conteúdo ou acesso à rede.
  Não oferecer colaboração cloud, bibliotecas remotas, importação PNG/SVG com
  fonte embutida ou equivalência ao renderer rough.js sem implementação/teste.

## Aceitação

1. Criar desenho A, formas/texto/estilo, Undo/Redo, salvar e conferir JSON real.
2. Reiniciar e A→B→A: arquivos e desenhos restaurados, sem mistura de projetos.
3. Importar fixture JSON upstream-compatible com campos extras e regravar sem
   perder esses campos; entradas inválidas e conflito externo preservados.
4. Testar cliques/atalhos/drag/pan/zoom e inspecionar capturas reais em 1×/1,25×.
5. Build local instalado e pacote/CI/release conferidos antes de relatar deploy.

Resultados, limitações e limpeza constam de `docs/status.md` e do relatório da
entrega. Compilação isolada não comprova os cenários acima.
