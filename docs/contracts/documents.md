# Contrato de documentos e filesystem — M1, M2 e primeira fatia M3

`storm-core::AppService` é a única entrada de comandos para GUI, CLI e testes.
`dispatch` confirma validação/enfileiramento; conclusão de I/O chega por `poll`.
O chamador mantém `poll` enquanto `is_busy()`; só encerra depois do ack de sessão.
Não há egui nem execução de scripts no núcleo. IDs de documento são estáveis na
sessão; reabrir a aplicação atribui novos IDs e restaura paths/aba ativa.

## Documento

Texto UTF-8 até16MiB e1.000.000linhas canônicas, sem NUL; até32documentos.
Buffer Ropey1.6.1 canônico; String é cache lazy invalidado ao editar/desfazer/
refazer, sem materialização integral por frame. Snapshots são COW. Histórico no
domínio: até128entradas e32MiB de bytes UTF8 alterados por pilha. Cada comando Edit aceito é uma etapa de undo. M3.10 adiciona EditDelta
versionado com intervalo escalar, remove/insert no rope e agrupamento de
digitação/deleção contígua por até2s entre eventos; EndEditGroup separa cursor/
foco. Colagem/newline/Save/navegação/ações semânticas/Undo/Redo encerram grupos.
A política2s é própria; não foi medida como timeout do PhpStorm. `expected_version` impede escrita de uma edição
obsoleta. Versão cresce em edição, undo e redo; dirty compara com texto salvo.
Conversões escalar Unicode ↔ UTF-16 rejeitam meio de par substituto. São índices
absolutos, ainda não protocolo LSP nem segmentação de grafemas. CRLF, LF, BOM e
ausência de newline final são preservados pelo núcleo.

## I/O

Um worker, filas de no máximo 32 pedidos/respostas, `try_send` na interface.
Leituras, varredura, gravações e restauração acontecem fora da thread da GUI.
A M3.33 substitui o antigo corte5000/depth16: a árvore carrega todos os filhos
imediatos da raiz e de cada pasta expandida, sem exclusão por nome/ocultos.
Entradas ordenadas, pastas primeiro; links aparecem e só são seguidos por
expansão explícita, sem recursão automática. `entries` é cache preorder de
pastas carregadas, não índice recursivo do workspace. Contrato:
m3-project-tree-loading-contract.md.
CancelAll
invalida leituras e solicita cancelamento cooperativo antes do commit de saves.
Filesystem calls individuais não são interrompidos à força. Um save que já
commitou tem seu ack aplicado mesmo após cancelamento, pois descartar o ack
falsificaria o estado do disco. Drop sinaliza cancelamento e desconecta canais;
não bloqueia GUI aguardando filesystem. Encerramento normal deve aguardar idle.

Abrir canonicaliza caminhos. Salvar verifica conteúdo externo duas vezes,
recusa destinos substituídos por symlink e hard links múltiplos, preserva
permissões, escreve tempfile no mesmo diretório, sincroniza e renomeia.
Em Linux a troca atômica protege de escrita parcial, não constitui bloqueio
contra processos concorrentes entre a última verificação e rename. Não há ainda
lock cross-process, journal de recuperação, preservação de ACL/xattrs nem
garantia testada de crash/power loss ou de Windows/macOS. Aviso de falha de
sync do diretório após rename significa conteúdo gravado com durabilidade
incerta. Erros anteriores preservam buffer e original. Edição feita durante
save permanece dirty se divergir do snapshot efetivamente gravado.

## Sessão

JSON schema 2 guarda workspace, paths das abas, path ativo, caminho SQLite e
configurações locais dos providers, sem conteúdo dos documentos ou credenciais.
O leitor também aceita schema1. Restauração não conecta banco nem executa providers. Não há journal de unsaved buffers na M1. Close dirty
sem `discard=true` é rejeitado. Troca de pasta requer documentos limpos e
preserva abas ao atualizar a mesma pasta. PersistSession requer idle. Restore
requer sessão vazia e lê tudo antes de aplicar; erro preserva arquivo de sessão
e impede sobrescrevê-lo. Cancelar a restauração também não autoriza sobrescrita.
Arquivos ausentes geram erro visível. A GUI pede
confirmação antes de descartar mudanças e antes de sair, ou salva e aguarda ack.

