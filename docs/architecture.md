# Arquitetura — M0/M1 e M2 em implementação

Decisão 2026-10-20: Cargo workspace modular com dependências em uma direção. `codisseu-editor` mantém texto, Unicode e histórico. `codisseu-core` contém documentos, comandos e serviço de aplicação; filesystem/sessão roda em worker. `codisseu-ui` apresenta o serviço em egui; `apps/codisseu` inicia/configura a janela e oferece entrada CLI. M2 acrescenta `codisseu-db` e `codisseu-language`, consumidos por `codisseu-core`, sem dependência de egui. Não há motores de editor separados por linguagem.

Historicamente M1 usou TextEdit como componente provisório; texto e histórico pertencem ao domínio. Buffer Ropey e cache de texto permitem trocar o widget na M3. Limites explícitos reduzem amplitude nesta etapa; não há virtualização do editor nem alegação de edição profissional completa.

I/O de arquivos ocorre em worker, com filas limitadas e versões de documentos. Salvar escreve snapshot e detecta conflito externo; retorno tardio não limpa edições novas. Encerrar exige resolver documentos alterados e aguardar persistência da sessão. Estado tem schema; nenhum script roda simplesmente ao abrir workspace. Não existe serviço de rede de automação.

`codisseu-db` é implementação própria sobre rusqlite/SQLite, com uma conexão por worker, comandos tipados, resultados limitados, cancelamento e gerações para invalidar respostas antigas. A fatia M2 presente abre banco existente em modo somente leitura e consulta schema/dados; edição de células, commit/rollback e bancos remotos não são capacidades concluídas dessa fatia. Não existe dependência de runtime do DataGrip. A decisão do usuário substitui a proposta histórica de extrair QueryGrip: DataGrip é a referência pesquisada, não a origem do código Rust.

`codisseu-language` implementa transporte LSP/JSON-RPC por stdio com processos externos PHP/Phpactor e Node/TypeScript language server. Filas e documentos têm limites; sessões e versões invalidam diagnósticos antigos; conversões de posição distinguem UTF-16 de bytes UTF-8. A ativação dos providers é explícita, com falhas visíveis. Isso não equivale a paridade de análise, refatoração ou execução com PhpStorm/WebStorm. Configuração e instalação local ficam em `tools/install_providers.py` e `.tools/`; versão do PHP do host deve ser conferida para reprodução.

Referências: Rota A. Na M2, paleta Dark de fonte pública IntelliJ Platform com revisão fixa e Inter/JetBrains Mono OSS com notices; mapeamentos e limites em `visual-reference-m2.md` e `design/token-evidence.json`. Shell PHP/Web usa organização documentada do PhpStorm; exploração/resultado SQL usa fluxos públicos DataGrip. A composição na janela Storm é própria, sem equivalência visual comprovada.

egui/eframe/egui_kittest usam a mesma versão, lockfile e Rust fixados após consulta de metadata/documentação oficial. Backend wgpu nativo Wayland/X11. Testes, falhas de integração e capturas reais permanecem em `status.md`; a existência das crates não marca a M2 como concluída.

## M3.1

AppService coordena também `storm-search`, worker independente de filesystem,
com filas limitadas e gerações de cancelamento. `codisseu-editor::search` contém
busca literal KMP e montagem da substituição; o serviço revalida ID/versão antes
de aplicar pela operação Edit e seu histórico. GUI não calcula a busca nem salva
substituições diretamente. Detalhes em `m3-search-contract.md`. TextEdit permanece
ponte provisória; não se declara editor definitivo ou parsing incremental.

## M3.3

`codisseu-core::navigation` coordena pedidos LSP e abertura assíncrona validada.
Captura origem e versões dos buffers, escolhe destino, envia Work::Navigation ao
worker de filesystem e só ativa após revalidar texto/UTF-16. Buffers abertos não
são recarregados. Histórico de posições é independente do histórico de edições.
`codisseu-language` transporta Definition com capacidades, IDs e cancelamento;
`codisseu-ui::navigation` traduz menu/teclado e apresenta seleção de destinos.
Tudo passa pelos mesmos comandos tipados; nenhuma ação de navegação salva.

