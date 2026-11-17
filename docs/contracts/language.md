# Contrato de linguagens — M2

`storm-language` usa apenas std, serde e serde_json; não depende de egui. Não implementa diagnósticos por regex. As mensagens exibidas vêm de processos LSP reais iniciados explicitamente.

## Serviço compartilhado

`LanguageService::new()` inicia somente o worker interno, sem executar provider. `dispatch(LanguageCommand)` valida e enfileira; `poll()` aplica resultados; `is_busy()` representa comandos pendentes e inicialização, nunca o mero fato de um processo estar vivo. A GUI precisa continuar chamando `poll()` quando houver providers Ready, pois a análise acontece depois do ack de sincronização.

Comandos:

- `Start { workspace, configs }`: substitui a sessão LSP, resolve o workspace no worker e inicia os executáveis configurados. Uma única configuração por linguagem, até duas; executável até 4096 bytes e até 64 argumentos somando 32 KiB.
- `Stop`: envia shutdown/exit, aguarda e recolhe os processos; invalida os diagnósticos da sessão.
- `Sync { path, version, text }`: UTF-8 completo de documento aberto, até 1 MiB, até 32 documentos, versão monotônica positiva no intervalo integer LSP. Roteia PHP e TS/TSX/JS/JSX por extensão. Caminhos canônicos precisam estar sob o workspace.
- `Close { path }`: envia didClose e remove os resultados associados.

`ProviderConfig { language, program, args }` é serializável, com `Language::Php` / `Language::TypeScript` (JSON `"Php"`/`"TypeScript"`). Não há concatenação com shell. O arquivo de configuração é um array JSON. `ProviderStatus { language, state, detail }` usa `Stopped`, `Starting`, `Ready`, `Failed`. Ausência de executável, erro de transporte e timeout são erros explícitos.

`Diagnostic` contém path, version opcional, linha/coluna inicial/final base zero em **UTF-16**, mensagem, severidade LSP e linguagem. `utf16_byte_offset` converte para byte UTF-8, rejeitando posição no meio de um par surrogate. Não é coluna visual nem índice de grafema.

`analyzed_versions[path]` registra a versão do cliente associada ao publishDiagnostics aceito, incluindo lista vazia. **Não é confirmação do servidor quando `unversioned_paths` contém o caminho.** Esse conjunto público marca publicações sem versão, inclusive listas vazias. Novo Sync limpa imediatamente diagnósticos e ambas as marcas. A fila do receiver confere a versão atual e o epoch da sessão antes de aplicar a resposta; respostas antigas versionadas são descartadas. Falha/parada invalida os resultados da linguagem.

`dispatch(Sync)` aceita a fila, sem prometer entrega ao provider. Falhas posteriores são publicadas em `sync_failures: HashMap<PathBuf, SyncFailure>` com `version` e `message`, além de `last_error`. O consumidor pode remover a entrada ao tratá-la, invalidar sua marca de envio e tentar novamente a mesma versão com limite de tentativas. Novo Sync/Close/Start/Stop limpa as entradas correspondentes; falhas de uma versão anterior não se aplicam à atual. Se a sincronização inicial falhar durante o handshake, o provider passa a Failed e precisa de reinício explícito.

## Limite do push sem versão

Phpactor 2026.10.07.0 enviou versões nos testes reais; typescript-language-server 6.0.2 enviou `version: None`. Para este último, o broker associa o evento ao último didOpen/didChange **enfileirado no transporte** para esse documento e o receiver revalida a versão atual. A marca é inferida: o protocolo push sem versão não permite provar que uma resposta tardia corresponde ao texto mais recente quando várias edições se sobrepõem. `Diagnostic.version` permanece None e não é inventada. O teste real confirmou correção e limpeza depois de uma edição controlada; robustez diante de edições concorrentes sem versão permanece M3. O serviço nunca aceita diagnósticos de arquivos não abertos ou de outra linguagem/sessão.

## Transporte e recursos

Framing Content-Length, cabeçalho até 8 KiB e corpo até 4 MiB, JSON UTF-8. Header duplicado, ausência de tamanho, tamanho excessivo e corpo incompleto são erros. Ordem: initialize → resposta/capacidades → initialized → didOpen/full didChange. Somente UTF-16 é anunciado; outro encoding respondido é rejeitado. Full changes são enviados mesmo quando o servidor anuncia incremental; essa variante é válida no formato LSP, mas não há diff incremental nesta etapa.

