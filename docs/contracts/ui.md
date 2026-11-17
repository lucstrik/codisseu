# Contrato da interface M1/M2 e revisão visual V3

Atualização de abertura/saída11/11/2026:
workspace-close-contract-20261111.md.
Topo/welcome/menu compartilham intenção de projeto persistente, com destino,
espera cancelável e resolução de arquivos não salvos. O ACK é consumido antes de
tentar abrir. Fechar abandona respostas tardias do seletor, interrompe novos
pedidos de UI e solicita Stop dos providers antes de esperar o estado ocioso.
Persistência mantém ACK/erro explícitos; progresso de saída usa os tokens do app.
Navegador/Docker/API só entram na finalização após resolver a gravação da sessão.

StormApp apresenta AppService; nenhum caminho de menu/atalho escreve arquivo diretamente. Editor usa TextEdit como ponte provisória: o histórico pertence ao TextBuffer do domínio e o histórico duplicado do widget é limpo. Cache String é renovado ao mudar versão/documento; edição rejeitada invalida o cache. Paste passa pelo widget; cursor/seleção dirigidos pertencem ao domínio desde M3.11, sem confundir índices escalares com UTF16 do LSP.

IDs estáveis: painéis storm.menu/toolbar/project/activity/status, editor (storm.editor, document_id), abas por document_id, árvore por path. Labels acessíveis são fornecidos para campos, botões e splitter. Inter e JetBrains Mono incorporadas, com fallback egui/Noto Emoji/Noto Sans Symbols2. A V3 substitui os ícones textuais por desenhos vetoriais próprios em `components.rs`, mantendo nomes acessíveis. Cores e geometrias têm procedência e grau de confiança em `design/token-evidence.json` e `docs/visual-spec-v3.md`.

Abrir usa diálogo de caminho dentro do app; não é seletor nativo de arquivos com navegação de diretórios. I/O assíncrono pode falhar; erros ficam visíveis. Ctrl+Shift+O é consumido antes de Ctrl+O para evitar conflito de modificadores. Atalhos de editor ficam inativos durante diálogos/encerramento. Cancelar fechamento devolve foco ao editor.

Janela intercepta close e exige resolução de dirty. Encerramento normal desabilita novas edições, aguarda operações, solicita PersistSession e espera ack. Falha de sessão permite voltar ou sair sem alterar sessão anterior. O estado básico persiste workspace/paths/aba ativa; dimensões dos painéis persistem desde M6.2; caret, seleção, scroll e grupos foram ampliados nos contratos M3 posteriores.

Captura opt-in por --capture recebe os pixels renderizados pelo viewport e grava PNG em worker. Espera I/O concluído sem erro e três passes; JSON registra escala, tamanho e pass solicitado. Não captura outras janelas, não carrega imagens como fachada e não simula estado do produto. --close-after-capture encerra pela mesma persistência normal. Erro de captura permanece na UI; o runner externo possui timeout.

Verificação: tests/workflow.rs usa cliques, foco, seleção Ctrl+A, teclado, menus e arraste do splitter via egui_kittest. Janela real Wayland/wgpu/Vulkan capturada separadamente. Clipboard do SO, IME/teclas mortas, leitor de tela real, troca de DPI e arraste de abas não foram validados. Não há baseline de comparação com JetBrains nesta etapa.

## M2 — capacidades integradas e visual

Database à direita: conexão/desconexão explícitas, schema real e tipos/PK/nullability. Na V3, caminho e botão Conectar aparecem no estado desconectado; conectado mostra basename da base, namespace `main` efetivamente consultado pelo adaptador e tabelas/colunas. Cabeçalho contém ações reais com labels Desconectar, Atualizar schema e Abrir SQL…. Resultados inferiores: grade somente leitura, linhas visíveis virtualizadas, valores tipados em tooltip, truncamento explícito e cancelamento. SQL é um documento compartilhado; Ctrl+Enter executa seleção não vazia ou documento. IDs: storm.database/schema, storm.db.connection/main, storm.db.grid/scroll, storm.problems, storm.providers.settings. Os rails fixos não oferecem handles falsos de resize; Project/Database/resultados têm divisórias reais.

Providers têm configuração por JSON de executáveis, iniciar/parar explícitos e status real. Problemas lista mensagens do LSP e navega ao documento/posição UTF-16, rejeitando posição inválida. O texto completo fica no tooltip; publicações sem versão são identificadas, inclusive quando vazias. Atalhos do editor ficam suspensos no diálogo dos providers.

Destaque lexical próprio PHP/JS/TS/SQL usa cores da fonte OSS centralizada e cache por texto/documento. Limites:256KiB/32768segmentos; fallback texto simples acima do limite. Não é parser completo: sem heredocs, interpolação semântica, regexJS ou linguagens embutidas. Não fornece diagnósticos. Ainda usa TextEdit provisório, sem múltiplos cursores ou acessibilidade do SO comprovada.

Capturas M2 incluem observação serializada real: documentos/versões, schema, contagem de linhas, status/diagnósticos e inferência sem versão. `--wait-diagnostics` espera publishDiagnostics de cada PHP/TS; não prova que push sem versão corresponde a edição concorrente. Startup assíncrono passa pelos mesmos comandos; captura final espera startup concluído, idle, ausência de erro e três passes. Encerramento normal para providers/desconecta, aguarda ack e persiste a sessão. O callback de captura solicita repaint para que saída não dependa de input externo.

Testes novos: `capabilities.rs` para widgets SQLite e `lsp_workflow.rs` para LSP real, configuração, foco/navegação/edição. Validação nativa separada por `tools/native_m2.py`.

## V3 — componentes e geometria

O header principal usa40 unidades lógicas, rail40, botões principais30 e cabeçalhos de ferramenta41, com valores documentados no snapshot OSS fixado. UI Inter13 também foi confirmada no código da plataforma. Abas do editor usam fila42 como adaptação inferida da fórmula upstream, não como medida de screenshot. Editor15, largura lateral245, limites de resize e gutter continuam decisões Storm. A decoração nativa do compositor não entra nessas medidas.

