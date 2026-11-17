# M5.5 — serviço nativo de depuração

Contrato do incremento solicitado em 24/10/2026. O núcleo é próprio, em Rust, sem GUI, JVM ou host VS Code. A interface comum usa DAP; o adaptador PHP converte DAP para DBGp/Xdebug. Não é uma certificação de todas as funcionalidades do PhpStorm/WebStorm.

## API e execução

`codisseu-debug` expõe `DebugService::{execute,poll,is_busy,has_active_sessions}`, comandos tipados e snapshots de sessões. Cada sessão possui um worker independente; PHP e Node/Chrome podem permanecer pausados simultaneamente. IDs de sessão não são IDs de thread/frame DAP. `SelectThread`, `SelectFrame`, `Variables` e `Evaluate` são sempre vinculados à sessão escolhida. `stop_epoch` avança a cada evento `stopped`, inclusive quando a execução volta à mesma linha/frame, permitindo atualizar watches.

Configuração: runtime PHP/Node/Chrome, Launch/Attach/Listen, executável, programa, cwd, argumentos separados, ambiente explícito, porta, URL, source maps e mapeamentos. `trusted=false` bloqueia início. Configuração não instala ferramentas nem executa hooks ao abrir uma pasta. O serviço não usa shell. O ambiente do adaptador conserva somente variáveis de sistema/runtime enumeradas, mais o ambiente explicitamente passado ao programa. Não imprime ambiente nem registra tráfego DAP por padrão.

- PHP Launch: executávelPHP com Xdebug instalado; ativa `xdebug.mode=debug`, `start_with_request=yes`, hostloopback e porta escolhida pelo adaptador. Não altera php.ini.
- PHP Listen: não inicia/adota o processoPHP externo. Porta padrão 9003, `listen_host=127.0.0.1` por padrão. Outro endereçoIP, por exemplo `0.0.0.0`, requer configuração explícita autorizada. `path_mappings` usa raiz do servidor → raiz local.
- Node Launch: Node 24 local disponibilizado no projeto, programa/argumentos/cwd, console interno, source maps e bootstrap DAP independente. Sessão só vira Running quando o target efetivo terminou a inicialização.
- Node Attach: Inspector em127.0.0.1, porta padrão 9229. Stop desconecta sem terminar o processo externo.
- Chrome Launch/Attach: adaptador aberto de navegador, URL/webRoot, executável e argumentos; perfil de debug independente gerenciado pelo adaptador. Porta de Attach padrão 9222. Uma URL de desenvolvimento já servida é responsabilidade da configuração/projeto; não inicia npm/dev-server automaticamente.
- TypeScript: códigoJS compilado com source maps mapeia breakpoints/frames ao `.ts`. Não há compilação ou instalação automática de dependências. Node 24 também pode ser escolhido para os projetos que usam seu suporte nativo aTS, mas esse modo não foi certificado pelo corpus atual.

Os mapeamentos PHP são correspondências de raízes; source-map overrides JS/TS são os padrões do adaptador quando a configuração está vazia e o mapa explícito do usuário quando fornecido. A aplicação informa limitações e erros do adaptador; não cria valores simulados.

## Transporte e ciclo de vida

PHP usa stdio com framing `Content-Length`. EmUnix, JS usa socket local dentro de diretório temporário próprio 0700, removido no encerramento. Não abre uma porta DAP de rede. O fallback nãoUnix está preparado com loopback, sem certificação funcional/plataforma nem fronteira equivalente de permissões. Node/Chrome usam seus protocolos nativos de inspeção conforme o adaptador; PHP Listen só expõe o endereço explicitamente configurado.

O servidor JS pode solicitar `startDebugging`: o worker cria outra conexão DAP ao mesmo adaptador e inicializa o target pendente. Não inicia VS Code. `runInTerminal` não é anunciado; programas rodam no console interno. Requests e respostas têm correlação por conexão/seq; epochs descartam respostas de frames/variáveis/evaluação anteriores após passo, continue ou seleção diferente. ACK de controle atrasado não apaga a pilha recém-pausada.