## M3.4

`codisseu-core::completion` mantém contexto versionado, opções e seleção. Aceitação
monta edições em Work::Completion no worker limitado de arquivos, sem I/O de
documento nem save. A resposta só aplica por Edit se o token/versões continuam
atuais; preserva undo e outras edições não salvas. `codisseu-language::completion`
decodifica instruções de edição do provider e recusa capacidades não implementadas.
`codisseu-ui::completion` mantém somente seleção/âncora visual e traduz menu/input.

## M3.5

`codisseu-core::references` coordena a consulta, snapshots e cache limitado de texto;
Work::ReferencePreviews/ReferenceOpen fazem leituras/validações fora da GUI.
Escolhas usam a seleção e o histórico comuns de navegação, sem salvar ou trocar
buffers abertos por disco. `codisseu-ui::references` apresenta a lista persistente,
preview e controles próprios. `codisseu-language` espera a indexação inicial real
do Phpactor e adapta a exclusão de declarações do wrapper TS por uma consulta
semântica de definição dentro do mesmo contexto versionado de referências.

## M3.6

`codisseu-language::rename` decodifica preparação e WorkspaceEdit com limites e
sem executar workspace/applyEdit. `codisseu-core::rename` mantém contexto, guarda
versões, solicita Work::RenamePreview/RenameVerify e só altera buffers após
preflight do conjunto. Arquivos fechados são abertos com histórico antes/depois;
arquivos dirty conservam conteúdo atual. Última transação pode ser desfeita/
refeita em grupo sob guards de todos participantes, sem gravação implícita.
`codisseu-ui::rename` apresenta nome e proposta, sem fazer edições por conta própria.
A M3.6 não possuía análise própria de colisões. A camada adicional da M3.7 abaixo
substitui o aviso genérico por verificação local e bloqueio explícito.

## M3.7

`codisseu-syntax` é puro, sem GUI/I/O: gramáticas Tree-sitter reais, snapshots antes/
depois e reparse incremental por edição. Retorna escopo, conflitos e motivos de
Unsupported com coordenadas anteriores UTF-16. `Verified` cobre apenas o espaço
de nomes de membros da mesma classe; não equivale a resolver binding ou tipos.

Work::RenamePreview inclui buffers abertos como contexto; Work::RenameVerify
repete análise e verificações de caminho/texto. AppService conserva proposta
privada, versões e relatório separado; não aplica Rejected/Unsupported e não
permite contornar o worker alterando o relatório público. A UI apresenta o
bloqueio e a fonte anterior em `rename/conflicts`, sem editar por conta própria.
Contrato: m3-rename-conflicts-contract.md. Não há índice completo persistente do
workspace nem análise global de herança/typechecking nesta fatia.

## M3.8

codisseu-core::worker coleta um corpus limitado de fontes no workspace fora da UI.
RenameCorpus privado guarda candidatos e snapshots; Apply reenumera e revalida
contextos antes do commit atômico nos buffers. codisseu-syntax::hierarchy constrói
identidades nominais/imports e relações AST puras, sob orçamento de memória/tempo.
O grafo verifica nomes relacionados antes/depois, sem executar I/O nem substituir
o provider. É uma análise local conservadora; não é índice persistente nem
typechecker. Contrato.

M3.9 acrescenta codisseu-syntax::contracts: comparação conservadora de cabeçalhos
primitivos explícitos, sem resolução de tipos, inferência ou dependência da GUI.
A mesma verificação pura roda na proposta e no Apply. O relatório conta pares
únicos certificados; isso não amplia o corpus nem substitui edições do LSP.
Contrato.

## M3.10