`components::header` estabelece faixa alinhada e separador inferior; os painéis aplicam padding explícito sem duplicar margens externas. `icon_button` oferece clique/teclado, label acessível, tooltip, hover/pressed/foco e estado selecionado. `tree_row` usa linha24, disclosure e ícone próprios; não é uma imagem do IDE. O desenho vetorial usa canvas20 para ações e16 para árvore, sem importar marcas ou SVGs comerciais. Altura de header e raio de canto são conceitos separados.

Abas de resultados/problemas/atividade são planas com indicador ativo4. Status de providers ficam em faixa própria com texto13; a indicação de publicação sem versão continua visível. Formulário Database conectado foi retirado do fluxo principal para dar espaço ao catálogo real; erros continuam aparecendo. O adaptador permanece SQLite somente leitura, sem expansão funcional artificial.

Os números upstream são coordenadas lógicas Swing; o egui tem outra rasterização. Hover branco alpha22/255 e pressed38/255 vêm do tema OSS. Raio6 do botão, raio2 do indicador e stroke1.4 são adaptações Storm documentadas, sem equivalência pixel a pixel comprovada. Estados e dimensões precisam de inspeção nas capturas reais; não se altera baseline para esconder divergências.

Regressões obrigatórias: `workflow.rs`, `capabilities.rs`, `lsp_workflow.rs` e FULL-01 mantêm ações reais. `visual_contract.rs` valida a geometria/interação observada da interface nas larguras1280/900/800, sem comparar apenas constantes. A evidência final de testes/capturas V3 deve ser registrada em `docs/status.md` pelo integrador; esta seção descreve o contrato e não antecipa aprovação de toda a suíte.

Revisão final V3: seleção de abas/rails/árvore e expansão de pastas são expostas ao AccessKit. A árvore mede o nome e a profundidade antes de alocar a linha, permitindo rolagem horizontal; evita pintar linhas fora do clip. A grade mede larguras uma vez por resultado/fonte/escala usando `DbService::result_revision`, mantém células24 e não mantém geometria obsoleta entre consultas. Testes reais em `visual_contract` e `sql_width_cache`.

## V4 — menus, formulários e árvore de objetos

A barra principal coloca o nome do workspace junto dos menus. MenuBar preserva hover entre menus abertos, Escape, nomes acessíveis e ações habilitadas conforme estado real; atalhos são apresentados em coluna própria. As medidas de formulário usam tokens com procedência: mínimo72×28 para botões,28 para campos simples, borda1/foco2 e raio4 inferido. Menus usam24 por decisão Storm; popup/seleção usam as cores públicas verificadas. O editor de código não recebe a altura de formulário.

Abrir caminho usa Modal: bloqueia interação com o fundo, começa no campo, confirma Enter somente quando o evento pertence ao campo ou ao botão Abrir e cancela via Cancelar/Escape. Cancelamento volta ao editor. Aberturas que terminam depois na fila transferem foco à nova aba após `poll`, sem roubar foco de outro diálogo ativo. Salvar pelo menu/toolbar também retorna ao editor. A confirmação de descarte continua protegendo alterações não salvas; voltar é a ação primária. O seletor ainda recebe caminho, sem navegação de diretórios.

Database compartilha o renderer de árvore com Project, mantendo IDs explícitos por conexão/caminho de objeto. O container expõe Tree e cada objeto TreeItem com nível, seleção, expansão e descrição de tipo/PK/NULL. Clique seleciona e alterna ramo; Left/Right recolhem/expandem ou navegam ao pai/filho; Up/Down navegam linhas; Enter/Space alternam ramos. Folhas não fingem abrir editor de dados. Estado permanece após refresh do mesmo objeto; desconexão remove catálogo. Campos/botões Database e providers usam estilo de formulário, com Conectar desabilitado quando vazio.

Regressões: `menus_dialogs.rs` (modal/foco/Enter/Escape/menus), `database_tree.rs` (SQLite real, teclado, IDs/metadados e refresh). `tools/native_controls.py` acrescenta teclado/cliques XTest no XWayland, direcionados somente à janela criada pelo runner/PID correspondente. Confere bytes salvos, arquivo não editado, fechamento pelo menu e sessão. Captura X11 é imagem real da janela; waits entre ações permitem entregar eventos, mas somente as asserções de disco/sessão comprovam efeitos. IME, clipboard do sistema, Alt+F4 no KWin e leitor de tela não recebem status de verificados.

Os dois runners nativos registram SHA do executável efetivamente carregado via `/proc/<pid>/exe`, impedindo atribuir capturas a um build substituído durante execução. Resultados, falhas de preparação do runner e caminhos finais ficam em `docs/status.md` e `docs/visual-validation-v4.md`.

## M3.1 — busca literal no documento

Ctrl+F abre Localizar, Ctrl+H também mostra Substituir; menus Editar usam o mesmo
fluxo. F3/Shift+F3 e botões navegam ciclicamente. Enter no campo Localizar avança,
Shift+Enter volta. Escape fecha e devolve foco ao editor. Os campos de busca
mantêm seu próprio undo; Ctrl+Z neles não desfaz o documento.

AppService executa Find/ReplaceMatches em worker, com ID/versão/query. A UI só
habilita ações para resultado do documento/versão/query atuais. Troca de aba,
texto ou query invalida a seleção de busca; posições são escalares, nunca bytes.
Replace-all é um único passo no histórico do domínio e não salva automaticamente.
Busca é literal e sensível a maiúsculas, sem regex, normalização ou refatoração.
O painel usa tokens/formulários existentes e quebra as linhas em editor estreito.
Seleção atual aparece no editor e sua posição é rolada para a área visível.
O limite de navegação de ocorrências é explícito; substituir todas usa o worker
sobre o documento completo, sujeito ao limite de tamanho e guarda de versão.

Cenário: `reference/scenarios/M3-SEARCH.md`. Aprovação e limites da rodada devem
ser consultados em `docs/status.md`, sem antecipar conclusão de toda a M3.

## M3.2 — documentação do símbolo e captura condicionada

