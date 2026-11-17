# Jira — tarefas no fluxo de desenvolvimento

Pedido de 03/11/2026. Implementação própria Rust/egui, opt-in por perfil.
Não é alegação de exclusividade no mercado nem reprodução de interface alheia.

## Cenário e apresentação

Configurações → Integrações → Jira habilita a entrada na barra direita. O painel
começa em Minhas tarefas, sem concluídas; selecionar abre um leitor central,
preservando documentos, grupos e buffers. Clique na aba do arquivo para voltar. Cabeçalho,
busca, filtros progressivos, estados vazios acionáveis e paginação permanecem
disponíveis. Descrição e comentários aceitam Markdown e ADF; HTML é texto,
sem execução ou acesso a recursos locais. Hierarquia e vínculos
podem ser abertos mesmo quando não pertencem ao usuário atual.

Filtros por texto/chave, responsável, projeto, tipo, pai/épico, equipe e categoria
de status são combinados no servidor. Alterações persistem automaticamente após
debounce; redefinir restaura Minhas tarefas abertas. Vistas nomeadas são opcionais.
Equipe é campo customizado descoberto no tenant; ausência/permissão insuficiente
é comunicada. Seletores pesquisam nomes sem carregar o diretório inteiro.

## API, dados e limites

Jira Cloud REST v3, Basic com email/token, HTTPS validado, redirects recusados.
Endpoint de gateway com cloudId pode ser configurado para tokens com escopos.
Busca enhanced `/search/jql`, 50 itens por página, campos mínimos; somente a
página atual em memória, histórico de até64 cursores. Não baixar anos de histórico,
calcular contagens globais ou executar polling remoto contínuo. Descrição,
comentários e worklogs apenas ao abrir; comentários/worklogs em páginas de20.
Requisições limitadas por timeout e resposta máxima; uma operação por worker
principal e até4leituras de worklog simultâneas no worker de horas, fila limitada. Respostas obsoletas não substituem filtro/card/conexão
atual. Desativar invalida operações e descarta conteúdo em memória.

Preferências JSON versionadas e atômicas, separadas do workspace; nenhum token
ou conteúdo de cards em JSON/logs/snapshots. Token em memória protegida contra
Debug e opção explícita de cofre do SO em namespace próprio. Falha do cofre não
gera fallback em arquivo. Nenhuma varredura/importação automática de `.secrets`.

Desde o pedido de04/11, perfis habilitados/configurados tentam uma reconexão pelo
cofre ao restaurar, fora da GUI e sem repetir automaticamente após ausência/erro.
Perfis antigos com chave salva são compatíveis. Desconectar persiste a escolha
de não reconectar; conectar pelo cofre ou guardar uma chave válida habilita essa
preferência. Token novo de sessão não é reutilizado ao reiniciar. Esquecer remove
o segredo. Contrato específico: jira-reconnect-contract.md.

Apontamento exige card, duração positiva, início com fuso e revisão explícita.
POST de worklog não tem retry automático; falha de transporte pode significar
gravação concluída e exige conferir no Jira antes de reenviar. Mantém estimativa
restante (`adjustEstimate=leave`). Permissões respeitam o servidor.
Nenhum apontamento/comentário é publicado em cards empresariais durante testes.

## Fontes e validação

