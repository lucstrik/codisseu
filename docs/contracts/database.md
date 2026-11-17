# Contrato SQLite — M2

## Serviço único e conexão explícita

`storm-db::DbService` fornece `dispatch(DbCommand)`, `poll()` e `is_busy()`.
Comandos: `Connect(PathBuf)`, `Disconnect`, `RefreshSchema`, `Execute { sql }` e
`Cancel`. O AppService integra esses comandos à mesma janela e sessão do editor.
A confirmação de `dispatch` é somente enfileiramento; estado e erro final chegam
por `poll`. Instanciar o serviço não abre arquivo nem conecta automaticamente.

Um worker mantém uma `Session` com exatamente uma `rusqlite::Connection` até
desconexão/troca. Abertura, introspecção e consulta ficam fora da thread da GUI.
Uma troca malsucedida preserva a conexão anterior. Troca/desconexão requerem
worker sem pedidos pendentes. Encerramento interrompe a consulta, desconecta os
canais e fecha a conexão no worker, sem join bloqueante na GUI.

## Escopo somente leitura

M2 abre arquivo existente, canonicalizado e regular com `SQLITE_OPEN_READ_ONLY`,
sem `CREATE`. Define `query_only=ON`, `trusted_schema=OFF` e um authorizer que
permite leitura e introspecção delimitada. Escritas, controle de transação,
ATTACH/DETACH, alteração de pragmas e funções de extensão/arquivo são rejeitados.
Nenhuma função SQL da aplicação é registrada; carregamento de extensões não é
habilitado. `Statement::readonly` é uma checagem adicional, não a única proteção.

Executa uma instrução com colunas de resultado. `Connection::prepare` da versão
fixada usa o parser SQLite para rejeitar instruções adicionais antes da execução.
O adaptador continua recebendo uma única instrução e rejeita SQL adicional.
Desde o incremento de23/10, a GUI envia a seleção explícita ou a instrução
sob o cursor, identificada pelo lexer do dialeto (não split textual por `;`).
O comando de domínio ExecuteSql(None) mantém o documento inteiro como contrato
para chamadores diretos; o roteamento da GUI envia SQL explícito.
Edição de grade, commit/rollback, scripts e bancos remotos ficam para M4.

## Tipos, schema e limites

`Value` é reexport de `rusqlite::types::Value`, com NULL, i64, f64, texto e blob.
Conversão pública `FromSql` rejeita texto UTF-8 inválido. `display_value` produz
somente uma prévia limitada (512 caracteres/32 bytes de blob); o valor transportado
preserva todos os bytes aceitos. String vazia e NULL têm labels diferentes.

`TableInfo`/`ColumnInfo` vêm do schema real e dos pragmas SQLite parametrizados.
O campo `nullable` considera `NOT NULL` e alias INTEGER de rowid, sem tratar todo
PK SQLite como não nulo. Views são incluídas; não há grade editável nesta etapa.

Limites de aplicação:

- 8 pedidos/respostas pendentes; fila cheia retorna erro.
- SQL: 256 KiB. Resultado: 500 linhas, 64 KiB por célula, 4 MiB de payload e
  estruturas contadas. Linha que excede limite é omitida inteira, sem cortar valor;
  `truncated` e `limit_reason` explicam a interrupção da prévia.
- Schema: 256 tabelas/views, 256 colunas por tabela, 8192 colunas totais, 1 MiB de
  nomes/metadados. Excesso falha explicitamente, não publica schema parcial.
- Limites SQLite adicionais: LENGTH 1 MiB, COLUMN 256, SQL_LENGTH 256 KiB,
  EXPR_DEPTH 100, COMPOUND_SELECT 50 e ATTACHED 0. Busy timeout: 500 ms.

Esses limites não são um teto de RSS do processo: SQLite também usa cache e
memória de execução. O cache sugerido é 2 MiB; queries complexas podem consumir
recursos internos além do orçamento de resultados. Não há benchmark de RSS.

## Cancelamento e versões

`Cancel` incrementa geração e chama `InterruptHandle::interrupt` real.
O progress handler verifica geração e orçamento de 10 segundos a cada 1000
instruções da VM. O status diferencia pedido de cancelamento, interrupção
confirmada pelo SQLite, tarefa cancelada antes de iniciar e resposta que já
havia terminado e foi descartada. Resultado antigo não substitui a consulta
mais recente. Conexão já aplicada conserva seu acknowledgement mesmo quando
cancelamento chega tarde. Nova consulta redefine o handler e usa a mesma conexão.

## Evidências

Em 20/10/2026 passaram 13 testes de integração SQLite e 1 de encerramento no
crate, sem mocks nem serviços remotos. Testes cobrem tipos, extremos i64,
nullable/PK, nomes escapados, erros, parser único, read-only/ATTACH, inexistência,
limites de linha/célula/bytes/schema/colunas, UTF-8 inválido, cancelamento efetivo,
fila, troca malsucedida e persistência da conexão via `pragma_data_version`.
O teste de Drop interrompe consulta em andamento e verifica worker encerrado e
aquisição posterior de transação exclusiva por conexão independente.

Comandos: `cargo test -p storm-db --locked` e
`cargo clippy -p storm-db --all-targets --locked -- -D warnings`.
Integração GUI/FULL-01 é registrada separadamente pelo integrador.

Fontes técnicas: [rusqlite 0.40.2 Connection](https://docs.rs/rusqlite/0.40.2/rusqlite/struct.Connection.html),
[SQLite interrupt](https://www.sqlite.org/c3ref/interrupt.html),
[SQLite readonly](https://www.sqlite.org/c3ref/stmt_readonly.html) e
[SQLite authorizer](https://www.sqlite.org/c3ref/set_authorizer.html).

## Identidade do resultado para apresentação

`DbService::result_revision()` retorna o id da consulta publicada enquanto há um resultado e `None` quando ele é limpo/uma nova consulta está pendente. A UI usa esse id junto da fonte e escala para invalidar seu cache de larguras; nenhuma dependência egui entra no domínio. O teste GUI `sql_width_cache` executa consultas estreita/larga/estreita e desconexão reais.

## Incremento remoto prioritário — M4 (23/10/2026)

O contrato SQLite acima permanece válido. O novo serviço nativo
`codisseu_db::remote::RemoteService` compartilha AppService/Command/poll e a GUI.
Contrato e limites: m4-database-contract.md.
Conexões remotas não são SQLite, nem usam seu Value/authorizer como proteção.

## Incremento SQL diário — 23/10/2026

[Contrato de seleção/completion/catálogo](../m4-sql-workflow-contract.md),
scanner e execução,
backend de namespaces/recuperação.
