# Banco no Codisseu — MVP em 23/10/2026

Atualização30/10: o funil **Enable Local Filter** da toolbar habilita um funil
em cada cabeçalho. Abra-o para buscar valores e marcar quais linhas da página
mostrar. As escolhas combinam colunas; **Clear Values** desmarca todos,
**Select All Values** remove a restrição daquela coluna. Desligar o funil
conserva escolhas para religar. **Clear Local Filters**, no header ou nas opções,
limpa condições; **Reset View** também restaura a apresentação. Coluna oculta
continua filtrando. Isso atua só na página carregada e não envia SQL.

**Select Column**, no menu do cabeçalho, seleciona a coluna para copiar somente
suas linhas visíveis. A busca da lista Ctrl+F12 agora recebe foco ao abrir;
Escape ou clique fora fecha o popup. Nomes longos não invadem os ícones do header.

Atualização29/10: botão direito no cabeçalho permite **Hide Column**, **Hide
Other Columns**, **Show All Columns** e **Show Column List**. Com foco numa
célula, **Ctrl+F12** abre a lista; digite para buscar, use setas e **Space** para
ocultar/mostrar. Colunas ocultas aparecem riscadas. Arraste a divisória direita
do cabeçalho para redimensionar; duplo clique restaura a largura natural.

**Ctrl+F** numa célula ou a lupa da grade abre **Find on Current Page**. **Cc**
diferencia maiúsculas; **Filter rows** restringe as linhas exibidas. Setas
anterior/próximo navegam correspondências; **Escape** no campo fecha a busca.
Tudo atua apenas na página carregada, sem executar outra consulta. A cópia
inclui somente células/linhas/colunas visíveis; **Export Data** conserva a página
completa do resultado. **Reset View** restaura filtro, larguras e visibilidade.
Disponível na grade SQLite e nas grades de resultados remotos. No editor,
Ctrl+F continua sendo a busca do documento. Contrato: m4-grid-inspection-contract.md.

Abra Database à direita e use o ícone **Adicionar datasource** ou **Data Sources…**. Escolha o motor,
preencha host/porta/banco/usuário/senha e, se necessário, CA PEM. TLS verifica
certificado/hostname. Memcached usa texto sem TLS/SASL; Mongo permite Auth database.
**Apply** salva só metadados; **Test Connection** conecta e fecha; **Connect** abre
a única sessão remota ativa e salva o cadastro. Com **Lembrar senha no cofre do sistema**, a senha é guardada no cofre do SO.
Campo vazio reutiliza a senha salva; desmarcar e aplicar remove a entrada.
Sem essa escolha, a senha fica apenas na sessão.
Reabrir restaura cadastros desconectados. Salvar **Read-only** aplica essa
permissão à mesma conexão SQL aberta; não é necessário reconectar para essa
opção. Havendo transação pendente, faça Commit/Rollback antes de alterá-la.
Mudanças de servidor, autenticação e TLS continuam exigindo reconexão.

**Console** cria arquivo no editor comum, vinculado ao datasource. **Ctrl+Enter**
executa a seleção explícita ou a instrução sob o cursor. Use uma instrução por
execução, separada das demais com `;`. O dialeto é o do servidor conectado.
`DESCRIBE banco.tabela`, `DESC`, `SHOW` e `EXPLAIN` funcionam em MySQL/MariaDB;
PostgreSQL usa seus comandos nativos, como `SHOW search_path` e `EXPLAIN SELECT`.
Comandos psql (`\dt`) não são SQL e não são interpretados pelo aplicativo.

Com **Read-only** ativo, o servidor e o console protegem contra escritas.
Desative e salve para executar também `INSERT`, `UPDATE`, `DELETE`,
`CREATE`, `ALTER`, `DROP`, `SET`, `USE` e demais instruções nativas do servidor.
DML abre transação manual: **Commit** confirma e **Rollback** reverte. Também
é possível enviar `BEGIN`, `COMMIT` e `ROLLBACK` pelo editor. DDL segue a
semântica nativa: MySQL/MariaDB podem confirmar implicitamente a transação.
SQLite permite SQL nativo depois de **Habilitar edição**; inicia em leitura.
Quando uma sessão que permitiu escrita expira por inatividade, use **Connect**
explicitamente e restabeleça seu contexto (`USE`, `search_path`, variáveis etc.).
O console não recria esse estado automaticamente em outro socket.
A saída informa linhas afetadas; metadados/planos/RETURNING não habilitam edição
pela grade nem recebem wrapper de paginação. SELECT compatível conserva páginas.
Não há execução automática de scripts, DELIMITER ou comandos shell do cliente.