Fila de comandos 32, eventos 128, entrada por processo 32 e saída por processo 8. Pipes têm threads próprias para não bloquear a GUI/coordenador em I/O; backpressure é limitado. stderr fica separado em ring buffer de 32 KiB, sem publicação automática em logs/UI. Diagnósticos: até 200 por documento e 2048 caracteres de mensagem; exceder 200 atualiza o status com truncamento explícito. Ambiente dos processos é reduzido a HOME/USERPROFILE/PATH/SYSTEMROOT/TMPDIR/TEMP/XDG_CACHE_HOME, LANG e XDEBUG_MODE; variáveis de tokens/senhas não são herdadas por padrão.

Initialize tem limite de 15 segundos. Encerramento espera resposta shutdown até 500 ms e término após exit até 500 ms por provider; sobreviventes são mortos/recolhidos. Em Unix cada provider cria grupo de processos e os sobreviventes do grupo (incluindo tsserver) são encerrados. Windows/macOS não foram exercitados. `Drop` sinaliza encerramento e delega o join a um reaper separado, sem esperar na thread chamadora; a GUI deve enviar Stop e aguardar o ack no encerramento normal. Finalizar todo o processo abruptamente antes do reaper concluir não fornece garantia de cleanup.

Na M2 não havia restart automático, rename nem WorkspaceEdit. Rename/WorkspaceEdit foram acrescentados na M3.6 abaixo; execução de scripts e endpoint de rede próprio continuam ausentes. Requisições de configuração retornam valores nulos; registros dinâmicos e métodos não implementados retornam erro de método, sem anunciar sucesso falso. A confiança no projeto deve anteceder o comando explícito Start: providers podem ler configurações/plugins do workspace, embora Storm não rode npm/Composer automaticamente.

## Evidência

Testes `protocol::*`: framing, URI com espaço/acentos/emoji, rejeição de frames inválidos, UTF-16. Testes `missing_provider_is_a_visible_failure_not_ready` e `invalid_workspace_terminates_starting_state`: falhas observáveis. Teste separado `real_php_and_typescript_diagnostics_and_clear_with_unicode`: providers reais, erro PHP e TS, posição após emoji, correção não salva, limpeza, descarte de comando obsoleto, parada. Ele é explicitamente ignorado na suíte comum e executado separadamente; ausência de ferramentas na suíte real causa falha, nunca skip silencioso.

## M3.2 — hover versionado

Hover/CancelHover, HoverResult, hover_error e hover_pending seguem o
contrato M3.2. Capacidade estática negociada,
correlação ID/epoch/versão, uma consulta atual, cancelRequest best effort,
timeout 10s e limite 32 KiB. MarkedString/MarkupContent são texto inerte;
null é ausência de documentação. O intervalo opcional não é utilizado.
Teste de transporte usa Python 3 já disponível como provider controlado,
somente na suíte de testes; não é dependência do produto. Testes externos
PHP/TS continuam separados e exigem STORM_PROVIDER_CONFIG.

11/11/2026: o núcleo também expõe `PreviewHover`, mesma consulta versionada,
para documentação passiva por mouse. Não encerra grupos de edição ou cancela
assinatura/conclusão/refatoração independentes; erros especulativos não tomam o
status do usuário. O transporte LSP permanece Hover/CancelHover. Teste real
cross-file, PHPDoc, completion e assinatura usa CODISSEU_PROVIDER_CONFIG.

## M3.3 — definição

Definition/CancelDefinition, DefinitionTarget/Result e estados públicos seguem
contrato M3.3. Capacidade estática, correlação
ID/epoch/versão, timeout10s, cancelamento e no máximo64 destinos locais
validados. Location/LocationLink/null são decodificados; URI externa ou qualquer
resultado inválido rejeita o conjunto. Sync de qualquer buffer invalida a consulta.
Testes de transporte controlado e providers reais são separados.

## M3.4 — completion

Completion/CancelCompletion e CompletionResult/Item/Edit seguem o
contrato M3.4. Capacidade estática, IDs próprios,
versão/posição/epoch, cancelamento e timeout10s. Até128 itens,64KiB por texto de
edição,16 edições adicionais e limites de512KiB por resultado normalizado.
TextEdit/InsertReplaceEdit/itemDefaults preservam intervalos validados;
insert_edit/text_edit distinguem insert/replace. Snippets, comandos e instruções
inseguras ficam explicitamente inaplicáveis. isIncomplete/truncated são distintos.
Não há completionItem/resolve nem execução de commands nesta fatia.