## Evidência desta implementação

Em 20/10/2026, `cargo test -p storm-editor -p storm-core --locked` passou:
15 testes de filesystem/sessão e 3 testes de buffer, sem testes ignorados.
`cargo clippy -p storm-editor -p storm-core --all-targets --locked -- -D warnings`
e formatação dos dois crates também passaram. Os testes usam arquivos reais em
diretórios temporários, não simulam respostas. GUI é validada em suíte separada.

## Integração M2

AppService é dono de DbService e LanguageService sem dependência da GUI. `Database`, `ExecuteSql`, `LoadProviderConfig`, `ConfigureProviders`, `StartProviders` e `StopProviders` são comandos tipados compartilhados por widgets, inicialização CLI e testes. O arquivo de configuração (até128KiB) é lido no worker. Executar SQL valida ID/versão do documento e seleção em índices escalares, nunca offsets UTF-8 confundidos com seleção.

O polling sincroniza versões novas de documentos compatíveis com providers Ready. Falhas assíncronas de Sync preservam caminho/versão; há até duas novas tentativas por versão, depois erro explícito. Respostas versionadas/epochs antigos são descartados; a limitação do push TS sem versão está em `language.md`. Troca de pasta e operações de banco/início LSP são serializadas para impedir uma conexão antiga de aparecer no workspace novo. Trocar a raiz limpa conexão, diagnósticos e documentos limpos; apenas atualizar a árvore da mesma raiz preserva a sessão.

Evidência M2: `tests/workspace_capabilities.rs` cobre os conflitos de enfileiramento; `tests/fullstack.rs` executa FULL-01 com providers reais. Resultados finais em `../status.md`.

## Busca e substituição — primeira fatia M3

`Find { id, expected_version, query }`, `ReplaceMatches { id, expected_version,
query, replacement, selection }` e `CancelSearch` usam o mesmo AppService.
`selection: Some(range)` é uma ocorrência exata em índices escalares; `None`
substitui todas. Query literal, sensível a maiúsculas e sem sobreposição.
Busca vazia retorna zero resultados; substituição com query vazia é recusada.

Worker separado com no máximo 8 pedidos sem acknowledgement, cancelamento
cooperativo e descarte por geração/versão. `search` expõe `SearchResult` com
`document_id`, `version`, `query`, `ranges` e `truncated`; `search_error` informa
falhas. `search_pending()` cobre busca corrente e substituições pendentes;
`is_busy()` também aguarda o descarte dos pedidos cancelados. `CancelSearch`
invalida somente Find e preserva substituições aceitas/I/O. `CancelAll` e troca
de workspace invalidam também substituições. Troca de aba limpa Find; uma
substituição aceita continua vinculada ao ID original, nunca à nova aba.

A lista contém até 10.000 matches, com truncamento explícito. Replace-all
recalcula todos no worker; verifica tamanho final até 1 MiB e ID/versão antes
de aplicar uma única edição/undo. Query até 64 KiB; substituição até 1 MiB;
ambas sem NUL. Não salva implicitamente, e nenhuma alteração ocorre se a
versão mudou. Falhas de substituição também ficam em status/last_error para
sobreviver à busca automática posterior. No-op não cria versão nem histórico.

Contrato detalhado, limites e cenários: M3 busca.
Validação do núcleo/editor nesta fatia: 34 testes aprovados (27 core + 7 editor),
zero falhas, um FULL-01 externo reservado; clippy sem avisos. Busca global,
regex, grafemas visuais e refatoração semântica não fazem parte desta fatia.

## M3.2 — consulta de documentação

Hover { id, expected_version, character_index } valida o documento e traduz
cursor escalar em UTF-16, sincronizando conteúdo não salvo antes do pedido LSP.
CancelHover e CancelAll invalidam a consulta. Sem edição nem save implícito.
Contrato de estados/limites: M3.2.

## M3.3 — navegação entre documentos

