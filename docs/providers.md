# Providers locais — M2

**Atualização de armazenamento,24/10/2026:** o instalador agora extrai o PHAR
fixado uma vez e executa `.tools/phpactor-2026.10.07.0/bin/phpactor`. Isso evita
o acúmulo de cópias descompactadas observado após encerramentos forçados.
O download, vendor e licenças são preservados; sem Composer ou novo servidor.
O caminho histórico `.tools/phpactor.phar` vira um launcher PHP próprio de235
bytes para que perfis antigos também usem o runtime extraído, sem regravar
sessões ou autorizações. Arquivos estrangeiros e symlinks são recusados.
Diagnóstico, limites e testes em [provider-storage.md](provider-storage.md).
As referências ao PHAR como entrypoint abaixo descrevem a instalação histórica.

Instalação e verificação realizadas em 20/10/2026. Tudo fica em `.tools/`, sem instalação global e sem alteração do QueryGrip. A instalação não executa npm, Composer ou scripts do workspace. Os binários não são iniciados ao abrir uma pasta: a aplicação exige ação explícita de iniciar providers.

## Versões e origens

| Componente | Versão usada | Origem e licença |
|---|---|---|
| PHP CLI | 8.4.26, `/usr/bin/php` | Runtime existente no Debian; observado por `php --version`, não instalado por esta tarefa |
| Phpactor | 2026.10.07.0 | [Release upstream](https://github.com/phpactor/phpactor/releases/tag/2026.10.07.0), PHAR; [licença MIT fixada](https://raw.githubusercontent.com/phpactor/phpactor/2026.10.07.0/LICENSE) |
| Node.js | 24.22.0 Linux x64 | [Distribuição oficial](https://nodejs.org/dist/v24.22.0/), checksum SHASUMS256; LICENSE distribuído no arquivo inclui Node/MIT e componentes terceiros |
| typescript-language-server | 6.0.2 | [Pacote oficial](https://registry.npmjs.org/typescript-language-server/6.0.2), Apache-2.0; LICENSE também inclui partes Microsoft sob MIT |
| TypeScript | 6.0.3 | [Pacote Microsoft](https://registry.npmjs.org/typescript/6.0.3), Apache-2.0, LICENSE.txt preservado |

A versão 7.0.2 do pacote TypeScript era latest no registro, mas a documentação do wrapper recomenda TypeScript 6; foi selecionado 6.0.3 explicitamente. Ambos os pacotes npm acima declaram zero dependências npm externas na distribuição consultada. O PHAR inclui fornecedores próprios: sua licença raiz não transforma todos esses componentes em MIT. A instalação preserva o artefato completo; empacotamento/distribuição futuros exigem inventário e notices dos fornecedores do PHAR/Node. Nenhum ativo JetBrains foi usado.

Hashes SHA-256 dos arquivos baixados:

- Node: `cc2f601cee82d9a5a233302fe307abc600727035fd7d2c3a3571755989e95008`
- Phpactor: `b28bf6166e016d539fc2f2aec18ef918b12590e436a6f1ccbdf30b5bbdbdb16b`
- typescript-language-server: `d225b223a8a4764ecbd94bdba85d2ef185eaa7b9842a1a31c38c2bbe7f2fe9b9`
- TypeScript: `33cd0ee1beaa8c9e9d15a9da836c62ddea4c34a42d7c2d349dbc80d94165d22a`

Node foi validado contra o [SHASUMS256 oficial](https://nodejs.org/dist/v24.22.0/SHASUMS256.txt). O hash PHAR foi conferido contra o digest da API de release GitHub. Os arquivos npm são verificados pelo SHA-512 integrity do registro, fixado em `tools/providers/npm-sources.json`; o manifesto local também registra SHA-256. Isso é validação de integridade por HTTPS, não uma afirmação de verificação de assinatura independente.

## Instalar e testar

```sh
python3 tools/install_providers.py
CODISSEU_PROVIDER_CONFIG="$PWD/.tools/providers.json" cargo test -p codisseu-language --test providers --locked -- --ignored --nocapture
CODISSEU_PROVIDER_CONFIG="$PWD/.tools/providers.json" cargo test -p codisseu-core --test fullstack --locked -- --ignored --nocapture
CODISSEU_PROVIDER_CONFIG="$PWD/.tools/providers.json" cargo test -p codisseu-ui --test lsp_workflow --locked -- --ignored --nocapture
```

O instalador foi implementado e executado em Linux x64. Não é um gerenciador multiplataforma. Gera `.tools/providers.json` com caminhos absolutos da instalação atual e `.tools/manifest.json`; depois de mover o projeto, execute novamente para regenerar os caminhos. Os arquivos baixados permanecem em `.tools/downloads`, e `.tools/install.log` guarda o resultado desta sessão. Configurações não contêm credenciais. O PHAR é invocado por PHP com `xdebug.mode=off`; o Node local inicia o wrapper com `--stdio`, e o cliente fornece a localização fixada de tsserver.

O teste real passou com erro de sintaxe PHP e erro TS de atribuição string→number. A versão PHP foi enviada no diagnóstico; a versão TS veio ausente e é identificada por `unversioned_paths`, inclusive quando a lista publicada está vazia. `analyzed_versions` associa a versão do cliente e não confirma o texto analisado pelo servidor nesses eventos. A posição UTF-16 após emoji apontou ao identificador esperado, e os diagnósticos foram limpos após didChange com correções ainda não salvas. A parada aguardou/recolheu os processos. Logs locais: `.tools/provider-tests.log`, `.tools/language-unit.log`, `.tools/language-clippy.log`. Consultar `docs/status.md` para a validação integrada final da GUI.

As três suítes explícitas acima foram executadas, cada uma com um teste real aprovado. FULL01 exercita PHP/TS e consulta SQLite no mesmo AppService, salvamento e restauração sem iniciar processos/conexões automaticamente. O teste de widgets configura e inicia providers, clica no diagnóstico TS, verifica foco e posição UTF-16 por inserção de texto após emoji, corrige o buffer sem salvar, observa limpeza e encerra providers. Ele identificou uma falta de glyph U+1F327 no fallback inicial; após incluir Noto Sans Symbols 2 no produto passou em 4,47 s, com verificação de acessibilidade ativa. Logs: `.tools/fullstack-tests.log`, `.tools/lsp-gui-tests.log`, `.tools/lsp-gui-clippy.log`. A suíte comum os marca como ignorados; só a execução explícita fornece essa evidência. O Harness gera eventos egui e não representa cliques do compositor/sistema operacional.

O contrato e as limitações (especialmente diagnósticos push sem versão, ausência de completion e plataformas ainda não testadas) estão em [contracts/language.md](contracts/language.md). Fontes técnicas: [LSP 3.17](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/), [Phpactor LSP](https://phpactor.readthedocs.io/en/master/usage/language-server.html), [wrapper TypeScript](https://github.com/typescript-language-server/typescript-language-server).

## M3.3 — atualização do índice Phpactor

O perfil gerenciado gerado pelo instalador inclui antes de `language-server`:
`--config-extra={"language_server_worse_reflection.workspace_index.update_interval":0}`.
Na versão fixada, WorkspaceIndex mantém um documento pendente global; o debounce
podia substituir uma edição ainda não aplicada ao receber vários buffers.
A fonte MIT do PHAR foi inspecionada; evidência e testes em
`artifacts/m3-definition-20261020/phpactor-index-observations.json`.
A flag aplica notificações imediatamente no processo externo e pode aumentar seu
custo; sem benchmark. Não é injetada pelo cliente em configs arbitrárias.
Definição real PHP/TS foi verificada com fonte e destino não salvos, sem usar
diagnósticos como confirmação de indexação. Detalhes em `m3-definition-contract.md`.

## M3.4 — cache de reflexão Phpactor

O config-extra gerenciado mantém index update_interval0 e agora define
`"worse_reflection.enable_cache":false`. A fonte MIT empacotada foi inspecionada:
cache é habilitado por padrão, lifetime1s, e invalidação por documento não garante
atualização de todas as reflexões usadas no completion após renomear um membro.
O handler de definição purga cache global; o de completion não segue esse fluxo.
Dois testes reais consecutivos passaram com a opção desligada, sem esperar
diagnósticos nem dormir antes do pedido. Manifestos/fontes/hashes em
`artifacts/m3-completion-20261020/phpactor-completion-cache-observations.json`.
A configuração vem do instalador e só do perfil gerenciado; não há injeção em
providers arbitrários. Custo de CPU/indexação em projetos grandes não medido.
Providers retornam nomes plainText sem() ou tipo em várias sugestões: esses
metadados não são inventados pela GUI.

## M3.5 — readiness e referências

Phpactor só passa a Ready após o término real do progresso inicial Indexing
workspace; o cliente negocia workDoneProgress, responde create e acompanha o
token begin/end. Desde M3.32, progresso válido do mesmo token mantém a
indexação ativa;120s sem progresso ou30min totais causam falha explícita.
Não encerra a indexação saudável simplesmente porque passou de30s.
Isso corrige uma corrida em DirtyFileListProvider que os testes PHP-primeiro
reproduziram; não altera o perfil nem os arquivos do índice manualmente.

TypeScript6.0.3 só informa isDefinition em certas consultas originadas na
declaração. TLS6.0.2 podia incluir a declaração ao buscar usos apesar do parâmetro
false. Para o artefato identificado pelo manifesto local, Storm consulta uma
definição semântica privada e retira somente intervalos/caminhos exatamente
coincidentes. As duas fases têm o mesmo deadline10s, snapshots e cancelamento.
Providers genéricos mantêm o fluxo comum; não há filtro textual simulando análise.

Fontes MIT/Apache2 e hashes locais: artifacts/m3-references-20261020/
phpactor-index-initialization.json, phpactor-references-sources.json e
typescript-usage-declaration-observations.json. Licenças preservadas no mesmo
diretório. Nenhum algoritmo ou ativo do IDE proprietário foi incorporado.
Custos em projetos grandes e completude semântica não foram medidos.

## Portabilidade de instalação —29/10/2026

Pacote gerenciado usa codisseu-runtime/runtime.json junto à instalação, com
PHP/Node/Phpactor/TS privados e paths relativos, além de DAP/API Node compartilhado.
Default vazio descobre o manifesto; vetor gerenciado legado exato é rebaseado,
com aprovações antigas preservadas sem conceder autoridade ao novo executável.
Configuração explícita é mantida. Worker resolve arquivos; GUI não faz instalação
ou hashes por frame. Guia install-linux.md, contrato portable-runtime-contract.md
e relatório portable-runtime-report.md distinguem a instalação de desenvolvimento
com .tools da instalação portátil validada em Linux x86_64 sem rede/PHP/Node host.

## Python —02/11/2026

Basedpyright1.40.2 é o quarto provider gerenciado, executado pelo Node privado.
O instalador explícito `python3 tools/install_python_tools.py` prepara os motores
pinados em `.tools/python`; hashes/fontes ficam em `tools/python-tools.lock.json`.
No portátil eles já acompanham a instalação. A migração acrescenta Python somente
à configuração reconhecida como gerenciada; vetores manuais são preservados e
não se transfere aprovação para executáveis novos. Iniciar continua dependendo
da autorização exata do projeto/configuração.

A escolha de interpretador/venv por projeto informa `python.pythonPath` e renova
as consultas Python. Não altera slots PHP/TS/Rust. Basedpyright recebe buffers
abertos, autoimports e hints; Ruff cuida de formato/correções. Abrir um arquivo
não instala pacotes nem executa a aplicação. Veja [guia](python-usage.md),
[contrato de análise](python-language-contract.md) e [runtime](python-runtime-contract.md).