## M3.5 — referências e indexação inicial

References/CancelReferences e ReferencesResult seguem
contrato M3.5. referencesProvider estático,
context.includeDeclaration, Location[]/null, correlação ID/epoch/versão/posição,
deadline10s, cancelamento e até256 destinos locais validados. Truncamento descreve
o limite do cliente; não comprova completude semântica do provider. Link e Location
singular são inválidos neste método. Sync de qualquer buffer invalida a consulta.

O cliente anuncia workDoneProgress e responde window/workDoneProgress/create.
Phpactor identificado pelo serverInfo phpactor/phpactor permanece Starting até
begin/end correspondentes de Indexing workspace, sem Ready falso. Desde M3.32,
reports do mesmo token renovam o prazo de inatividade de120s; limite total30min.
O status mostra mensagem limitada a200caracteres. Progresso alheio/malformado
não renova o prazo. Documentos aguardam a barreira antes do envio. Não se usa diagnóstico nem
espera fixa como confirmação de índice. Outros providers mantêm o handshake comum.

O artefato typescript-language-server configurado é identificado pelo manifesto
local limitado16KiB, no worker. Nessa versão, busca iniciada num uso pode incluir
declaração apesar de includeDeclaration=false. O adaptador consulta definição
real numa fase privada, depois referências; remove somente destinos canônicos e
intervalos idênticos. Ambas as fases compartilham contexto/deadline/cancelamento.
Provider genérico não recebe essa adaptação. Não há filtro por regex/nome.

## M3.6 — preparação e proposta de rename

PrepareRename/Rename/CancelRename negociam capacidades estáticas e transportam
UTF-16, versões, IDs/epoch e deadlines10s. WorkspaceEdit é somente proposta;
workspace/applyEdit é recusado. Caminhos, formatos, limites e operações
não suportadas falham integralmente. PHP retorna versões positivas para buffers
abertos e0 para arquivos fora do Workspace do Phpactor nesta versão; TLS retorna
changes sem versões. O núcleo não transforma0 em versão inventada: verifica
snapshot integral dos arquivos fechados. Ambos providers aceitaram colisão
com método existente no corpus; não há garantia semântica de ausência de conflito.
Detalhes/evidências: M3.6.


M3.7: o provider continua resolvendo símbolos/edições, mas seu sucesso não aprova
conflitos. storm-syntax faz verificação AST independente de membros da mesma
classe no worker. Relatório diferencia Verified/Rejected/Unsupported; versões e
limites do transporte permanecem. Não afirma typechecking ou cobertura global.
Ver m3-rename-conflicts-contract.md.

## M3.8 — relações simples conhecidas

O LSP continua produzindo as edições. A análise AST resolve cadeias nominais PHP
(namespace/use) e TS (imports relativos nomeados/export direto) no corpus limitado.
KnownHierarchyMemberNamespaces não certifica binding, tipos nem resolução completa
do compilador. InheritedMember e OverrideChanged impedem aplicar alterações de
relação detectadas; override completo e casos incertos ficam Unsupported.
Contrato.

## M3.9 — equivalência explícita de overrides

Contratos primitivos iguais em métodos concretos da cadeia conhecida podem ser
verificados antes/depois. `checked_override_pairs` conta pares únicos, inclusive
quando ambas as declarações são propostas pelo LSP. Não é typechecker nem prova
de binding de chamadas. Propostas parciais e contratos incertos continuam
bloqueados. Escopo exato.

## M3.10 — declarações relacionadas

RenameValidation.related_methods identifica path/range UTF16/classe/nome das
declarações alteradas em pares certificados. A coleta deduplica e ordena, sem
sintetizar edição. Menos de duas declarações alteradas não abre seleção. Núcleo
conserva cópia privada e repete verificação no Apply. Escopo de tipos/corpus M3.9
continua, sem interfaces/traits/overloads/binding global.
Contrato.

## M3.19 — DiagnosticTag.Unnecessary

Cliente anuncia tagSupport.valueSet[1]. Diagnostic.unnecessary tem serde(default)
e exige tag1 inteiro em array válido limitado a8itens; ausência/malformação não
fabrica fading. Intervalos UTF16 invertidos/surrogates inválidos são rejeitados.
Limites/versionamento/epoch/limpeza existentes mantidos; decoração exige marca
analyzed_versions atual. TS sem versão ainda não prova causalidade de cada
publicação sob Syncs sobrepostos. Contrato.

## M3.22 — propostas relacionadas e snapshots fechados

