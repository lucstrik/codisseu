# SQL diário: instrução, metadados e controles — 23/10/2026

Pedido adicional do usuário: uma conexão MySQL sem banco padrão deve permitir
consultar nomes qualificados em vários bancos, explorar namespaces e completar
tabelas/colunas. Ctrl+Enter executa seleção explícita ou instrução do cursor.
Interface de banco usa ícones próprios com nome acessível e tooltip; formulário
conserva ações textuais finais e troca controles +/− desenhados como texto por
ícones vetoriais de toolbar. Referência observada é registrada separadamente.

## Metadados

Não é necessário selecionar um único banco para conectar MySQL. Lista de bancos
é explícita ou solicitada pelo autocomplete invocado/digitação. Tabelas só são
consultadas para o namespace expandido/qualificado; colunas só para a tabela
relevante. Nenhuma leitura de linhas/COUNT/indexação ampla automática. Uma sessão
SQL, os mesmos timeouts, filas e regras de proteção anteriores. Listas limitadas
a256 itens/1MiB por requisição; cache de8 namespaces/32 tabelas, invalidado ao
conectar/desconectar/atualizar. Consulta inválida recuperável preserva conexão;
falha de rede, timeout ou cancelamento fecha conservadoramente. PostgreSQL usa
savepoint para não perder alterações manuais anteriores por erro de instrução.

## Completion

Sugestões reais de namespace/tabela/coluna vêm de metadados da sessão vinculada.
Ctrl+Space explícito e digitação de prefixo/ponto no editor SQL ativam popup;
comentários/strings não disparam consulta de metadados. Sem resposta velha
aplicada em documento/versão/sessão diferentes. Enter/Tab aplicam um delta atômico
com Undo/Redo, setas navegam, Escape cancela. Identificadores escapados pelo
dialeto; aliases de FROM/JOIN simples suportados, ambiguidades não inventadas.
Sem prometer análise equivalente de todos os dialetos/subqueries do DataGrip.

## Cenários

Fixtures locais MySQL com dois bancos e PostgreSQL com dois schemas; execução
na segunda instrução de um arquivo, strings/comentários com ponto e vírgula,
seleção explícita, consulta inválida seguida de válida na mesma sessão, schema
qualificado, completion de tabela e de coluna/alias, aplicação com Undo/Redo e
mudança de datasource/versão invalidando popup. Interface real capturada nas
escalas1/1,25; sem dados corporativos. Não corrigir silenciosamente SQL sem FROM.

## Correção de catálogos grandes — 23/10/2026

O limite256 deixa de rejeitar tabelas/views de um namespace inteiro. O novo
contrato paginado usa até256 nomes por página,
cursores anterior/próxima e filtro literal de prefixo no servidor. Completion
usa cache próprio de8 prefixos, separado das8 páginas do explorer, e busca
nomes além da primeira página. Namespaces/colunas mantêm limites anteriores.