Exibir/Documentação do símbolo e Ctrl+Q consultam o cursor real via AppService.
Janela storm.documentation junto ao cursor, texto selecionável com rolagem,
espera/erro/vazio explícitos; Escape/fechar volta ao editor. Edição/troca de
aba fecha conteúdo antigo. Desde11/11/2026, Markdown recebido usa o parser
compartilhado em modo inerte: títulos/listas/ênfase formatados e código cercado
ou inline com a fonte e a paleta do editor. Sem carregar imagens, executar HTML
ou ativar links. Layout compacto, selecionável e rolável, com quebra por palavras
e limites do viewport. Tokens próprios; contrato detalhado em
hover-visual-contract-20261111.md.

Captura com --wait-diagnostics aceita publicação vazia legítima. A nova opção
--wait-nonempty-diagnostics também exige ao menos um diagnóstico por documento
PHP/TS, destinada às fixtures que deliberadamente contêm erros. Não cria
mensagens nem muda o serviço LSP; impede captura prematura dessas fixtures
após publicação inicial vazia. Exige --start-providers e --capture. O runner
nativo mantém timeout externo e verifica conteúdo e linguagem dos diagnósticos.
Não resolve a correlação de push TypeScript sem versão.

## M3.3 — ir à definição e medidas locais

Navegar/Ctrl+B consulta o caret real; Ctrl+Alt+Left/Right e menu fazem histórico.
Escolha de múltiplos destinos usa modal `storm.definition.choices`, IDs por linha,
Up/Down/Enter/Escape; Escape cancela espera. Busca/diálogos têm atalhos próprios.
Resultado validado foca/rola o editor; não salva. Contrato em
m3-definition-contract.md.

PhpStorm262 Dark foi observado em janela nativa isolada. Editor JetBrainsMono13,
passo22, offset texto68, caret row#26282E e número atual#A1A3AB têm evidência
em `design/token-evidence.json`. Número foi aproximado à borda esquerda e o
rótulo de caminho passou ao nó acessível, retirando a linha acima do código.
Painel de atividade pode ser ocultado pelo controle Exibir já existente;
splitters continuam reais. Diferenças e regiões em
visual-validation-m3-definition.md.

## M3.4 — popup de sugestões

Ctrl+Space funciona com foco no editor; Editar/Completar código é ação explícita
por menu. O popup storm.completion usa opções ListBoxOption com IDs estáveis e
seleção acessível. Up/Down escolhem, Enter insere e Tab substitui quando há intervalos distintos;
TextEdit único mantém seu intervalo. Escape fecha;
clique também aceita. Nenhum newline/tab é inserido junto da aceitação.
Movimento de cursor, edição, troca de aba e foco fora do editor encerram a lista.

Popup Dark medido: linhas24, borda1, footer27/Inter12, textoJetBrainsMono13, PHP
Bold13 de fonte pública OFL. Larguras314/364 foram observadas nesta fixture,
com limite no viewport; oito linhas visíveis são decisão Storm. Texto de label
e detalhe tem regiões de clip separadas, com tooltip integral. Lista rola sem
forçar seleção antiga a cada frame. Tokens/procedência em design/token-evidence.
Diferenças semânticas dos providers permanecem visíveis; não se inventa tipo ou
parênteses para fazer a captura parecer completa.

## M3.5 — Find persistente e preview

Navegar/Buscar referências e Alt+Shift+7 usam o caret real com foco no editor.
Alt+F7 é alias; o primeiro atalho foi observado no perfil KDE da referência.
Texto imprimível associado ao atalho X11 é consumido junto com a tecla.
Resultado usa o painel inferior compartilhado, grupos por linguagem/workspace/
pasta/arquivo, linhas selecionáveis e preview somente leitura. Classificações
semânticas de tipos de uso do original não são inventadas.

Up/Down/Home/End percorrem usos visíveis, pulando arquivos recolhidos. Clique
seleciona/atualiza preview; Enter ou duplo clique abrem a fonte e mantêm Find.
Escape cancela consulta pendente; em lista concluída devolve foco ao editor
preservando resultados. Fechar encerra lista/painel. Splitters inferior e entre
resultados/preview são reais. Texto longo mantém extensão de rolagem e tooltip.

TextEdit aplica o texto recebido no frame pelo serviço Edit antes do lote de
atalhos Desfazer/Refazer/Salvar. Os comandos do lote mantêm sua ordem: Undo→Save
enfileira o texto posterior ao Undo; Save→Undo deixa o Undo não salvo. Testes
RawInput verificam também texto+Save no mesmo frame, buffer/bytes/marca de alteração.
Listas customizadas cancelam a navegação espacial do egui ao consumir setas,
inclusive no frame imediatamente após clique/foco.

Linhas24/Inter13, cores Dark, header41 e divisória foram medidos no PhpStorm262.
Altura264 é default desta fixture, redimensionável. Agrupamento por linguagem,
indentação19 e ícones próprios são escolhas Storm; tipos semânticos e Call
Hierarchy permanecem ausentes. A comparação final está em
validação visual M3.5.

## M3.6 — nome, preview e aplicação

Shift+F6/Editar abre campo de nome com preparação real. Enter/Preview solicita
a proposta; painel inferior mostra arquivos/edições e contexto antes/depois,
seleção por mouse/setas, splitter real e Refatorar/Cancelar. Preview de código
é limitado a7linhas/8KiB sem quebrar UTF-8; recorte é identificado. Snippets de
árvore têm512escalares e novo texto no tooltip128. Não renderiza arquivo inteiro.
Escape cancela e devolve foco. Aplicação concluída fecha painel e atualiza
editor; shortcuts continuam respeitando Edit antes do lote Undo/Redo/Save.

Diálogo PHP463x225, barra de ações35px, fontes e cores têm observação local;
TS usa o mesmo diálogo Storm e difere do original254px/com opções adicionais.
Sem controles que finjam recursos ausentes. Aviso de conflitos não verificados
permanece visível pois ambos providers aceitaram colisão real. Contagem inclui
declaração de fato proposta, em vez de imitar2usos do original.