Stop é restrito à sessão. Launch termina processos próprios; Attach/Listen usa `terminateDebuggee=false`. Os testes verificam externamente que Node/PHP continuam vivos após desconexão. EmUnix, cada adaptador inicia em grupo de processos próprio; cleanup final elimina apenas esse grupo, recolhe o adaptador e remove seu socket. A flag de encerramento independe da fila, inclusive quando cheia. Prazo de desconexão 1,2s; startup/request 30s e conexão inicial 5s. Processos externos de Attach/Listen não pertencem ao grupo próprio.

PHP Listen permanece disponível para requisições sucessivas; a saída de uma thread PHP não encerra o listener. O adaptador PHP recente oferece Pause por control socket quando Xdebug/plataforma/localização o permitem; conexões remotas e versões antigas podem recusar a operação com erro explícito. JS/Chrome oferecem Pause real. Evaluate executa código no processo pausado, sendo uma ação explícita do usuário.

## Limites de recursos

- 8 sessões ativas;32 snapshots de sessões retidos.
- 512 breakpoints globais;16 conexõesDAP internas por sessão.
- Fila de 32 comandos por sessão, 8 snapshots/eventos compartilhados e 256 requests pendentes por worker.
- Mensagem DAP de 8 MiB; cabeçalho8KiB; stack 100 frames, threads 128 e scopes 32.
- Primeira página de variáveis 200; expansão sob demanda; 128 referências, 10.000 valores e 4 MiB de strings por sessão. Expansões antigas são removidas com aviso explícito quando o limite é atingido.
- Output de 1 MiB / 512 entradas; truncamento de cada entrada 64 KiB e aviso de descarte das anteriores. Avaliações de 1 MiB / 100 entradas, expressão de 16 KiB e resultado de 64 KiB. Strings truncadas têm marcador.

Esses limites são política do produto, não valores observados no PhpStorm. Não representam uma medição de p95, uso mínimo de memória ou desempenho de todos os programas/frameworks.

## Dependências e procedência

Manifesto reproduzível: [tools/debug-adapters.json](../tools/debug-adapters.json). Instalador explícito: `python3 tools/install_debug_adapters.py`; verificação sem escrita: `python3 tools/install_debug_adapters.py --verify`. Somente biblioteca padrão Python; verifica tamanho/SHA256 do download e hashes do entrypoint/licença, rejeita traversal/symlinks de arquivos e não executa lifecycle scripts. Destino `.tools/debug`, sem instalação global ou atualização automática.

- xdebug/vscode-php-debug 1.40.2, commit 5cee2626f227e0ebe88df2c03a1a88b4b4295173, MIT, VSIX oficial. Executa somente `extension/out/phpDebug.js`, não o host/extensão UI.
- microsoft/vscode-js-debug 1.140.0, commit bccbbafdcb6bb96ab0706f411a1e66681d5bc4de, MIT, standalone DAP oficial, entrypoint `src/dapDebugServer.js`.