Para editar células: use SELECT simples com PK completa ou carregue a tabela.
Duplo clique/F2 abre o editor tipado, Enter aceita a proposta local e Escape
cancela. **Commit** envia e confirma; **Submit** envia mantendo a transação aberta.
**Rollback** desfaz o enviado sem confirmação. Identidade ambígua, colunas geradas,
PK e tipos sem tratamento seguro permanecem protegidos na edição pela grade.
**Revert** descarta apenas alterações locais. Fechar com alterações exige decisão.

O padrão é leitura, 500 linhas, 10s por consulta e desconexão após 60s ociosos.
SQL mantém uma conexão, sem pool; Mongo pode ter sockets extras de monitoramento
mesmo com pool de aplicação 1. Transação ociosa é revertida ao desconectar. Stop
solicita cancelamento/fecha sessão; não afirma interrupção remota confirmada.
Limite de linhas não mede custo do plano no servidor.

Exemplos de consoles NoSQL (somente leitura neste incremento):

```json
{"collection":"users","filter":{"active":true},"limit":100}
```

```text
GET minha:chave
SCAN 0 MATCH prefixo:* COUNT 100
HSCAN meu:hash 0 COUNT 100
```

```text
get minha:chave
stats
```

O primeiro exemplo é MongoDB, o segundo Redis, o terceiro Memcached. Redis SCAN
é explícito, COUNT é sugestão do servidor; KEYS/HGETALL/scripts/escritas ficam
bloqueados. Memcached não varre chaves. Sem Redis Cluster, Mongo SRV/replica-set
com fanout, SSH, exportação CSV ou escrita NoSQL nesta entrega. Aurora reutiliza
protocolos MySQL/PostgreSQL. Endpoints AWS e servidores TLS não tinham sido
validados no incremento inicial; os testes posteriores de conexão estão abaixo.

## Cadastros locais diagnosticados em 23/10/2026

EXAMPLE_DB local usa PostgreSQL em 127.0.0.1:5432, banco psigest, usuário sail,
TLS desativado somente nesse servidor local sem TLS. O hostname pgsql do .env
é interno ao Docker, não o endereço para um aplicativo no desktop.

ExampleGroup DEV usa MySQL remoto com TLS e CA regional oficial do RDS já
configurada no cadastro pessoal. Database ficou vazio porque o arquivo de
credenciais não indica schema. Ambos permanecem Read-only; selecione o cadastro
e mantenha **Lembrar senha no cofre do sistema** para reconectar com o campo
vazio. Em23/10, após pedido do usuário, as senhas desses dois cadastros foram
importadas dos arquivos autorizados para o cofre e conferidas, sem nova conexão
aos bancos. Não há senha no JSON do cadastro.
Esses dois cadastros passaram Test Connection e SELECT 1 na interface real;
isso não certifica todas as consultas ou o desempenho nas bases completas.

## Vários bancos e sugestões SQL

MySQL permite deixar **Database** vazio e usar nomes qualificados:
`SELECT * FROM corrier.remetentes LIMIT 10;` ou
`SELECT * FROM corrier_fat.fatcte LIMIT 10;`. Na árvore do datasource, expanda
o banco para carregar tabelas e a tabela para carregar colunas. Duplo clique
na tabela abre a prévia paginada. Atualizar objetos invalida o catálogo.
PostgreSQL conecta a um database e lista os schemas desse database.

No editor SQL conectado, **Ctrl+Space** ou a digitação de prefixo/ponto abre
sugestões reais. Exemplo: depois de `corrier.` aparecem tabelas; em
`SELECT r.no FROM corrier.remetentes r`, o prefixo `r.no` oferece `nome`.
**Enter/Tab** insere identificador escapado, **↑/↓** navega, **Esc** fecha.
**Ctrl+Z/Ctrl+Shift+Z** desfaz/refaz a inserção. Não há consulta de linhas para
completar nomes. Aliases de FROM/JOIN simples são suportados; CTEs/subqueries
complexas e search_path PostgreSQL personalizado ainda exigem qualificação.