Definition/ChooseDefinition/CancelDefinition e NavigateBack/Forward passam pelo
AppService, com versão/cursor escalar. `navigation` expõe espera/escolhas/erro e
NavigationSelection versionada. O worker valida caminho e só lê destinos ainda
fechados; buffers abertos são preservados. Núcleo converte UTF-16 contra o texto
atual antes de ativar a aba; cancelamento invalida inclusive erros tardios.
Histórico64 por pilha, sem persistência nem reaplicação de cursor de versão antiga.
Contrato/limites: M3.3.

## M3.4 — aplicar sugestão

Completion/AcceptCompletion/CancelCompletion passam pelo AppService. Contexto
captura caret e versões de todos os buffers; alterações invalidam resultado.
AcceptCompletion recebe replace=false (Enter) ou true (Tab). Worker monta todas
as edições não sobrepostas contra snapshot não salvo; revalidação precede Edit.
Uma aceitação é um único undo, nunca save. Duplicata durante aplicação é recusada
sem cancelar a edição já aceita. Selection pública versionada devolve caret à GUI.
Detalhes e limites: M3.4.

## M3.5 — lista de referências

References/ChooseReference/CancelReferences compartilham AppService. O worker
prepara contexto e linha do uso contra os buffers atuais ou leituras limitadas,
com cache8MiB. Uma escolha não altera texto/undo e mantém a lista; publicar a
seleção usa o histórico comum de navegação. Arquivo fechado é comparado por texto
integral ao snapshot do preview antes de abrir. Mutações/cancelamento descartam
inclusive falhas tardias; caminho externo e UTF-16 inválido não abrem abas.
Contrato completo: M3.5.

## M3.6 — rename entre documentos

PrepareRename/PreviewRename/ApplyRename/CancelRename compartilham AppService.
Worker prepara todos os textos e revalida arquivos fechados antes de aplicar
um conjunto indivisível aos buffers. Não salva. Undo/Redo agrupam a última
renomeação se todos os participantes conservam versões/textos esperados; edição
intermediária invalida o grupo e preserva os históricos individuais. Não existe
journal global persistente. Contrato: M3.6.


M3.7: análise de conflitos usa snapshots atuais dos buffers; proposta privada e
versões guardadas impedem aplicar relatório alterado ou contexto obsoleto. Um
bloqueio não cria entrada de histórico nem salva. Nome corrigido usa o mesmo
commit de buffers e undo/redo agrupado da M3.6. Ver m3-rename-conflicts-contract.md.

## M3.8 — contexto de herança

A enumeração limitada inclui classes PHP/TS fechadas no workspace; buffers abertos
têm prioridade, inclusive não salvos. A proposta privada guarda o manifesto
ordenado. Apply reenumera candidatos, confere inclusão/remoção e relê contextos
fechados, além das versões de todos buffers abertos. Mudança ou limite bloqueia
commit; não há Save implícito. Undo/redo conserva o contrato M3.6.
Escopo e limites.

M3.9 conserva os mesmos snapshots privados e a revalidação M3.8 ao aceitar
cadeias completas com contratos primitivos iguais. Alterar um buffer relacionado
após preview invalida a operação. Aplicação e undo/redo agrupados abrangem também
o override inicialmente fechado, sem Save implícito. A suíte externa exige
bytes exatos e três participantes previamente dirty por linguagem.

## M3.10 — seleção privada e deltas do editor

SelectRenameMethod e ConfirmRenameMethods exigem token, versões e fase corretos.
A cadeia certificada é privada; Apply só segue depois de confirmar todos os seus
métodos. Desmarcar um bloqueia confirmação, sem filtrar chamadas por heurística.
Cancel preserva buffers/disco; revalidação e histórico agrupado continuam M3.9.

EditDelta valida intervalo/NUL/tamanho antes da mutação, preservando redo em erro.
O delta da GUI é ancorado no cursor resultante para evitar deslocar inserções em
texto repetido e romper agrupamento. Cache String do widget ainda existe; não há
virtualização, restauração geral do cursor no Undo/Redo nem editor definitivo
concluído. Contrato desta fatia.


## M3.11 — histórico dirigido