Uma proposta que contém todas as declarações relacionadas pode ainda omitir
chamadas: no corpus `fixtures/rename-contracts`, Phpactor consultado a partir da
classe concreta altera ambas as declarações PHP, mas omite a chamada cujo
parâmetro é tipado pela interface. Consultar a declaração da interface retorna
essa chamada. O teste guarda ambas as respostas reais; a proposta incompleta
executada numa cópia descartável falha com método indefinido.

Depois da primeira validação nominal, o núcleo consulta `textDocument/rename`
em cada declaração certificada antes de publicar o preview selecionável. São
no máximo 32 consultas sequenciais e 30 segundos para a expansão inteira, além
do timeout de 10 segundos por requisição. O worker une exclusivamente intervalos
reais dos providers: duplicatas idênticas são deduplicadas, sobreposições ou
versões divergentes abortam o conjunto. Novas declarações fora do conjunto
original também abortam. Todos os buffers mantêm suas versões; o manifesto do
corpus e os bytes originais dos arquivos fechados são revalidados após cada
consulta. Não há síntese de chamadas por busca textual. A cópia privada do
relatório impede que alterações no estado público autorizem ou corrompam a fase.

`LanguageCommand::RenameClosed` aceita caminho canônico fechado no workspace,
snapshot exato limitado a 1 MiB, posição UTF-16 e novo nome. O worker confirma
os bytes em disco, envia `didOpen` temporário com versão 1, consulta rename e
envia `didClose`. Essa abertura pertence apenas à requisição: não acrescenta
documento/aba, versão pública, diagnósticos ou histórico. Um documento realmente
aberto é recusado nessa operação; seu buffer não salvo usa `Rename` comum.
O resultado conserva versões originais de cada edição e registra separadamente
`transient_document_version = 1`; `version = 0` identifica a requisição sobre
snapshot fechado, sem inventar revisão do editor. O núcleo valida a versão
temporária exclusivamente contra esse snapshot antes da união.

Sucesso, erro do provider, versão inválida, cancelamento e timeout encerram a
abertura temporária. Se a fila de envio estiver cheia, o worker conserva a
abertura pendente e repete o fechamento; nenhuma nova operação pode adotá-la,
nem o resultado ser publicado como sucesso enquanto o fechamento falhar.
Stop/Drop recolhem o processo conforme o contrato existente. Respostas tardias
são descartadas por ID/epoch e por qualquer Sync/Close/CancelRename.

Nomes novos passam pelo candidato Unicode XID de `codisseu-syntax` e pela
validação exata do identificador na gramática PHP/TS; não são normalizados.
`precipitação` foi exercitado com ambos os providers, aplicação/Undo/Redo/Save
e execução PHP/compilação TypeScript. Consultar todas as declarações do conjunto
certificado não equivale a provar binding global, reflexão, autoload dinâmico,
aliases/tipos arbitrários ou referências fora do corpus limitado.

### Configuração TypeScript e propostas parciais

O corpus adicional `fixtures/rename-contracts/multiple` reproduz uma interface
com duas implementações em arquivos distintos. Com `multiple/*.ts` incluído no
`tsconfig.json`, o provider retorna três declarações e uma chamada, e o núcleo
valida/aplica/desfaz/refaz/salva as quatro alterações. Com esses mesmos arquivos
fora de `include`, a consulta na interface retorna somente sua declaração;
cada consulta isolada numa implementação retorna interface e essa implementação,
mas continua sem a chamada do arquivo também excluído. Apenas abrir/consultar
esse último arquivo altera o conhecimento do provider. Os logs preservam as
quatro propostas distintas; não se assume que todas as referências foram vistas.

Por isso, a expansão atual exige que a primeira proposta já contenha todas as
declarações da família nominal verificada. Uma proposta parcial permanece
`Rejected/OverrideChanged`; consultar automaticamente somente as declarações
faltantes não provaria que as chamadas também foram encontradas. O produto não
altera `tsconfig`, abre todos os arquivos ou inventa edições para superar esse
bloqueio. Comparação nativa equivalente exige o mesmo escopo de projeto das
fixtures; falhas anteriores de configuração permanecem registradas.

## M3.23 — ativação e prova de referências

O desktop habilita a política de ativação automática externa ao projeto. Na
primeira pasta/configuração exige confiança; próximas aberturas confiáveis
iniciam providers sem Start. AppService::new/Restore isolados continuam sem
executar processos até EnableProviderAutostart/Start explícito. Stop suspende
a tentativa atual e seu ACK não pode ser substituído por status antigo.
Contrato da política.