As ações do resultado são ícones com tooltip/nome acessível: Execute, Stop,
Submit, SQL Preview, Revert, Commit e Rollback. Preview mostra a alteração
antes de enviar; Submit e Commit continuam ações separadas. Erros SQL
recuperáveis mantêm a sessão; cancelamento/timeout/rede fecham e revertem.

## Quando há mais de256 tabelas

Expandir um banco/schema carrega até256 tabelas por página, sem rejeitar o
banco inteiro. Os ícones acima da lista permitem primeira, anterior e próxima
página; o contador informa apenas quantas estão na página atual. Não há COUNT
nem leitura das tabelas para listar seus nomes.

Use **Filtro** para digitar o começo do nome e confirme com **Enter** ou a
lupa. Apagar o filtro e confirmar volta à primeira página completa. O filtro
interpreta `%` e `_` literalmente. O autocomplete pesquisa o prefixo no servidor
e encontra tabelas além da página exibida; se houver mais de256 sugestões,
digite mais letras. A navegação não altera o resultado SQL que está aberto.

## Aparência Islands Dark

Cadastro: General contém identificação, endereço, usuário, senha e Database;
SSL contém TLS e certificado CA; Options mantém somente leitura, timeout,
inatividade e tamanho da página. Save:Forever lembra a senha no cofre do sistema;
Never mantém apenas o comportamento de sessão. Não há senha em JSON.
No preview, copiar preserva SQL/parâmetros completos; Submit envia alterações,
Commit confirma separadamente. Refinamento e limites em m4-db-visual.md.

### Services

O painel inferior de resultados agora se chama **Services**. A árvore Database
lista as fontes conectadas e suas consoles; clicar na console ativa o documento
sem executar consulta ou criar conexão. A duração exibida pertence ao resultado
concluído. **Output** mostra o estado atual e detalhes da execução; **Result 1**
mostra a grade. Fontes apenas salvas ficam no Database lateral e não aparecem
como sessões ativas fictícias. Resultado carregado de tabela tem folha própria.
A grade conserva cabeçalhos durante o scroll e larguras naturais das colunas.
# Melhorias de26/10/2026

- Clique direito na fonte: **Conectar** e **Propriedades…** em linhas separadas.
  Duplo clique direito conecta usando a credencial já configurada. Duplo
  esquerdo também conecta. Ocultar Database preserva catálogo/expansões da sessão.
- No resultado, **Export Data… → Exportar consulta completa (CSV)…** reexecuta a
  consulta capturada e salva todas as linhas retornadas, com progresso e
  cancelamento. Um LIMIT escrito no SQL continua valendo. O arquivo deve ter
  nome novo. Confirme/reverta transação pendente antes. Exportação da página e
  demais formatos existentes continuam disponíveis no diálogo inicial.
- **Ctrl+Alt+E** ou ícone de histórico abre consultas concluídas da fonte nesta
  sessão. Copiar/inserir não executa; inserção pode ser desfeita com Undo.
- Ícones **Selecionar instrução SQL** e **Explain Plan** atuam sobre o editor
  ativo. Plano estimado não acrescenta ANALYZE. Tabelas/colunas e planos vêm do
  banco real; não há análise estatística simulada.



## Queries salvas —27/10/2026

A pasta **Queries**, no Database, lista os arquivos criados pela IDE mesmo após
fechar projeto ou reiniciar, independentemente da conexão. Duplo clique abre;
abrir não executa a query. Arquivos antigos console-<timestamp> continuam na lista.
**Nova query** no menu da conexão funciona desconectado e usa query-1.sql,
query-2.sql etc. As queries ficam em consoles junto do perfil da IDE; Ctrl+S
salva o conteúdo nesse arquivo.

Clique direito na query da lista ou da aba: **Renomear query…** ou **Excluir
query…**. F2 também renomeia na lista. Informe o nome sem precisar digitar .sql;
o vínculo, histórico de edição e alterações locais acompanham a renomeação.
Excluir exige confirmação, remove permanentemente o arquivo e fecha suas abas;
salve ou descarte alterações pendentes antes. Nenhum dado do banco é removido.