Fontes primárias verificadas em 24/10/2026: [PHP adapter](https://github.com/xdebug/vscode-php-debug/tree/v1.40.2), [JS standalone](https://github.com/microsoft/vscode-js-debug/tree/v1.140.0), [configuração JS](https://github.com/microsoft/vscode-js-debug/blob/v1.140.0/OPTIONS.md), [DAP](https://microsoft.github.io/debug-adapter-protocol/overview.html). `documented`: os repositórios descrevem DAP/DBGp, distribuição e opções. `observed`: os entrypoints oficiais efetivamente executaram os cenários de fixture abaixo. Nenhum código/ícone proprietário foi incorporado. Imagens distribuídas nos arquivos dos adaptadores permanecem no pacote externo; não são ativos da UI Codisseu.

Licenças e avisos retidos em [reference/public/debug-adapters](../reference/public/debug-adapters/README.md). O manifesto inclui os 18 pacotes nomeados encontrados no PHP VSIX e seus avisos; dependências transitivas contêm MIT/ISC/BSD-2-Clause/BlueOak-1.0.0. Isso não é uma conclusão automática de que toda biblioteca de qualquer distribuição futura terá a mesma licença. O bundle JS conserva seus avisos compilados; o pin é da distribuição inteira.

## Validação

Resultado final registrado em backend/results.json: **19 testes distintos aprovados, zero falhas e zero ignorados** (2 unitários, 3 de validação controlada e 14 com adaptadores/runtimes reais). Casos reais usam somente fixtures próprias, temporários e perfis de Chrome isolados; nenhum projeto/banco empresarial, credencial ou VPN.

- `CARGO_INCREMENTAL=0 cargo test -p codisseu-debug --all-targets --locked --jobs 1 -- --test-threads=1`: saída 0: 2 unitários + 17 de integração aprovados; execução final de integração 16,03s.
- `CARGO_INCREMENTAL=0 cargo clippy -p codisseu-debug --all-targets --locked --jobs 1 -- -D warnings`: saída 0, sem avisos.
- Instalador: download/extract em diretório temporário, verify, adulteração do entrypoint e rejeição pelo hash passaram; temporários removidos. Instalação real `.tools/debug` verificada sem alterações.

Ambiente observado: PHP 8.4.27/Xdebug 3.5.3, Node 24.22.0, compilador TypeScript local e Google Chrome instalado, Linux. Imagens/GUI do Codisseu e referência PhpStorm são de responsabilidade do corpus integrado da UI; este documento não confunde compilação com teste funcional nem cria screenshots de backend.

Casos de runtime aprovados: PHP+Node simultâneos; PHP+TypeScript no Chrome simultâneos; breakpoints JS e PHP; valores 42/evaluate; StepIn/StepOut e stepOver; Pause Node e PHP via control socket; source map Node TS e Chrome TS; mapeamento remoto→local PHP; desconexão de Attach/Listen preservando processos externos; listener com duas requisições sucessivas; breakpoint não verificável; término inesperado Node confirmado externamente com exit 7; loop na mesma linha com epochs/valores novos; Drop com fila de comandos cheia. No PHP, o adaptador habilita retorno observável (`breakpoint_include_return_value`): StepOut pode parar na linha de retorno antes de outro StepOut chegar ao chamador.

Os 3 casos de validação controlada são lista/mute/limite de breakpoints, falta de adaptador e bloqueio de configuração não autorizada/programa ausente; não são anunciados como execução de programa real. As 2 unidades verificam framing Unicode e rejeição de mensagem excessiva. A execução final usa worker SHA256 `e93a728e68b36da271094b0d29314b3d902d814f37b74c4c3339b7f56f38ccb0`; hashes de todos os fontes e versões dos runtimes constam no JSON. Tentativas anteriores identificaram/corrigiram thread ID 0 válido em JS, bootstrap ainda não pronto para Pause, chegada de ACK após parada, estado final Node Attach e asserções que ignoravam os stops reais de retorno/breakpoint PHP. Logs finais são os vinculados, sem somar reruns como testes novos.

Limites: Chrome Attach está implementado por configuração, porém o corpus de navegador usa Launch; não há certificação de Attach Chrome, Docker/container real, Xdebug remoto por TLS/túnel ou servidores de produção. O cenário de path mapping usa duas raízes temporárias equivalentes, sem Docker. PHP Pause depende do control socket acessível; um servidor em outro host/container pode recusá-lo. O tipo de retorno de um processo pode ficar desconhecido (`exit_code=None`) quando o adaptador não emite `exited`; não é inferido como 0. Não há modificação de variáveis, breakpoints condicionais/logpoints ou attach por seletor de PID na UI deste contrato. Source maps dependem do build/configuração do projeto. Windows/macOS, desempenho com programas grandes e integração Laravel especializada permanecem fora desta certificação.
