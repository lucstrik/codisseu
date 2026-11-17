# Editor e Markdown — atalhos Linux

Os comandos abaixo atuam no editor de código focado. Menus de contexto e **Editar → Ações de edição** oferecem as mesmas operações sobre o documento aberto, com Undo/Redo. As cores e fontes são as do Codisseu. Referência observada: VS Code1.143.1 Linux; cenário e limites.

| Ação | Atalho |
|---|---|
| Mover linha/bloco para cima/baixo | Alt+↑ / Alt+↓ |
| Duplicar linha/bloco para cima/baixo | Ctrl+Shift+Alt+↑ / ↓ |
| Excluir linhas | Ctrl+Shift+K |
| Inserir linha abaixo/acima | Ctrl+Enter / Ctrl+Shift+Enter |
| Gerar PHPDoc antes de função/método PHP | Digitar `/**` e pressionar Enter |
| Formatar arquivo PHP/SQL/JavaScript/TypeScript ativo inteiro | Ctrl+Alt+L (mesmo com seleção); também Ctrl+Shift+I / Shift+Alt+F |
| Indentar / remover recuo | Tab / Shift+Tab; Ctrl+] / Ctrl+[ |
| Selecionar linha | Ctrl+L |
| Comentar/descomentar linhas | Ctrl+/ |
| Adicionar / remover comentário de linha | Ctrl+K, Ctrl+C / Ctrl+K, Ctrl+U |
| Comentar/descomentar bloco | Ctrl+Shift+A; alias Shift+Alt+A |
| Próxima ocorrência | Ctrl+D |
| Pular ocorrência selecionada | Ctrl+K, Ctrl+D |
| Todas as ocorrências do texto selecionado | Ctrl+Shift+L; Ctrl+F2 |
| Adicionar cursor acima/abaixo | Shift+Alt+↑ / ↓; Ctrl+Shift+↑ / ↓ |
| Adicionar/remover cursor com mouse | Alt+clique |
| Cursor no fim das linhas selecionadas | Shift+Alt+I |
| Desfazer mudança de cursor/seleção | Ctrl+U |
| Ampliar/reduzir seleção | Shift+Alt+→ / ← |
| Ir ao par de delimitadores | Ctrl+Shift+\ |
| Navegar por palavra / selecionar por palavra | Ctrl+← / →; Ctrl+Shift+← / → |
| Ir ao começo/fim do documento | Ctrl+Home / End; com Shift seleciona |
| Início inteligente da linha / fim | Home / End; com Shift seleciona |
| Remover palavra anterior/seguinte | Ctrl+Backspace / Delete |
| Rolar uma linha/página | Ctrl+↑ / ↓; Alt+PageUp / PageDown |
| Copiar / recortar / colar | Ctrl+C / X / V |
| Copiar/recortar linha sem seleção | Ctrl+C / X |
| Selecionar tudo / cancelar seleção extra | Ctrl+A / Escape |
| Desfazer / refazer | Ctrl+Z / Ctrl+Shift+Z; Ctrl+Y |
| Localizar / substituir | Ctrl+F / Ctrl+H |
| Próximo/anterior resultado | F3 / Shift+F3 |
| Abrir projeto no seletor de pastas do sistema | Ctrl+Shift+O |
| Novo arquivo / salvar | Ctrl+N / Ctrl+S |
| Ir à definição / voltar / avançar | F12 / Ctrl+Alt+← / Ctrl+Alt+→ |
| Ir à implementação | Ctrl+F12; Ctrl+Alt+clique |
| Ir ao supermétodo/supertipo PHP | Ctrl+Alt+U; seta ascendente na margem |
| Ir ao tipo (incluindo retorno de factory) | Ctrl+Shift+B |
| Localizar usos | Shift+F12; Alt+F7 |
| Alternar sidebar esquerda/direita | Ctrl+B / Ctrl+Alt+B |
| Completar código / assinatura | Ctrl+Space / Ctrl+Shift+Space; assinatura também Ctrl+P |
| Renomear símbolo | F2; Shift+F6 |
| PHP/Blade: inserir live template | Trigger+Tab; Ctrl+J abre a lista |
| Formatar documento | Ctrl+Shift+I; alias Shift+Alt+F |
| Formatar seleção | Ctrl+K, Ctrl+F |
| Cancelar formatação pendente | Escape |
| Dobrar / abrir bloco | Ctrl+Shift+[ / ] |
| Dobrar tudo / abrir tudo | Ctrl+K, Ctrl+0 / Ctrl+K, Ctrl+J |
| Dobrar nível1–7 | Ctrl+K, Ctrl+1…7 |
| Alternar bloco / dobrar recursivamente | Ctrl+K, Ctrl+L / Ctrl+K, Ctrl+Shift+L |
| Dobrar/abrir recursivamente | Ctrl+K, Ctrl+[ / ] |
| Dobrar comentários | Ctrl+K, Ctrl+/ |
| Dobrar/abrir regiões | Ctrl+K, Ctrl+8 / 9 |
| Dobrar/abrir fora da seleção | Ctrl+K, Ctrl+- / = |
| Remover espaços finais | Ctrl+K, Ctrl+X |
| Juntar linhas; maiúsculas/minúsculas | Editar → Ações de edição |
| Markdown: alternar prévia/fonte | Ctrl+Shift+V |
| Markdown: fonte e prévia lado a lado | Ctrl+K, V (V sem Ctrl) |

Vírgula significa sequência: solte as primeiras teclas e pressione o segundo conjunto. Escape cancela. O prefixo expira após5s. Em `.sql`, queries cadastradas e JSON com Mongo ativo, Ctrl+Enter no editor executa a seleção ou o bloco sob o cursor; Ctrl+Shift+Enter insere linha acima. Commit pelo editor usa Ctrl+Alt+K; Ctrl+K continua disponível no contexto Git fora do editor. Campos de formulários e terminal mantêm seus próprios atalhos.

Ctrl+N abre um documento Untitled em memória; Ctrl+S pede o caminho e recusa sobrescrever um destino existente. Markdown abre em Source. A barra superior alterna Source/Split/Preview, sincronização de rolagem e zoom. A prévia acompanha alterações ainda não salvas. Tarefas clicáveis alteram o próprio arquivo e aceitam Undo; só Ctrl+S grava. Títulos, listas, tabelas, links, blocos de código e imagens locais PNG/JPEG/GIF/WebP são renderizados nativamente. Clique em link local abre o documento; duplo clique no conteúdo revela a fonte. Imagens remotas não são carregadas; HTML é literal, SVG/diagramas/matemática/extensões não estão implementados.

A formatação PHP e SQL funciona localmente: Ctrl+Alt+L formata apenas o arquivo ativo inteiro, mesmo com seleção, com Undo/Redo e sem salvar. Não ocorre ao abrir, digitar ou salvar. Ver contrato PHP e SQL. Em `.js`/`.jsx` e `.ts`/`.tsx`, Ctrl+Alt+L usa o servidor JavaScript/TypeScript já existente, que precisa estar ativo/sincronizado. Formata somente o arquivo ativo inteiro, mesmo com seleção. Ver contratos JavaScript e TypeScript. TypeScript foi exercitado com servidor real; o Phpactor instalado não anuncia formatação. As ações correspondentes informam indisponibilidade em vez de modificar o texto por aproximação. Indentação é inferida de amostra limitada do documento. Comentários são transformações textuais por extensão; seleção ampliada usa palavra/linha/documento, sem prometer análise semântica. Folding é lexical limitado a1MiB/20mil linhas. Até512 cursores; sem personalização de keymap ou snippets de extensões neste incremento. Não se declara equivalência com todos os comandos/contexts registrados no VS Code.

A árvore Project oferece New File/New Folder, Rename…/F2 e Delete, para arquivos e pastas.
Com foco na árvore, Ctrl+C/X/V copia/recorta/cola o item no projeto; Alt+Ctrl+C copia
o caminho absoluto e Alt+Ctrl+Shift+C o relativo. Ctrl+Enter abre arquivo ao lado;
Alt+Shift+F pesquisa o texto na pasta; Alt+Ctrl+R abre a pasta que contém a seleção.
O contexto oferece terminal no diretório selecionado, comparação de arquivos salvos,
Markdown Preview e ações Git aplicáveis. O clipboard de arquivos é interno ao workspace.
O F2 atua no item selecionado quando a árvore tem foco; no editor, conserva
Rename de símbolo. Delete na árvore pede confirmação para a Lixeira. Ver
[Laravel e arquivos](laravel.md) para templates e limites da análise Laravel.


Navegação PHP (M3.35): mantenha **Ctrl** sobre um símbolo resolvido para ver o
sublinhado de link. **Ctrl+clique** em uso ou **F12** abre a declaração; Ctrl+clique numa declaração PHP abre usos.
**Ctrl+Alt+clique** ou **Ctrl+F12** busca implementações de interfaces/métodos.
**Ctrl+Alt+U** sobe ao método ancestral mais próximo em cada ramo ou aos
supertipos diretos da classe/interface. **Ctrl+Shift+B** consulta o tipo resolvido,
inclusive o retorno de uma factory. Ambos aparecem no menu Navegar e contexto.
**Ctrl+U** continua desfazendo cursor; **Ctrl+B/Ctrl+Alt+B** conservam as sidebars.
Quando houver vários destinos, use **↑/↓/Enter**, clique no item, **Escape** ou
clique fora para fechar; o ícone no título abre os destinos no painel Referências.
**Ctrl+Alt+←/→** volta/avança. Provider PHP precisa estar pronto; consultas não
salvam arquivos. Bindings Laravel dinâmicos dependem da análise do provider.

Project (M3.36): por padrão, o painel acompanha o arquivo ativo e revela sua
posição após navegação/abas/histórico. Nos três pontos do Project, **Behavior ›
Always Select Opened File** desativa esse acompanhamento. Aparece então o alvo
**Select Opened File**, junto de New File: clique para localizar o arquivo do
editor. A escolha persiste ao fechar normalmente; revelar não salva nem altera
o caret. Pastas são carregadas sob demanda. Barra com ícones próximos, sem modal.


Opções de breakpoint (29/10/2026): Ctrl+Shift+F8 abre propriedades da linha
ativa; botão direito no marcador oferece editar/ativar/remover; Alt+clique
alterna Enabled e Shift+clique prepara logpoint. Enter aplica, Escape cancela.
Ctrl+F8 conserva adicionar/remover. Guia: debugging.md; contrato
m5-breakpoint-options-contract.md.

## Python —02/11/2026

`.py`/`.pyi`/`.pyw` compartilham os atalhos do editor: Ctrl+Q documentação,
Ctrl+B definição, Shift+F6 rename semântico com prévia, Tab para templates e
Ctrl+Alt+L para formatar o arquivo completo com Ruff (até256KiB, sem Save).
Alt+Enter oferece as ações Ruff de correções seguras/imports do arquivo quando
há diferença real, com prévia colorida e Undo. Shift+F10/Play abre a revisão de
execução do arquivo `.py`/`.pyw`; `.pyi` não executa. Comentários usam `#`, Enter
considera blocos/strings e a indentação existente. Dicas de tipos/parâmetros são
visuais e podem ser desativadas nas preferências. [Guia e limites](python-usage.md).