No rename, clique na linha já focada não repete request_focus (isso reinicia
o filtro egui). O handler consome a seta e cancela navegação espacial antes de
focar o próximo uso; regressão exige clique→Down imediato sem frame neutro.
Runner nativo verifica a posição/altura do preenchimento selecionado no PNG,
além das provas de bytes/undo/redo/save.

Input nativo pode agrupar clique e a seta seguinte em um frame. Nesse caso o
movimento é aplicado depois de selecionar a linha clicada, preservando a ordem
dos eventos; a seleção do clique não pode sobrescrever a navegação posterior.
Teste RawInput agrupa press/release do mouse e ArrowDown em uma única entrada.


M3.7: conflito/estrutura Unsupported abre modal próprio, com seleção acessível,
fonte anterior somente leitura e anotação persistente. Aplicação bloqueada;
Alterar nome retorna ao campo preparado, Cancelar/Escape preserva buffers e
retorna foco. Rodapé tem áreas separadas de status e botões, verificadas por
ponteiro real para impedir sobreposição de hit-test. Não oferece Refactor Anyway.
Ver m3-rename-conflicts-contract.md e visual-validation-m3-rename-conflicts.md.

## M3.8 — escopo de hierarquia e identificador do conflito

Rodapé usa o escopo do relatório, distinguindo mesma classe de hierarquia
conhecida. A árvore informa Conflitos na hierarquia quando pertinente. O código
somente leitura marca o range primário real em UTF-16, convertido estritamente
para posições do galley; range truncado ou surrogate parcial não é desenhado.
Cor própria com procedência em design/token-evidence.json. Não há seleção de
métodos relacionados até existir suporte funcional a contratos completos.
Contrato.

## M3.9 — preview de cadeia completa

O mesmo painel revisa todas as edições LSP de overrides certificados no escopo
primitivo. Rodapé informa o número de pares únicos com contratos iguais; tooltip
mantém limites da análise. A cadeia inteira pode ser aceita ou cancelada; seleção
independente de métodos relacionados continua pendente, sem checkboxes fictícios.

O marcador do identificador no preview comum usa o token #373B39 também observado
em capturas novas do PhpStorm262 Dark. Compartilha conversão UTF16→colunas do
galley com o modal de conflitos. A seleção da árvore permanece azul. Não mede
prefixos isolados nem multiplica coluna UTF16 por largura de caractere.

## M3.10 — revisão dos métodos e digitação incremental

Modal918×525 em viewport1280×800, tabela180px com cabeçalho25/linhas21, fonte do
método selecionado somente leitura, checkboxes reais e IDs estáveis por token/
índice. Clique/setas mudam fonte; Space altera marcação; Todos/Nenhum modificam
a seleção privada; OK/Enter só confirma cadeia completa, Cancelar/Escape cancela
a operação. Controles mantêm papéis acessíveis. Preview final conserva M3.9.

TextEdit permanece ponte: GUI envia delta escalar ancorado no cursor e intenção
de digitação/backspace/delete/colagem. Foco/cursor separam grupos, sem reconstruir
rope por tecla. Glifo ausente usa tofu em vez de panic; isso preserva os bytes e
não significa cobertura de todas as fontes. Renderização/cache String integral,
IME completo, grafemas e cursor no Undo/Redo permanecem pendentes.
Contrato.


## M3.11 — ponte do editor e seleção dirigida

O adaptador `storm-ui::editor` é separado do shell. Sincroniza seleção do domínio
antes do input TextEdit, limpa o histórico duplicado e envia EditTransaction com
texto/seleção antes/depois no mesmo comando versionado. Movimento envia
SetSelection sem alterar versão de texto ou notificar LSP. A seleção de cada aba
sobrevive à perda de foco. Menu/toolbar mantêm pedido de foco até um frame iniciar
com o editor focado, permitindo digitação logo após Undo/Redo.

O caret próprio usa a ponta real do galley, snap físico e tokens medidos2×18px na
escala1. Seleção de código usa #214283, sem mudar seleção de formulários. Espaço
entre glifos0.2 é adaptação egui para passoASCII8px medido; não é uma propriedade
atribuída ao original. IME/grafemas/afinidade horizontal, seleção de sessão,
virtualização e arquivos grandes continuam pendentes.
Contrato.


## M3.12 — editor próprio virtualizado

O código usa widget próprio (`editor/{layout,input,tiles,lexical,accessibility}`),
com AppService e seleção/histórico canônicos; TextEdit fica nos formulários.
Cache por revisão/linha/fontes/escala, remapeamento após newline,256linhas e4MiB
por documento/view. Linhas até2048escalares usam galley individual; maiores usam
prefixos medidos em tiles de até1024escalares, preferindo corte em whitespace e
com contexto de grafemas. Até8tiles novos globais por frame; miss lazy limitado.
Coordenadas ainda não medidas são None e aparecem como layout pendente. Não se
adivinha largura nem se constrói galley integral de16MiB. Preedit tem cache
transitório próprio e snapshot Rope privado, sem entrar no documento/histórico.

Estado lexical entre linhas roda em worker,1job coalescido, geração/cancelamento
e metadados limitados a65536mudanças. Enquanto desconhecido, cor simples; após
aceitar a revisão, invalidação de cores. Não é parser/semantic tokens nem resolve
linguagens mistas. Linhas longas usam destaque simples; grafema patológico acima
de4096escalares tem erro visual explícito, preservando conteúdo editável.

Mouse/arraste/autoscroll, seleção dirigida, tabs4, PageUp/Down, CtrlHome/End,
clipboard/CRLF, grafemas e IME passam pelos mesmos comandos versionados. Coluna
do rodapé expande tabs pelos mapas já medidos; None exibe reticências. A política
Unicode de status é escalar+tabs, sem alegar equivalência a toda coluna do original.
AT-SPI recebe texto Unicode original em TextRuns visíveis mais endpoints e ações
reais. Isso não certifica travessia assistiva de todo arquivo nem Orca/Wayland.
Contrato.