No menu da conexão, **Excluir…** remove o cadastro e sua credencial, preservando
as queries salvas. Queries cuja conexão foi removida continuam editáveis, mas
não executam em outro datasource por engano. Conexão/carregamento usam ícones;
**Output** mostra os detalhes quando solicitado, e erros continuam visíveis.


## Separar DEV, QA, PROD e motores —27/10/2026

**Queries** agora contém uma pasta para cada conexão. DEV, QA e PROD podem ter
cada uma sua própria query-1.sql, em arquivos separados. Redis usa query-1.txt
e MongoDB query-1.json. A numeração é independente por conexão.

A árvore mostra `query-1.sql [ExampleGroup DEV]` e a aba SQL mostra
`query-1 [ExampleGroup DEV]`, com o ícone do motor. Renomear a conexão atualiza
a identificação sem mudar o arquivo. Renomear/excluir a query atua somente na
pasta correspondente; o diálogo também informa a origem.

Queries existentes continuam no caminho salvo e passam a aparecer no grupo de
sua conexão. Abrir uma query não conecta nem troca de ambiente automaticamente;
para executar, conecte a fonte indicada na aba. Uma conexão ativa diferente é
recusada. Excluir o cadastro mantém as queries num grupo de conexão removida.

Ao abrir/trocar uma query, a faixa de abas rola até a aba ativa quando necessário,
para que a identificação da conexão permaneça visível.


## AWS RDS sem configuração manual de CA —27/10/2026

Com TLS ligado e **CA certificate** vazio, conexões SQL RDS/Aurora reconhecidas
usam automaticamente as raízes oficiais da AWS incluídas na IDE. A aba SSL
mostra **Automático · AWS RDS**. Basta cadastrar host, porta, usuário e senha;
não é necessário copiar o certificado de outra conexão ou baixar um PEM.

O certificado e o hostname continuam verificados. Um caminho PEM informado
manualmente tem precedência e não é substituído silenciosamente. Hosts comuns
continuam usando as raízes públicas padrão. Certificados privados/corporativos
podem continuar exigindo um PEM explícito. Atualizar as raízes incorporadas faz
parte da atualização auditada do Codisseu.

QA/PROD ExampleGroup foram configurados com TLS automático, somente leitura e
senhas no cofre do SO; conexão e SELECT1 validados sem acessar dados da aplicação.
DEV e vínculos das queries foram preservados. Após carregar o binário atualizado,
os cadastros podem conectar usando a senha salva.

## Repouso e resultados de consultas

O padrão de inatividade agora é10min. Após esse período, uma conexão sem
alterações enviadas pendentes entra em espera; executar outra busca reconecta
automaticamente na mesma origem e continua a leitura. O resultado e o catálogo
permanecem visíveis. Disconnect explícito continua desconectando. Transações
com alterações ainda não confirmadas mantêm a política de rollback ao expirar.
As três conexões pessoais que ainda estavam em60s foram ajustadas para600s;
reabra a IDE para carregar o novo código e configuração.

Resultados SELECT/WITH têm setas no cabeçalho: crescente, decrescente e ordem
original. O Codisseu aplica a ordem ao conjunto completo no banco, indicado no
tooltip; a referência observada ordena apenas a página em resultados de query.
A ordenação volta à primeira página e não modifica o texto no editor.

Na barra acima da grade, junto dos ícones, use primeira/anterior/próxima página,
recarregar e a faixa de linhas.
Ctrl+Alt+↑/↓ navega anterior/próxima; `+` indica que há mais linhas. Até500linhas
por página, respeitando limite menor da conexão e orçamento de payload. Não há
COUNT automático, All ou última página por varredura. Alterações locais precisam
ser enviadas ou descartadas antes de navegar. CSV completo continua exportando
a consulta original inteira, sem o LIMIT/OFFSET acrescentado pela paginação.

## Executar uma query salva sem conectar antes

Abra a query da biblioteca e execute normalmente. O Codisseu identifica a fonte
pelo vínculo salvo e conecta usando a senha dessa fonte no cofre do sistema.
O mesmo fluxo troca de outra conexão ativa quando não há transação/alterações
pendentes. Abrir o arquivo ou iniciar a IDE continua sem conectar por conta
própria. Cadastro removido, senha indisponível/incorreta ou erro real de rede
precisam ser corrigidos; a query nunca cai em outra fonte nem no SQLite.