AST entrega metadados das declarações certificadas, não uma seleção fabricada por
nome. RenameState mantém fase Selecting e escolhas privadas; comandos tipados
exigem confirmação da cadeia completa antes da revalidação/commit existentes.
A UI em rename/related renderiza/revisa esse estado sem aplicar edições.

codisseu-editor introduz TextDelta/EditKind, validação atômica, mutação incremental
do rope e grupo temporal no domínio. A GUI ancora o delta no cursor resultante
para texto repetido e retém cache de versão quando aceito. Domínio permanece sem
egui. TextEdit/layout/String integral ainda limitam o editor; esta é a primeira
fatia da substituição, não conclusão. Contrato.


## M3.11

codisseu-editor possui Selection e HistoryEntry com texto/seleção antes/depois,
independentes de egui. codisseu-core expõe EditTransaction/SetSelection versionados;
completion e rename usam os mesmos endpoints. codisseu-ui::editor isola cache,
sincronização TextEdit e renderização do código do shell. A ponte ainda usa
String/layout integrais; isto prepara sua substituição, sem declarar editor
definitivo ou desempenho para projetos grandes. Contrato.


## M3.12 — editor/viewport próprios

`codisseu-ui::editor` agora contém input, cache vertical, tiles horizontais, estado
lexical em worker e árvore acessível. O buffer continua único no domínio, sem
egui. Snapshot Rope não exige String; seleção pertence ao documento e a geometria
é medida pelo shaper. A composição gera somente um snapshot visual transitório.
TextEdit não renderiza código. Limites atuais e evidências em
m3-editor-viewport-contract.md e [status](status.md).

## M3.13 — identidade Codisseu e views

O rename ativo abrange executável, crates, símbolos, IDs egui, app-id e lançador.
Os nomes acima identificam a árvore atual; os nomes Storm nos artefatos são
históricos. `codisseu-core::views` contém IDs e seleções por grupo/documento e
remapeia endpoints por TextChange com spans escalares de Edit/Undo/Redo.
Um Document/TextBuffer/histórico continua único. A GUI guarda apenas viewport,
scroll/cache/IME e composição de dois grupos, com clips e IDs próprios.
Ativar grupo passa por Activate, encerrando digitação e cancelando semântica
obsoleta; Save/LSP/SQL usam o documento da view ativa. As views não são persistidas.
`editor::delimiters` aplica assistência limitada por EditTransaction/SetSelection;
pares gerados são rastreados por view e descartados em revisões desconhecidas.
Contexto lexical multilinha vem do worker; trabalho síncrono tem orçamento16K.
Pares e matching não pretendem substituir um parser semântico. O caso PHP heredoc
é conservador e conserva o sufixo sem destaque em vez de inventar contexto.

## M3.16 — composição e posições persistidas

`codisseu-core::view_session` concentra EditorSplit e serialização versionada de
grupos/posições, sem egui ou texto. O worker valida referências a arquivos antes
de aplicar a sessão; o AppService reconstrói IDs e seleções escalares. GUI mantém
cache/IME e usa o scroll do serviço, retornando offsets limitados pela viewport
por SetViewScroll. Os novos comandos não encerram grupos de digitação nem mudam
versões/LSP. Isso substitui a limitação histórica de views somente por execução;
lista global de abas e limite de dois grupos continuam.

## M3.17 — membership local

DocumentView guarda tabs ordenadas e posições apenas dos membros. AppService
separa CloseTab (instância) de Close (documento canônico) e ActivateTab valida
pertencimento. Fechar divisão mescla instâncias únicas, sem perder dirty. Worker
continua único para I/O; view_session migra editor_state1→2 e valida listas antes
de reconstruir IDs. Somente última instância fechada libera buffer e sessão LSP.
O editor mantém cache/IME por par view/documento e descarta pares sem membership.


## M5.5 — debug comum