Confiança `documented`, consultadas em03/11/2026: [busca enhanced](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/),
[worklogs](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-worklogs/),
[ADF](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/),
[tokens e gateway](https://support.atlassian.com/atlassian-account/docs/manage-api-tokens-for-your-atlassian-account/),
[campo Team](https://developer.atlassian.com/platform/teams/components/team-field-in-jira-rest-api/).
Referências UX `documented`: [estados vazios](https://atlassian.design/components/empty-state),
[skeleton](https://atlassian.design/components/skeleton/usage) e
[painéis](https://atlassian.design/components/panel/examples). São orientações;
não medidas locais de cores/geometria. Tokens e ícones licenciados já existentes
do Codisseu; nenhum ativo Atlassian incorporado. Orbit confirmou busca enhanced;
Postman instalado não oferece `status`/simulation, usando testes HTTP locais.

Aceitação: JQL e escaping, compatibilidade de perfil, persistência, cursor e
limites, ADF/Markdown hostis, HTTP401/403/429/5xx, timeout/redirect, stale replies,
worklog sem duplicação; widgets em100/125%, ativar/desativar/reset e leitor.
Leitura real autorizada do tenant Empresa Exemplo separada das fixtures sintéticas.
Evidências, comandos efetivos e limites finais em docs/status.md.

## Avatares, imagens e alinhamento — complemento do pedido

Em03/11 o usuário pediu imagens fornecidas pela API, incluindo foto redonda
pequena. Avatares de responsável/criador/comentários/horas e anexos PNG/JPEG/GIF/
WebP aparecem no leitor; fallback com iniciais quando ausentes ou indisponíveis.
ADF media sem URL resolvível conserva marcador, e anexos acessíveis do card são
apresentados na seção Anexos. Não promete resolver IDs internos de mídia de
Confluence, SVG, vídeos ou arquivos sem permissão. Clique em links é explícito.

Worker exclusivo para imagens, fila8, timeout6s/3s de conexão, até4 redirecionamentos
validados,4MiB codificados/4096×4096/4milhões de pixels por arquivo, redução somente
quando necessário para1600×1200. Decodificação fora da GUI. Cache de32entradas e
24MiB de pixels no gerenciador; leitor até24MiB adicionais conservados por Arcs,
com texturas correspondentes. Sem cache de imagens em disco. Requisições novas
não bloqueiam textos; respostas de conexão antiga são descartadas.

Autorização HTTP somente para origens exatas configuradas; no gateway, somente o
prefixo do cloudId. CDNs HTTPS Atlassian/Gravatar e i0…i3.wp.com recebem requisições
sem token. Origens locais/arquivos/outros domínios são recusados. Endpoint oficial
attachment/content ou thumbnail com redirect=false evita encaminhar credenciais
para CDN. Accept=*/* é necessário no cenário real observado; os bytes passam por
decoder de imagens limitado, nunca por HTML. Gravatar→i2.wp.com foi observado
localmente, sem supor que todo avatar usará esse destino.

Captura Foto1 fornecida pelo usuário: desalinhamento do avatar/texto e busca/filtros
`observed` no Codisseu; build/escala da imagem não declarados. Cada linha agora
mede o título efetivamente renderizado e centraliza o avatar nessa caixa; chave
fica acima, mantendo texto à esquerda. Busca tem rótulo/borda/recuo explícitos;
status e ações compartilham eixo central. Teste compara bounds avatar/título em
100/125%; screenshots revisados separadamente do corpus anterior.


Avatares sem URL, com erro, transparentes ou de cor uniforme usam iniciais do
primeiro/último nome; nome ausente mostra ?. Uma amostragem limitada identifica
somente imagem uniforme, sem prometer reconhecer todos os avatares genéricos de
terceiros. A foto/arte fornecida pela API com detalhes é preservada. Fixtures de
GUI não usam fotos pessoais; o teste de iniciais inclui Lucas Torres Sao Felicio
→ LF, palavra única e nome ausente. O pedido posterior do usuário substitui a
fixture anterior de círculos azuis por iniciais legíveis.

Leitura em andamento pode terminar no worker depois de desativar a integração;
o resultado obsoleto é descartado. Timeout não certifica cancelamento no servidor.
Profile usa comparação com bytes anteriores e trava de gravação para evitar que
outra janela seja sobrescrita. Trava residual após crash requer verificação local;
não há apagamento silencioso de arquivos de configuração.


Em05/11 os filtros avançados passam a8pontos entre linhas/controles28, rótulos
à esquerda e altura limitada à viewport. O atalho Criados para mim neste mês usa
o mês local corrente, independente de um período histórico de Minhas horas.
Pool HTTP separado e entrada direta pelo calendário: contrato
welcome-jira-references-contract.md.