## M3.14 — glifos, estilos e shell

Contrato: cobertura original das fontes públicas,
offset exclusivamente visual; PHPDoc/variáveis/declarações com cores próprias ou
observadas e continuidade lexical corrigida. Abas preservam nome/foco e targets
separados de ativação/fechamento, incluindo proteção de dirty. Ícones por extensão
são badges próprios. Seleção inativa da árvore e rail seguem observações nativas.
A lista de documentos das abas é global; não confundir seleção/scroll por view
com membership independente de abas. Resultados e diferenças restantes em status.

M3.14: LineCache descarta galleys e tiles após recriação do atlas egui, mesmo
sem nova revisão. Validação inclui captura real na GPU na abertura, além de
interações em renderização por software. O divisor horizontal começa após o
pixel de borda do painel; hit target e geometria dos grupos permanecem intactos.

## M3.15 — Editor/Fonte

Editar→Configurações (Ctrl+Alt+S) abre modal com checkbox acessível de ligaduras e
preview que usa a política real do editor. Padrão ativado por escolha do usuário;
checkbox aplica comando tipado imediatamente, clique dá foco, Espaço alterna,
Concluído/Escape fecham e devolvem foco ao editor. Input/atalhos do documento não
atravessam o modal. O mesmo valor governa caches planos, tiles e preview IME de
ambos os grupos; alternar invalida layouts sem descartar texto/histórico/seleção.

Modo desligado separa grafemas ASCII em seções de shaping, mantendo clusters e
runs não ASCII conectados. O modo ligado conserva o shaping existente. Trabalho
continua limitado por linha/tile/quadro; a memória das seções é contabilizada.
`unicode-segmentation1.13.3`, já fixada no workspace, é agora dependência direta da
UI; nenhuma nova versão/pacote/font foi incorporada. Hinting/cobertura/baseline do
produto permanecem os da M3.14, após comparação dos modos disponíveis sem ganho.
Referência PhpStorm está com ligaduras desligadas e Subpixel observado em Settings;
a diferença de padrão é intencional. Evidência.

## M3.16 — composição restaurada

Orientação/proporção do divisor, grupo ativo e seleção/scroll por documento
passam ao serviço comum e à sessão do perfil. A GUI aplica scroll antes do input;
restaurar não força revelar um caret que estava fora da viewport. Cache, IME e
IDs efêmeros são reconstruídos. Lista de abas permanece global; membership
independente observado no PhpStorm continua pendente. Dois grupos, sem ampliar
limites de documentos/cache. Ligaduras continuam ativadas por padrão.
Contrato/corpus.

## M3.17 — abas por grupo

Faixa de abas renderiza somente membros da view, com IDs estáveis por grupo/ID
para corpo e close. Alvos separados, close16×41 com ícone16px; seleção do grupo
passivo conserva underline cinza e close visível. Região de pré-ativação por
clique passa a conter somente o editor abaixo das abas/busca: fechar aba passiva
não muda o foco/grupo ativo antes do comando. Ctrl+W fecha a instância local.
Cancelar última instância dirty devolve editor sem mutação; Descartar usa CloseTab.
Cache/IME da UI remove instâncias que deixam a lista local.

Geometria medida: faixa42px e componentes visíveis41px. Na adaptação egui,
listas avançam com sobreposição1px para alinhar limites observados. Larguras derivadas do texto Inter público e tokens, arredondadas
para baixo em pontos; leading border1px no painel esquerdo/horizontal. Underline
4px/arredondamento2 e linha inferior do gap. Referência privada não fornece ativos
ao produto. Contrato.

## M3.18 — close e cobertura das abas

Close normal/hover/down usa geometria própria; hover/down compartilham o estilo.
Corpo não selecionado clareia label/ícone e revela close sem alterar fundo;
selecionadas conservam sua apresentação. Foco por teclado mantém indicação
visível. Alvos/IDs e ActivateTab/CloseTab permanecem os daM3.17.

Mesh de cobertura calculada por pixel físico, cache32entradas/1MiB de dados de
geometria e chave por escala/fase/paleta/parâmetros. Não usar screenshot/asset
comercial nem alterar rasterização global. Press solto fora do alvo cancela,
conserva grupo passivo e permite input no editor ativo; Undo/Save reais fazem
parte do corpus. Contrato e
comparação. Resíduos do close,
ícones/texto/decorações/shell continuam explícitos; não há identidade global.

## M3.19 — abas e decoração Unnecessary

Contrato de abas e
contrato semântico. Badges TS/PHP partem de caminhos
SVG públicos auditados Apache2; renderer Rust independente com subdivisão
adaptativa geral, cobertura e cache limitado, sem PNG da referência. Close e
underline refinados por geometria própria. Labels mantêm Inter4.001 e sua pintura anterior, após rejeitar fonte3.019 e
avanços inteiros por regressão visual. Ligaduras/fallbacks/editor preservados.

Botão More26×24 por grupo tem ID estável, papel/nome acessível, foco e teclado;
lista somente ações reais. Abrir/cancelar no passivo não ativa sua view; remover
divisão envia CloseView com ID explícito e mescla abas sem perder dirty/Undo.
A orientação usa SetEditorSplit e preserva o grupo ativo. O limite de dois grupos
é representado pelos estados habilitados. O menu difere do conjunto do original.

EOF em viewport curta limita o scroll de contexto à linha do caret, impedindo
retries/repaints infinitos depois de alternar orientação. Seleção/bytes intactos.
Decoração Unnecessary usa diagnósticos aceitos, versões e UTF16 validados;
pintura em clips disjuntos dos galleys existentes, sem alteração de métricas,
shaping ou texto acessível. Composição IME suspende o fading.

## M3.22 — preview de rename e revisão nominal

Os previews de rename, conflitos e métodos relacionados usam o mesmo shaping
por linha do editor: expansão de tabs em colunas escalares, ligaduras conforme
preferência, espaçamento, altura e deslocamento dos glifos. O mapa da linha crua
para a linha expandida posiciona os ranges UTF-16 convertidos, inclusive após
emoji/combinantes. São superfícies somente de leitura, limitadas pelos corpora
existentes; não criam outro buffer ou histórico.