`codisseu-debug` mantém DAP e workers independentes por sessão, sem egui.
`AppService` expõe comandos e integra o polling; uma sessão pausada/rodando não
marca a IDE inteira como busy. Configurações/breakpoints usam worker de perfil
externo com schema e gravação atômica. Esse I/O participa da espera de saída,
mas não da restrição que exige sessão vazia para RestoreSession. A GUI guarda
somente apresentação, seleção, expressão/draft/autorização temporários.
Fonte/editor/Undo/Save são os mesmos dos outros recursos. Socket Unix próprio,
processos/limites/epochs e encerramento estão no contrato m5-debug-backend.md.

## AI Chat —25/10/2026

codisseu-ai mantém transporte JSONL, subprocessos, filas limitadas, máquina de
estados e transcrição normalizada, sem egui. AppService oferece Command::Ai,
polling e isolamento por workspace. UI guarda apresentação/composer/contexto
anexado; nenhum processo inicia ao abrir o painel. Um processo por turno retoma
sessão explícita do mesmo CLI; custos de startup não foram benchmarkados.
Leitura e escrita de pipes rodam fora da GUI; escritor separado permite cancelar
mesmo quando o CLI para de consumir stdin. Cleanup recolhe processo/grupo próprio
no Unix antes do estado final. Encerramento nativo espera o serviço terminar.
Claude é adaptador de print/stream-json com ferramentas explicitamente limitadas;
não se afirma execução real ou sandbox equivalente ao Codex. Detalhes/limites em
ai-chat-contract.md.

## M3.31 — encoding na fronteira de arquivos

FileEncoding pertence ao documento no núcleo. Worker resolve preferência por
raiz canônica e recorte universal de EditorConfig; leitura estrita gera Unicode,
e gravação estrita retorna bytes sem normalizar texto. Perfil separado da sessão
permite lembrar projetos após trocar de pasta e usar --no-restore. Reload tem
barreira de comandos, preflight completo e recibo; watcher recebe o codec efetivo
do snapshot para não reinterpretar fontes com a configuração de outro projeto.
Rename transporta encoding certificado no recibo do worker, antes de inserir
arquivos fechados como documentos. Não há nova dependência ou segundo editor.

## API-01 — cliente HTTP nativo por projeto,26/10/2026

codisseu-api fornece modelos de coleções/pastas/requests/environments/specs/mocks,
worker de persistência e transporte reqwest, importadores independentes e ciclo
de scripts em processo Node gerenciado. AppService possui ApiService; OpenWorkspace/
Restore/CloseWorkspace conciliam a identidade antes de expor dados. Filas e
gerações impedem resposta de projeto anterior. API tabs são recursos tipados
na faixa comum do editor egui; não são buffers de código artificiais. Os
documentos existentes conservam LSP, Save/Undo e grupos. Estado de coleções
persiste fora das fontes por hash SHA256 da raiz canônica, com schema e
checagem da identidade original. Segredos declarados/token capturado ficam
na sessão. Contrato e limites: api-client-contract.md e api-client-usage.md.

## Workspace com vários projetos —27/10/2026

A pasta do workspace organiza a janela e ferramentas compartilhadas; cada
Document agora tem project_root resolvido no worker. Marcadores e overrides
explícitos definem fronteiras sem executar manifests. Encoding é escolhido nessa
fronteira/por EditorConfig do arquivo, preservando codecs e bytes independentes.
A árvore permanece lazy, sem indexação completa para descobrir contextos.

ProjectLanguages conserva a API LanguageService ativa e até sete serviços em
cache por raiz/configuração. Troca de aba cancela propostas semânticas, troca
serviço e mapas de versões, sem criar worker novo no cache hit. Indexação só
começa após confiança e demanda; um agregador reconhecido vazio não é indexado
como projeto único. Stop/encerramento alcançam e drenam todos os caches.

Git mantém descoberta/status/índice/commit por repositório; APIs permanecem no
workspace, Docker por contexto e bancos por datasource. Contrato:
[multi-project-workspace-contract.md](multi-project-workspace-contract.md).


## Navegador integrado —02/11/2026

