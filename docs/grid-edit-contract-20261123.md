# Edição de resultados — 23/11/2026

Incremento M4 solicitado pelo usuário: duplo clique na célula de uma consulta
simples permite editar, preservando a proposta local até Submit ou Commit.
Implementação própria Rust/egui, sobre os workers e sessões existentes.

## Comportamento e integridade

- Duplo clique ou F2 abre editor na própria célula. Enter aceita a proposta;
  Escape cancela somente a edição atual. Células alteradas recebem destaque.
  A edição usa os índices originais, mesmo após filtro/ordenação local.
- O schema determina o editor e a validação, inclusive quando o valor original
  é NULL. Números rejeitam texto na digitação/colagem; limites de inteiro,
  precisão/escala, NULL, texto, booleanos, JSON e formatos temporais são
  verificados antes de propor e novamente no worker. O servidor continua
  validando constraints, permissões e triggers.
- Escrita é explícita. SQLite mantém Connect somente leitura; ativar edição
  reabre o mesmo arquivo em modo writable. Fontes SQL remotas respeitam a
  opção Read-only já existente. Colunas geradas, PK e tipos sem tratamento
  seguro permanecem protegidos.
- Origem comprovada por AST e metadados: uma tabela base, sem joins,
  agregações ou cálculos, com todas as partes da PK na projeção. Sem identidade
  inequívoca a grade informa somente leitura. Não adivinhar chaves.
- Submit aplica o lote em transação manual na mesma conexão. Commit aceita
  também propostas locais: aplica e confirma como uma operação sequencial do
  worker, sem reconexão ou repetição automática. Rollback descarta alterações
  enviadas. Revert descarta propostas ainda locais.
- UPDATE parametrizado por dialeto, quoting de identificadores, comparação
  otimista dos valores originais e exatamente uma linha por identidade.
  Múltiplas células da mesma linha viram um único UPDATE. Falha/conflito
  reverte o lote e permanece erro; nenhuma desconexão equivale a Commit.
- Nova execução, troca de conexão/projeto e saída protegem propostas e
  transações pendentes. Resultado confirmado precisa ser recarregado antes de
  outra edição para incorporar mudanças do servidor.

## Corpus

SQLite temporário real e PostgreSQL/MySQL/MariaDB em containers próprios,
somente loopback, sem bancos de usuário. Cobrir commit/rollback com observador
independente, alterações múltiplas, chave composta, NULL numérico, limites,
precisão, constraints, conflito, sem chave, joins, colunas geradas, quoting e
conteúdo hostil. GUI por widgets reais e janela X11 privada em 100/125% quando
disponível. Compilação não substitui esses testes.

A cobertura anunciada é somente a matriz efetivamente executada. Aurora exige
validação AWS própria; MongoDB/Redis/Memcached não têm UPDATE SQL nem o mesmo
contrato de transação. Nenhuma equivalência universal com drivers JDBC é
inferida desses testes. A liberação pública segue o contrato de publicação,
com pacote do CI aprovado e download/hash verificados. Remover apenas recursos
Docker e diretórios descartáveis desta execução, conservando fontes de teste
e evidências pequenas.

## Limites específicos

Tipos desconhecidos, domínios/arrays PostgreSQL e binários permanecem somente
leitura. SQLite também protege tabelas virtuais/shadow e rejeita escritas por
triggers no caminho controlado; um erro reverte o lote. Seus inteiros usam os
64 bits nativos; decimais que não cabem sem perda no armazenamento SQLite são
recusados. INTEGER MySQL/PostgreSQL segue os limites reais do schema.

JSON mantém os bytes numéricos no cliente. MySQL armazena números fracionários
JSON em sua representação nativa DOUBLE, que pode arredondar independentemente
do cliente; isso foi observado separadamente no servidor. DECIMAL SQL conserva
a precisão declarada. Para dígitos decimais exatos dentro de JSON MySQL, usar
uma string JSON. Essa limitação do servidor não é anunciada como preservação
universal de números JSON.