Selection {anchor,head} em escalares vive no TextBuffer. EditTransaction valida
versão, delta e endpoints antes da mutação; SetSelection valida versão/limites
sem incrementar versão de texto nem descartar redo. HistoryEntry guarda snapshots
antes/depois e suas seleções; navegação posterior não sobrescreve o destino do
Redo. Digitação sobre uma seleção pode iniciar grupo que inclui as próximas
letras; nova seleção nunca se agrega ao grupo anterior. Colagem continua atômica.
Completion guarda o destino final da aplicação, incluindo additionalTextEdits.
Rename mapeia seleção por cada edição UTF16 real, em ordem descendente, antes de
commit; Undo/Redo agrupado conserva endpoints em todos os participantes.
Texto/seleção são restaurados juntos em um passo Storm. Limites128entradas/8MiB
lógicos por pilha somam ambos os snapshots. Contrato.


## M3.12 — limites independentes do editor e linguagem

Editor abre/salva16MiB e1.000.000linhas, incluindo a última vazia. Worker valida
UTF8/NUL/tamanho/linhas antes de publicar Open. Edit/Paste validam Rope candidato
antes de mudar texto/seleção/versões/identidade/grupo/redo. Nada é truncado.
Histórico conserva128entradas/32MiB de bytes UTF8 alterados por pilha; snapshots
Rope são COW, sem somar o documento inteiro a cada tecla.
Save envia snapshot Rope, identidade e referência compartilhada do conteúdo salvo;
serialização/gravação ficam no worker. Conclusão tardia reconhece só o snapshot
realmente gravado; edição nova continua dirty. Detecção dirty por identidade de
histórico é constante; comparação Rope após outro ramo fica lazy por revisão.

LSP/hover/definition/completion/referências/rename mantêm1MiB por documento,
inclusive leitura de arquivos fechados. Grande documento não é clonado para Sync;
provider já sincronizado recebe Close e status explícito. Busca literal aceita
saída até16MiB; substituição de entrada conserva1MiB. Grafemas UAX29 e palavras
Unicode usam unicode-segmentation1.13.3 sobre snapshots; conversoresLSP seguem
UTF16. Contrato atual.

## M3.15 — preferência de ligaduras

`SetEditorLigatures(bool)` altera `EditorPreferences` do AppService, sem editar
buffers, seleções, versões, dirty ou histórico. Padrão ativado, conforme pedido do
usuário. `editor_preferences()` devolve uma cópia somente leitura. A preferência
é comum aos grupos/documentos, não pertence ao projeto. PersistSession inclui o
campo opcional `editor_preferences` no schema2; Restore aplica-o após validar toda
a sessão. Schemas1/2 antigos ou campo ausente recebem padrão ativado; bool inválido
falha sem sobrescrever a sessão. I/O continua no worker e aguarda ack no fechamento.
Com --no-restore a aplicação inicia com os padrões, incluindo esta preferência.
Configuração persiste com a sessão do perfil; ainda não é um arquivo de preferências
independente da restauração. Contrato/corpus.

## M3.16 — metadados dos grupos na sessão

SetEditorSplit/SetViewScroll atualizam somente metadados e preservam agrupamento
de digitação. EditorSplit e posições escalares/scroll não dependem de egui. Campo
opcional editor_state (schema1) na sessão2 salva até dois grupos e32 posições por
grupo; caminho deve pertencer aos documentos já validados/carregados pelo worker.
Estado inválido bloqueia restauração e sobrescrita, sem abrir caminhos adicionais.
Seleção é limitada ao texto atual; conteúdo não salvo e histórico não são
serializados. Sessões antigas continuam compatíveis.
Contrato atual.

## M3.17 — listas locais de abas

DocumentView inclui lista ordenada tabs, apenas IDs de documentos canônicos.
ActivateTab/CloseTab operam sobre um grupo; fechar cópia conserva buffer/Undo/LSP.
Última instância aplica Close global com sua proteção de dirty/save pendente.
Fechar divisão mescla abas únicas no sobrevivente. Novo Open acrescenta ao grupo
ativo; reabrir caminho existente prefere uma instância já aberta. Editor_state2
persiste membership/ordem e valida união das listas contra os arquivos carregados;
editor_state1 migra listas globais. Não se recuperam textos não salvos/históricos.
Contrato completo.

## M3.23 — política desktop e Rename local

