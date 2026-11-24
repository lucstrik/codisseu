# Console SQL nativo — 24/11/2026

Cenário relatado pelo usuário: `describe console_transport.transports;`
era recusado antes de chegar ao MySQL pelo guard SELECT/WITH e wrapper de
paginação. Pedido confirmado: console completo, respeitando Read-only e sua
alteração ao salvar. Implementação própria; nenhuma nova observação DataGrip.

O console envia uma instrução por execução ao servidor, incluindo DESCRIBE/DESC,
SHOW, EXPLAIN, DML, DDL e controles de sessão/transação do dialeto conectado.
Não traduz MySQL para PostgreSQL nem transforma comandos psql em SQL. Scanner
respeita strings/comentários; scripts, DELIMITER e comentários executáveis ou
aspas ambíguas por configuração são recusados antes de enviar. Sintaxe SQL
nativa não precisa ser reconhecida pelo parser de paginação.

SELECT compatível conserva páginas e ordenação; outros comandos usam protocolo
nativo sem wrapper, com limites existentes de retenção e prazo. Nenhuma origem
editável em metadados, planos ou RETURNING. Linhas afetadas são reconhecidas do
protocolo; resultados de escrita são drenados mesmo se excederem a grade.

Read-only permite consultas e inspeção, bloqueia DML/DDL/SET/CALL/controles de
escrita, conserva transação READ ONLY no servidor e guards dos helpers. EXPLAIN
somente de SELECT sem ANALYZE nesse modo. Desativar e salvar aplica a permissão
à mesma sessão SQL sem reconectar, se o alvo de conexão permanecer igual.
Mudança com transação pendente exige primeiro Commit/Rollback, sem confirmação
implícita. Alterações de alvo/TLS continuam exigindo reconexão.

Em fonte gravável, DML abre transação manual para Commit/Rollback; leituras
isoladas seguem autocommit do servidor. DDL/comandos de sessão seguem a semântica
nativa: DDL MySQL pode confirmar transação implicitamente. Estado do servidor é
reconciliado depois de comandos gerais; SQL BEGIN/COMMIT/ROLLBACK e SET autocommit
não podem deixar flag fictícia na UI. Erros DML recuperáveis conservam alterações
anteriores por savepoint. Cancelamento/timeout não reexecuta escrita; resultado
incerto é comunicado pelo caminho existente.

Validação prevista: PostgreSQL/MySQL/MariaDB isolados em Docker e loopback,
comando relatado em schema sintético, corpus SHOW/EXPLAIN, DML/RETURNING,
Commit/Rollback, DDL/commit implícito, troca Read-only via worker e execução de
SQL pelo controle real da interface. Recolher somente fixtures próprias.

SQLite também aceita SQL nativo ao habilitar edição, com parser do próprio
SQLite para instrução única, DML manual/RETURNING e estado is_autocommit real.
Sua conexão inicial de leitura continua protegida por flags do arquivo,
query_only e authorizer. ATTACH e extensões permanecem fora do escopo local.

Limites: rotinas MySQL com delimitadores internos/DELIMITER e scripts não são
suportados pelo scanner; PostgreSQL dollar-quoted é aceito. Protocolo raw pode
retornar resultado vazio sem nomes de colunas. Diagnósticos SHOW WARNINGS/ERRORS
são enviados sem prepare/controles prévios, mas controles de transação/probes de
operações anteriores podem substituir avisos do servidor. Estados XA/prepared,
COPY streaming e resultados múltiplos de rotinas não têm certificação própria.
Nenhuma alegação de todos os comandos, servidores ou paridade integral DataGrip.

## Inatividade e contexto SQL

Uma sessão que permitiu escrita pode executar USE, SET search_path, SET ROLE e
criar objetos temporários. Esse estado não pode ser reconstruído apenas com o
cadastro. Após a inatividade configurada, ela é fechada e requer Connect explícito
antes de executar outra query; ExecuteConsole não reconecta silenciosamente.
Marcar Read-only novamente não apaga essa condição da conexão já utilizada.
Sessões iniciadas e mantidas em Read-only conservam retomada das leituras.
Commit/Rollback não são repetidos em outra conexão. Nova conexão explícita começa
com o contexto do cadastro; SQL de inicialização próprio deve ser executado de novo.