`codisseu-browser` contém comandos tipados, protocolo IPC2 e transporte local
sem GUI/CEF. A GUI apresenta frames OSR em texturas WGPU e encaminha input;
abas, toolbar, endereço, erros e revisão de debug permanecem Rust/egui. O helper
`apps/codisseu-browser-engine` é outro workspace Rust, com lock próprio e SDK
CEF pinado. Só ele linka libcef e cria renderers. Falha do helper não derruba
o processo principal e não inicia um segundo motor de editor.

O broker só lança o helper após Open explícito e fica bloqueado sem sessão.
IPC Unix privado tem peer PID, perfil e permissões locais, tamanhos/filas/bytes
limitados e assinatura por aba/epoch. Frames são coalescidos e convertidos fora
da GUI; somente superfícies visíveis mantêm textura. O laço nativo CefRunMessageLoop
processa o clipboard/X11; tarefas CEF encaminham comandos limitados na thread UI.
Objetos entregues por FFI guardam somente IDs, Arc/atômicos e metadados imutáveis;
sinal ready usa Arc<AtomicBool>. Rc/RefCell de State/pump ficam em TLS e são
resolvidos somente após currently_on(UI), sem clonagem/destruição de Rc por
getters ou refcounts estrangeiros. Request/IO usa metadata thread-safe. Ctrl+V passa texto
literal do clipboard do host; uma resposta tardia pertence à superfície que a
solicitou e é descartada se ela fechar, protegendo o editor recém-focado.

Core coordena debug e DevTools fora da GUI. Cada conexão explícita usa ponte WS
loopback efêmera com nonce256bit, Host/Origin e uma aba/epoch. Não existe porta
CEF global nem descoberta com o segredo. O gateway do adapter faz roteamento
Target virtual explícito; respostas Runtime/Debugger/Page vêm da página real.
O modo do frontend é canal de página. O SDK DevTools é servido em origem privada
somente leitura, sem API do host; detalhes de bootstrap e procedência pertencem
aos recursos do helper. As duas conexões são mutuamente exclusivas por aba.
Fechar/Stop revoga imediatamente o canal; listener e processos próprios são
recolhidos fora da thread gráfica. Perfil de debug integrado usa schema3 e não
persiste tab ID, URI/token ou autorização; releases antigos o recusam.

Contratos: browser-contract.md e browser-ipc-contract.md. A aceitação registrada
em status.md distingue testes funcionais, screenshots e compilação; não amplia
a certificação a outras plataformas ou a todo recurso de um navegador pessoal.

## Inspetor frontend nativo —02/11/2026

O módulo `codisseu_browser::inspector` mantém um worker de CDP por sessão, com
comandos tipados, revisão/document epoch e snapshot latest coalescido. Core arbitra
os modos nativo, SDK DevTools e DAP da mesma aba; trocar ou fechar libera o dono
anterior e retoma eventual pausa. A GUI empresta o snapshot ao desenhar e envia
ações depois, sem copiar corpos de rede por frame ou executar I/O na thread gráfica.
O inspetor não abre outra porta de rede nem acessa APIs de host pela página.

Rede, cookies/storage e debugger vêm do browser real. Limites são aplicados a
objetos, filas, árvores e corpos; resposta de rede é lazy e exige tamanho conhecido
seguro. A árvore DOM usa caminhada própria limitada no isolated world da página
principal, sem acesso universal, e conserva referências opacas da sessão.
Não é um modelo interno de React/Vue. Avaliação explícita e watches usam o contexto
normal ou frame pausado selecionado; enumeração de propriedades não chama getters.

Painel, divisor, abas, formulários e erros usam egui/Tokens Codisseu; dados sensíveis
ficam ocultos por padrão e não entram nos diagnósticos genéricos. Navegação invalida
seleções e dados temporários. JWT é somente decodificado localmente, sem verificar
assinatura. Contratos browser-frontend-inspector-contract.md e
browser-inspector-backend.md; aceitação funcional e visual em status.md.