Restore no núcleo continua uma leitura; o desktop habilita separadamente
automação de providers em pastas/configurações já autorizadas no perfil externo.
A sessão não conecta SQLite automaticamente. O contrato da política está em
m3-provider-autostart-contract.md.
Rename local/métodos usa a mesma atomicidade de buffers/Save/Undo/Redo existente,
com prova lexical/de receptores em m3-rename-binding-contract.md.

## Seleções múltiplas literais — 23/10/2026

EditBatchTransaction aplica deltas escalares independentes ao snapshot original,
valida versão/seleção/limites/não sobreposição antes da mutação e gera uma versão
com Undo/Redo atômicos. Mapas esparsos forward/inverse preservam seleção das views
passivas entre intervalos editados. Texto entre ocorrências permanece compartilhado
no Rope; limite de histórico conta bytes alterados. UI mantém secundárias por view,
limita512 e restaura os estados reconhecidos do histórico; seleção primária continua
canônica para LSP e acessibilidade. Ctrl+D é busca literal, não refatoração semântica.
Contrato completo: m3-selection-productivity-contract.md.

## M3.31 — encoding de disco por projeto

O requisito histórico de UTF-8 apenas é substituído por
m3-encoding-contract.md. Buffer e posições continuam
Unicode, mas Document conserva FileEncoding real. Latin-1 e Windows-1252 usam
codecs reversíveis; salvar valida representabilidade e compara bytes do baseline
antes de gravar. BOM UTF-8 é preservado. Preferência por raiz canônica persiste
independentemente da sessão, em sidecar externo. Troca exige buffers limpos e
recarrega sem Save; falhas conservam buffers/perfil. Leituras semânticas do worker,
restauração, Git e watcher compartilham a política. Não certifica a interpretação
de arquivos legados pelos servidores LSP externos ou todos os padrões EditorConfig.

## Salvamento automático —05/11/2026

EditorPreferences inclui auto_save=true e auto_save_delay_ms=1000 por padrão,
com migração aditiva das sessões. SetAutoSave liga/desliga e valida250–60000ms.
O host interativo chama tick_auto_save depois de poll; o tick observa versões e
quietude de todos os buffers, então enfileira o mesmo Save protegido. Comandos de
edição/Undo/formatação/refatoração continuam buffer-only quando usados sem o tick.
Untitled/read-only/missing não são salvos automaticamente. Falha pausa tentativas
por documento até ACK de Save manual; ACK de snapshot anterior conserva dirty da
edição nova. Timer não é uma operação pendente de I/O nem bloqueia is_busy.
Contrato/corpus: editor-git-autosave-contract.md.
# Quebra automática de linha —10/11/2026

EditorPreferences.soft_wrap é uma preferência global de apresentação, ligada
por padrão para Markdown e todos os formatos do editor compartilhado. Perfis
schema1/2 sem o campo adotam true; false explícito persiste. O comando
SetEditorSoftWrap não cria edição, não muda versão/dirty/seleção, não altera
newlines/encoding e não abre um passo de Undo nem interrompe um grupo de digitação.
A projeção e os caches de linhas visuais pertencem à view, sem entrar no Rope,
LSP ou conteúdo salvo. Cada divisão usa sua própria largura/scroll/cursor.
Contrato e corpus: docs/editor-soft-wrap-contract.md; resultados em docs/status.md.

Com alturas visuais calculadas sob demanda, pixels sozinhos não restauram a
posição de leitura. Cenário reproduzido: setenta linhas longas, scroll longe
do caret, Persist/Restore deslocava a fonte visível da linha4 à69. Cada posição
de aba/view pode guardar SourceScrollAnchor opcional (índice escalar canônico
e offset vertical lógico). É metadata sem conteúdo/edição; deltas remapeiam o
índice e restauração limita-o ao tamanho atual do documento. Perfil antigo sem
âncora continua aceito. Offset inválido é recusado preservando o perfil.
SetViewScroll explícito invalida a âncora anterior quando os pixels mudam;
SetViewScrollAnchor publica a posição fonte do mesmo documento/view depois do
layout. Renderer novo/reflow usa essa âncora e não revela caret que estava fora.

## Navegação estrutural local — 11/11/2026