A proposta Rename agora passa pelo dispatcher lexical e pela auditoria de
receptores. MissingReference permite somente consultas LSP relacionadas para
completar referências, sem liberar Apply parcial. Vínculos desconhecidos e
conversões estruturais sem prova bloqueiam; nenhum novo método é inventado.
Contrato de binding e
variáveis/parâmetros locais.


## M3.35 — implementação e preview de navegação PHP

Implementation tem os mesmos campos path/version/line/character de Definition,
negocia implementationProvider estático e usa textDocument/implementation.
Compartilha IDs/epoch/slot/timeout10s/cancelamento e validação64destinos locais.
DefinitionResult.presentation é opcional/serde(default), destinado apenas ao
seletor: source_symbol e até64 labels nome/namespace/tipo/visibilidade obtidos
por AST PHP no worker. Snapshots abertos precedem disco; orçamento250ms/4MiB/
64arquivos. Falha de label conserva o destino e exibe caminho, sem alterar sua
resolução. Listas ordenadas com metadados alinhados. Não executa autoload/PHP.
Adaptador Laravel de definições não é aplicado a respostas de implementação.
Contrato/UI/evidências em m3-php-navigation-contract.md e status.md.

## Correções PHP/JS/TS —30/10/2026

[Contrato vigente](../quick-fixes-contract.md): CodeActions/Resolve/Cancel usam
o provider da linguagem ativa e tokens privados por dono. Diagnósticos originais
validados/limitados seguem o contexto UTF-16. Mesmo serviço de preview/Apply e
Undo/Redo/Save do núcleo; nenhuma execução genérica de command. InlayHint segue
restrito a Rust. Guia [quick-fixes-usage.md](../quick-fixes-usage.md).

## 11/11/2026 — referências de métodos em projetos JavaScript

O fallback TypeScript usa `tsconfig.json` ou `jsconfig.json` raiz, com defaults
do compilador para cada nome. JavaScript sem configuração usa o host inferido
observado do servidor configurado; `checkJs` não é habilitado silenciosamente.
Propostas sem diagnósticos de tipos continuam exigindo a conferência exata de
bindings e referências. Nome/bytes/ausência da configuração são revalidados na
prova privada; configuração dirty ou alterações invalidam a aplicação.

`missingReferences` contém somente path/range UTF-16 comprovados pelo compilador,
sem edições. Exclusivamente conflitos `MissingReference` sem outros problemas
podem disparar novas consultas `Rename`/`RenameClosed`. A união usa as edições
LSP e snapshots originais e repete a prova completa, com até 32 consultas únicas
em 30 segundos. Uma origem por arquivo é enfileirada a cada rodada; referência
persistente ou outro conflito continua bloqueando Apply. Não gerar substituições
textuais a partir dos metadados. Contrato do compilador.

## Estrutura local para a trilha — 11/11/2026

`codisseu-syntax::breadcrumbs` calcula contenção sintática PHP/JS/TS/JSX/TSX com
as gramáticas já declaradas. Não exige provider, não resolve símbolo externo e
não inventa nome semântico de parâmetro em chamada. Worker recebe snapshot do
buffer, com limite, deadline/cancelamento e resposta de geração atual. A conversão
de todos os limites byte→escalar é linear no texto; movimento de caret consulta
somente o índice já produzido. Contrato.

## Parada dos pipes Unix — 12/11/2026

Os três pipes do provider usam descritores não bloqueantes, com espera por
readiness somente quando o kernel retorna WouldBlock. Leituras e escritas
parciais mantêm os offsets do framing; o coordenador continua recolhendo o
processo principal antes de interromper suas threads de transporte. Depois do
reaping, `alive=false` encerra também leitor/stdout, stderr e writer/stdin sem
aguardar EOF de descritores que algum descendente fora do grupo ainda retenha.
A espera de readiness verifica o cancelamento a cada50ms; isto não representa
um prazo para descartar ACKs de saves ou transações. Reaping de processo em
estado de kernel não interrompível continua sem garantia de prazo. O fallback
não Unix permanece com pipes std; Windows não foi exercitado neste incremento.

Regressões próprias cobrem pipe vazio sem EOF, pipe de escrita cheio, framing
Unicode maior que a capacidade do pipe e Connection.stop depois da saída do
principal com stdout/stderr retidos por descritores externos. Evidência e
limites: relatório.