Labels do diálogo, células relacionadas e mensagens de conflito reutilizam o
cache LCD opcional limitado das abas, com fonte pública e fallback egui. O clip
da célula preserva bordas e linhas adjacentes. Checkbox relacionado tem response
próprio, papel CheckBox, nome estável e estado checked; clique/Space mudam a
seleção real, com foco visível. Não há pintura de um checkbox egui escondido sob
outro. A seleção parcial não autoriza Apply.

O diálogo conserva preview obrigatório e o escopo efetivo do projeto. Opções do
original sem implementação (ocorrências textuais, comentários/strings, seletor
de escopo) não são controles inertes. Por isso a igualdade integral do diálogo
não é certificada. Referência local PS-262.10968.76 Dark, escala1; medições e
limites em visual-validation-m3-rename-contracts.md.

Ao selecionar outra ocorrência, o preview revela o range real com margem
horizontal e mantém a numeração das linhas fixa. Depois dessa mudança inicial,
o usuário conserva o scroll manual. Não se recorta silenciosamente o alvo fora
da viewport apenas porque sua linha é longa.

## Shell e produtividade — 23/10/2026 (pedido extra da M3)

O header compacto substitui o rótulo CODISSEU/ações redundantes por Menu principal,
badge/nome real do projeto e pesquisa/configurações/providers. O menu pode ser
expandido no header; mantém geometria durante ações sucessivas e as ações de
Arquivo/Editar/Exibir/Navegar existentes. Rails40 e status30, divisórias próprias,
painéis com raio4 e borda fina conforme preferência adicional do usuário. Fonte
medida e distinção entre observação/adaptação em m3-shell-reference.md e
m3-shell-productivity-contract.md. Configurações abre preferências reais; providers
mantêm configuração própria. Painel inferior inicialmente oculto; ações reais o
abrem. ID codisseu.editor, grupos/documentos e fluxos de rename não são substituídos.

Pesquisar arquivos/duplo Shift abrem Modal de arquivos/abertos/ações,
com árvore limitada do core, nomes/caminhos Unicode e :linha:coluna. Campo
Pesquisar no projeto possui foco inicial; Escape volta ao editor. Duplo Shift
isolado é cancelado por outra tecla/clique/perda de foco e não intercepta campos.
Seleção secundária/duplo clique e limites: m3-selection-productivity-contract.md.

Terminal/Alt+F12 abre PTY apenas por ação explícita. Root tick drena saída mesmo
oculto; workers acordam a GUI por saída/status, sem repaint contínuo ocioso.
Foco terminal suspende atalhos do editor; Ctrl+C chega como ETX mesmo quando o
bridge produz Copy. Ao trocar workspace, fecha a sessão anterior; ocultar preserva.
Encerramento normal fecha sessão. Contrato/limites em m3-terminal-contract.md.

## M3.31 — encoding clicável no rodapé

O status exibe FileEncoding efetivo do documento (inclusive BOM) e abre menu
nativo egui com UTF-8/ISO-8859-1/Windows-1252. Sem arquivo aberto, exibe o padrão
do projeto; sem projeto, controle desabilitado. Seleção envia apenas
SetProjectEncoding ao AppService. Texto explica recarga e persistência por pasta;
falhas de dirty/I/O usam status/erro existentes. Nenhum widget grava fonte ou
converte arquivo diretamente. Contrato/corpus: m3-encoding-contract.md.


## M6.2 — dimensões compartilhadas e persistentes

Contrato em m6-panel-layout-contract.md.
IDs de região únicos codisseu.activity/project/database independentemente da
ferramenta. Viewports limitados impedem expansão por conteúdo. SetShellLayout
valida dimensões lógicas no core; Session schema2 incorpora campo opcional
shell_layout, validado no worker e aplicado junto à restauração assíncrona.
O mesmo ACK de PersistSession cobre layout; sessões antigas têm defaults.
Clamp de viewport não altera preferência; encoding respeita rail direito.
O Database vazio tem rolagem para manter todas as opções acessíveis.


## M5.9/M6.3 — scroll Git e avisos,26/10/2026

Contrato: m5-git-scroll-performance-contract.md. Truncamento da árvore continua
no modelo, limites e documentação; avisos recorrentes foram retirados da árvore
Project e da mensagem de abertura. Erros continuam apresentados. Autoria Git
usa fonte/centro vertical comum ao rodapé. Git Log virtualiza refs além de commits,
com preparação cacheada por dados/filtro/expansão e cópia de commits visíveis.


## M5.10 — ferramenta Docker inferior

ToolTab::Docker participa da navegação comum depois de Referências, com o mesmo
ID/altura de painel persistente. A seleção da lista lateral abre a ferramenta;
docker_tool_panel desenha detalhes/logs/ações/feedback e docker_panel conserva
somente inventário/filtros/configuração. Confirmar remoção valida ID/contexto/
estado nas duas superfícies. Header inferior oferece configuração Docker.
Contrato: m5-docker-bottom-contract.md. Captura serializada identifica panel=docker.


## M5.11 — Docker sem transição visual no refresh automático

A lateral e ferramenta inferior usam is_foreground_busy para aparência/controles;
is_busy continua incluindo a leitura automática para lifecycle, poll e shutdown.
Refresh periódico não acrescenta progresso nem altera o header/cancelar global.
Feedback de operações explícitas e erros reais permanece. Contrato:
m5-docker-refresh-contract.md.


## M5.12 — Source Control sem aviso permanente de limite

Pedido de26/10/2026: retirar a faixa “Repository scan reached its limit” acima
dos repositórios. discovery_truncated continua no modelo e os orçamentos não
mudam; não há linha nem margem reservada para esse aviso no painel. Contagem
continua sendo de repositórios carregados. Sem nova observação de IDE comercial.


## M5.13 — controles Docker na mesma linha

Detalhes/Logs/atualizar logs compartilham a linha horizontal rolável das ações
do container no painel inferior. Retirada a linha separada; nome e confirmação
continuam em suas posições próprias. Contrato m5-docker-bottom-contract.md.