`NavigateToRange` recebe documento, versão e intervalo escalar, verifica documento
ativo e ambas as posições e só então registra origem no histórico de navegação.
A GUI aplica a seleção pelo fluxo comum de NavigationSelection; nenhuma mutação
de fonte ou Save pertence à trilha. Views do mesmo documento compartilham os
intervalos do snapshot atual, mas cada uma fornece seu próprio caret. Dados de
inspeção antiga são descartados na mudança de documento/versão/tipo.

## Alterações externas e resolução explícita — 13/11/2026

Documentos limpos acompanham mudanças externas sem ganhar dirty. Se disco e
buffer já coincidem, o baseline é reconhecido sem escrever novamente. O mesmo
vale quando Save chega antes da rodada do watcher: seu ACK reconcilia o estado
limpo ou publica o conflito real, preservando os dois conteúdos.

`document_external_change(id)` expõe a revisão opaca e o caminho do snapshot
externo; o texto fica privado no núcleo. `ReloadDocument` e `OverwriteDocument`
exigem ID, versão local e revisão externa esperadas. Recarregar relê no worker,
revalida identidade/conteúdo e verifica novamente a versão local no ACK antes
de substituir o buffer. Sobrescrever usa o mesmo Save atômico, com os bytes do
snapshot externo escolhido como condição obrigatória das duas verificações.
Uma nova mudança externa publica outra revisão; a decisão anterior não vale
para ela. Cancelamento/erro preservam disco e buffer. Recarregar conserva Undo.

O watcher não aplica leituras capturadas durante Save/recarga pendentes; o ACK
reconhece o conteúdo realmente observado ou gravado. `document_save_pending(id)`
permite à UI aguardar o documento sem depender de outros serviços ocupados.
Auto-save pausa documentos com conflito conhecido até resolução explícita ou
reconciliação; nenhum tick tenta sobrescrever uma revisão nova automaticamente.
Leituras de resolução conferem metadata antes/depois e rejeitam destino não
regular ou caminho que virou symlink. Permanecem os limites de concorrência
entre última verificação e rename já descritos no contrato de I/O.

Cenários e contrato de UI: external-files-contract-20261113.md.
# Abertura de projeto durante operações — atualização11/11/2026

`workspace_change_blocker` expõe o motivo tipado de espera ou documentos dirty;
`OpenWorkspace` verifica novamente as mesmas guardas ao aceitar a operação.
A UI mantém uma intenção identificada pela origem e aguarda os ACKs, em vez de
perder a seleção diante de ocupação temporária. Validação de destino e aplicação
continuam assíncronas. `cancel_workspace_open` invalida leituras pela geração;
gravações aceitas conservam seus recibos. Nenhum buffer é descartado para abrir
uma pasta inválida. Contrato detalhado:
workspace-close-contract-20261111.md.

## Sessão por projeto e âncora de perfil —13/11/2026

`new_with_profile(session_path, profile_anchor)` separa a sessão de abas da
janela dos dados persistentes de banco/IA/API. `new(path)` mantém ambos no mesmo
perfil, preservando isolamento de testes. O host normal usa âncora estável;
Nova janela mantém JSON de abas independente. Sem alvo CLI/desktop retoma a
última sessão normal; alvo explícito prevalece e `-n` inicia vazio.

Banco e API usam pasta canônica ou arquivo `.code-workspace` como identidade;
IA conserva a raiz declarada de cada conversa. A troca aplica ACKs e limpa
estado transitório. Cadastro remoto global antigo exige vínculo explícito no
projeto correto. Abas de queries sem vínculo ou pertencentes a outro projeto
são omitidas da restauração, com cópia privada da sessão completa antes de
persistir a composição filtrada. Arquivos e credenciais antigos são preservados.

Fechamento aguarda gravações aceitas, incluindo transferências; progresso só
aparece após750ms. Confirmações de dirty/transação e falhas reais continuam
explícitas. Contratos/corpus: project-session-contract-20261113.md,
project-database-scope-contract-20261113.md e project-ai-http-scope-contract-20261113.md.

## Remoção do aviso de vínculo —13/11/2026

A atualização posterior do usuário removeu esse fluxo do painel Database. O
status de restauração não solicita vínculo; consultas antigas permanecem
preservadas fora da janela quando não pertencem ao projeto atual.