## Conexões e fechamento — 29/10/2026

Uma instância do Codisseu mantém uma sessão SQL para a fonte ativa; abas,
catálogo e paginação reutilizam essa sessão. Fechar a aba de consulta não
significa desconectar a fonte. Use **Desconectar datasource** para liberá-la
imediatamente; a inatividade também suspende a conexão física.

Cancelamento de SQL pode usar uma segunda conexão por poucos instantes para
encerrar a sessão exata no servidor. Se isso falhar, o Codisseu bloqueia nova
conexão até conseguir limpar a anterior. A senha permanece somente em memória
protegida/entrada já escolhida no cofre; nenhum perfil novo foi conectado
automaticamente por esta alteração. Medições locais.

## MongoDB —29/10/2026

General mantém Host/Port/User/Password/Database/Auth database; SSL mantém TLS e
CA PEM. Options agora permite Authentication (automático ou SCRAM), Replica set,
Connection mode (direto ou descoberta) e Read preference (primary/secondary e
variantes). Pool1 por servidor e monitoramento adicional; troca fecha cliente
anterior. Não se conecta ao restaurar. Cadastros Empresa Exemplo QA e PROD foram
preenchidos no perfil normal por solicitação, senhas no cofre. Produção usa admin
por ausência de database na URI; informe a base de trabalho em Database quando
necessário. O teste remoto autorizado validou apenas autenticação/ping, não
permissões ou conteúdo de collections. Detalhes em m4-mongo-options-report.md.

## Execução e desconexão —30/10/2026

Ctrl+Enter, Play do editor (junto das abas) e Play principal compartilham o alvo:
seleção explícita ou bloco sob o cursor. SQL mantém uma instrução por vez. Em
queries Mongo JSON, cada objeto completo é uma consulta, mesmo com várias linhas
e objetos separados por newline ou `;`. Strings/arrays/chaves internas não
separam o bloco; comentários externos podem usar // e /* */. O formato de leitura
Mongo existente permanece. A extensão de leitura find/aggregate está descrita
abaixo; não executa lote implícito nem avalia JavaScript.

O atalho exige foco no editor e contexto de consulta (.sql, query cadastrada ou
.json com Mongo ativo). Campos/terminal/JSON comum fora desse contexto mantêm
seus atalhos. A execução usa o buffer atual, sem salvar nem inserir linhas.

Clique direito no datasource oferece **Desconectar**, também para Mongo/Redis/
Memcached e na raiz SQLite. Ativa apenas na conexão correspondente e sem
operação/staging pendente; não exclui cadastro ou queries. Segue a política
transacional existente do Disconnect.


## Consultas Mongo com sintaxe de shell — 30/10/2026

Na query vinculada ao Mongo, pode executar diretamente:

```javascript
db.getSiblingDB("outra-base")
  .getCollection("pedidos")
  .aggregate([
    {$match: {clienteId: 123}},
    {$group: {_id: "$clienteId", quantidade: {$sum: 1}}},
    {$sort: {_id: 1}}
  ]);
```

Ctrl+Enter ou Play executa a chamada inteira sob o cursor, inclusive dentro de
um estágio do pipeline. getSiblingDB define o banco apenas para essa chamada;
a conexão e autenticação continuam as cadastradas. Também aceita
`db.getCollection('pedidos').find({clienteId:123}, {clienteId:1}).sort({_id:-1}).limit(10)`
e o JSON anterior. Erros aparecem no painel de resultados, inclusive sem grade.

Suporte a literais de leitura, implementado em Rust: objetos/arrays, chaves sem
aspas, strings simples/duplas, números, boolean/null e comentários. Variáveis,
funções, construtores como ObjectId/ISODate, opções extras de aggregate e outros
métodos ainda dão erro explícito. Estágios de escrita $out/$merge e JavaScript
no servidor permanecem recusados. Limite da página é aplicado ao resultado final
do pipeline, depois de agrupamento/ordenação; truncamento é informado.
Contrato e evidências em mongo-shell-read-contract.md e status.md.