## M5.14 — nome do container dentro de Detalhes

Nome é o primeiro metadado rolável, antes de Estado/Imagem/ID. Não há título
com nome acima da barra de ações, inclusive em Logs. Confirmação de remoção
continua identificando o container. Contrato m5-docker-bottom-contract.md.


## M3.33 — leitura completa por diretório no Project

Raiz lista todos os filhos. Expansão envia LoadDirectory para o worker, inclusive
por link explícito; não esconde .git/vendor/node_modules/target ou arquivos
ocultos. A lista plana continua preorder com widgets somente no viewport.
Resposta por revisão/geração impede aplicar outra árvore; erros de diretório
ficam no Project. Refresh/watch leem diretórios carregados e preservam estado.
TreeEntry continua compatível, entries passa a representar o cache carregado e
tree_directory_loaded distingue pasta vazia de pasta não consultada.
tree_truncated fica compatível no modelo, falso para listagens completas por
diretório. Contrato m3-project-tree-loading-contract.md substitui o antigo corte
global da árvore, incluindo a parte correspondente do M6.3.

## M3.34 — cores por formato e gutter de folding

O editor identifica formatos por nome completo antes da extensão, inclusive
.env.local/.gitignore/.editorconfig, compartilhando a paleta auditada e os
workers/cache limitados anteriores. Lexer adicional mantém continuação de
comentários/strings XML/Python/Rust/configuração sem provider externo.
Folding JSON usa ranges próprios com fechamento visível; PHP/JS/TS conservam
base lexical delimitada. Controles do gutter pintados e clicáveis fora do recorte
do código, acessíveis e com foco devolvido ao editor/grupo. Ícones são glifos
Codicons originais licenciados, créditos nas Configurações. Sem mutação de
texto/Save implícito ao recolher. Contratos m3-file-colors-contract.md e
m3-json-folding-contract.md; limites e evidências no status.

## M3.36 — revelar o arquivo ativo no Project

Project Options › Behavior › Always Select Opened File liga/desliga o
acompanhamento (padrão ligado). Desligado, Select Opened File usa o alvo junto
de New File. Barra26px sem espaço extra entre ações para caber no painel240px.
Sem nova modal. ProjectFilesUi acompanha somente mudanças do path ativo e da
preferência; não faz scroll contínuo. Expande ancestrais via LoadDirectory,
seleciona e pede scroll da linha lógica, inclusive fora do viewport virtual.
Mudança de documento/workspace invalida intenção anterior; erro/ausência encerra
a tentativa. Inline não é interrompido. O editor não expande pastas por conta
própria ao inicializar um buffer/grupo. Localização manual devolve foco ao editor
ativo com seleção/caret intactos. ProjectPreferences persiste na sessão por
perfil; migração aditiva mantém compatibilidade. LoadDirectory e alteração da
preferência não encerram grupos de Undo. Contrato m3-project-reveal-contract.md.

## Marcas Git e salvamento automático —05/11/2026

O host chama tick_auto_save e tick_git_line_changes após poll. Gutter usa hunks
versionados do índice mais próximo de cada arquivo, inclusive em grupos distintos,
sem depender do painel SCM; pintura é limitada ao viewport. Barra3unidades após
os números, verde/azul e triângulo vermelho para remoção, sem novos alvos de clique
sobre breakpoints/folding. Tokens e procedência em editor-git-token-evidence.json.
Configurações › Editor › Geral contém toggle de salvamento e intervalo em ms;
preferências persistem no perfil. Sair cancela e aguarda ACK dos processos Git de
comparação antes de fechar. Contrato: ../editor-git-autosave-contract.md.

Correção05/11/2026: editor_git_lane_width=10 reserva espaço próprio antes de
navegação/breakpoint/folding, evitando a lupa sobre a faixa Git. Número e faixa
mantidos; coluna estável mesmo sem hunks. Corpus combinado na mesma declaração,
bounds reais e cliques em100/125%, recibo editor-gutter-lanes-receipt-20261105.json.

## Nova janela —06/11/2026

Ctrl+Shift+N e Arquivo › Nova janela chamam o mesmo adaptador do host nativo.
Processo independente, início vazio e perfil próprio, sem reusar a sessão nem
os sidecars da janela original. Repetição automática da tecla não cria janelas.
Falha de spawn aparece no rodapé; buffer, seleção e workspace atuais preservados.
Contrato detalhado: ../new-window-contract.md.

## Linguagem e versão no rodapé —09/11/2026

Contrato vigente: language-version-status-contract.md.
Arquivo ativo sempre conserva linguagem/tipo, com versão declarada apropriada
quando disponível. JS/TS distinguem Node; sem versão permanecem os nomes.
Fallback Texto/Projeto/Sem arquivo impede indicador vazio. Serviço comum em
worker, sem I/O de configuração ou comandos durante pintura; fonte no tooltip,
clip/largura limitada200 e padding/alinhamento do rodapé preservados.
# AI Chat — auditoria de sessão e conversas,10/11/2026

Lista compacta com busca/teclado, largura vinculada ao painel, idade conhecida,
recentes e exclusão no hover/foco. Estados de execução/aprovação permanecem
disponíveis no ponto de estado, tooltip e descrição acessível da linha, sem
segunda linha obrigatória. Escopo efetivo, composer e envio são sincronizados
antes da interação; checkpoint por owner preserva teclas tardias de aberturas
assíncronas. Contratos: docs/ai-chat-session-audit-contract.md,
docs/ai-chat-workspace-scope-contract.md e docs/ai-chat-mentions-contract-20261110.md.
Medidas da referência observada/limites em reference/codex-conversations-20261110.md.
# Quebra automática de linha —10/11/2026

Exibir › Quebra automática de linha, Configurações › Editor › Linhas longas e
Alt+Z no editor compartilham SetEditorSoftWrap. Campo de configuração preserva
foco para Space; atalho não atua em campos de texto do chat/modais/popup. O
padrão ligado é decisão do Codisseu para o fluxo solicitado, não um default
atribuído ao VS Code. Continuação mantém os recuos do editor e reserva lateral
direita; números são das linhas reais. Não inserir newline ou formatar arquivo.
Geometria de cursor/ponteiro/seleção, acessibilidade e rolagem deve usar a mesma
projeção. Contrato: docs/editor-soft-wrap-contract.md; fonte da comparação e
limites: reference/editor-soft-wrap-20261110.md. Aprovação depende dos testes e
capturas reais registrados no status; não se antecipa paridade integral.


Arrastar e mover no Project: contrato de drag/drop. Mesmo worker Transfer
para arquivo/pasta; feedback, hover, autoscroll e cancelamento, sem sobrescrita.


Alinhamento imediato de criação/rename: contrato. Não centralizar
horizontalmente um nome já visível nem a edição inline; manter colunas da árvore.

## Histórico do editor por sequência —10/11/2026

Ctrl+G permanece Ir para linha. Ctrl+Shift+G aguarda vírgula sem abrir Git;
comparação somente leitura recua em pares de versões do arquivo ativo, com
editor/índice/revisões identificados e estado de carregamento/fim/erro explícito.
Repetição funciona na comparação; cancelar/contexto alterado invalida resposta.
Contrato: docs/editor-comparison-contract.md. Referência local VSCode+GitLens,
não alegação de comportamento padrão de toda instalação do VS Code.


Avanço Ctrl+Shift+G,ponto na comparação capturada; endpoint atual compara
HEAD/índice × editor. Cabeçalho usa basename com tooltip do caminho, ações
anterior/próxima/fechar e modo alinhados em barras compactas; área de código usa
espaço restante e renderer compartilhado sem metadata de patch no arquivo inteiro.
Layout reutiliza tokens existentes. Contrato editor-comparison-contract.md;
evidência reference/editor-history-navigation-20261110.md.

Search Everywhere auditado contra PS262 em10/11: prévia de fonte selecionada
usa lexer/shaping do editor, gutter e scroll próprios, sem alterar arquivo.
Clique/duplo clique dependem do preview; toggle preserva seleção; popup possui
atalhos/eventos. Contrato e limites: ../m3-search-everywhere-contract.md;
matriz original: reference/search-everywhere-20261110.md. Não há alegação de
paridade integral com provedores/plugins/ações ainda ausentes.


## Explorer — multisseleção, 10/11/2026

Contrato: Ctrl/Cmd alterna seleção
independente do editor; Shift/CtrlShift usam intervalo e âncora; Ctrl+A visíveis.
Chevron conserva filhos selecionados ocultos. Destaque de grupo e selected
acessível em cada linha, foco separado. Delete/contexto captura lote em diálogo
único, Cancel/Escape preservam arquivos, confirmação usa TrashMany assíncrono.
Raízes/.git/dirty protegidos, ACK parcial explícito; sem novos ativos/dependências.
Resultados e limites no relatório/recibos explorer-multi-20261110.

## 11/11/2026 — método pelo menu Refatorar

Botão direito no editor oferece Refatorar → Renomear símbolo… (Shift+F6),
usando o diálogo, preview, aplicação e histórico semântico existentes. O hit
real do editor captura view/documento/versão/posição escalar; uma seleção maior
pode continuar preservada para Copiar, mas sua outra ponta não escolhe o símbolo.
Mudança de versão desabilita a ação; mudança de view/documento antes do dispatch
é recusada. A preparação não altera o texto nem salva o arquivo.

No rename inline, só a continuação visual que contém o início do identificador
pode posicionar o campo. Outra continuação da mesma linha física não sobrescreve
sua geometria nem cancela a operação por considerá-la fora da área visível.
Testes de menu/origem Unicode em PHP/JS/TS nas duas escalas, seleção ampla,
contexto obsoleto e linha longa em `tests/rename_context.rs`. Escopo/evidências:
contrato e
relatório.

## 11/11/2026 — trilha contextual e operações

Rodapé esquerdo prioriza estrutura local PHP/JS/TS do documento/view ativos;
sem contexto, mostra a mensagem da operação. Mensagem antiga de Git não ocupa
permanentemente o lugar da trilha nem de operação posterior. Ausência esperada
vira “Projeto sem repositório Git”; permissão/timeout continuam erros, com
resumo legível e detalhe original. Escopos vêm de AST no worker, não de nomes
fabricados ou LSP obrigatório. Clique usa navegação versionada e histórico comum,
sem Edit/Save/Undo; prefixo elidido tem menu e tooltips. Alt+Home/Tab/Enter/Escape
mantêm foco, respeitando modais e Browser. Contrato/resultados:
trilha e
relatório.

## 13/11/2026 — arquivos externos e diálogos

Documento limpo acompanha o disco sem uma edição do widget. Conflito real
apresenta Usar versão do disco/Sobrescrever/Cancelar uma vez por revisão;
Cancelar silencia essa revisão e Save explícito pode reabrir. Decisões carregam
versões local/externa, aguardam ACK do worker e mostram falha dentro da modal.
Prompt aguarda outro diálogo; Escape não confirma sobrescrita. Fechar oferece
Salvar e fechar, Descartar e fechar e retorno ao editor, inclusive SaveUntitled
com caminho escolhido e intenção retomada depois do ACK. Autosave fica pausado
durante a decisão; Cancelar nunca equivale a autorizar descarte.

Confirmações de arquivo/caminho/projeto/Lixeira/saída compartilham helpers de
título compacto, frame/padding16, rodapé à direita e spinner16, com os tokens
existentes. Contratos: arquivos externos
e diálogos. Limites e resultados reais
constam no relatório e no status desta sessão.

## Database sem aviso de conexões anteriores —13/11/2026

Pedido posterior do usuário remove o bloco e a ação de vínculo do estado vazio.
Exibir somente o cadastro normal de fontes, com os mesmos tokens e margens.
A existência de metadados legados não acrescenta mensagens, botões ou modais.
Preservar os arquivos antigos e a separação dos dados por projeto.
