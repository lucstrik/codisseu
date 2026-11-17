# Jira no Codisseu

Abra **Configurações → Integrações · Jira** e marque **Mostrar Jira na barra
lateral direita**. A entrada Jira aparece junto de AI Chat e Database. Clique
nela e em **Conectar ao Jira…**. Informe o endereço HTTPS da empresa, o email
Atlassian e seu token de API. **Guardar token no cofre do sistema** é opcional;
sem essa opção, um token novo é usado somente durante a sessão. Com a chave
guardada, o Jira reconecta em segundo plano ao reabrir a IDE, mantendo seus filtros.
Não é necessário preencher o token ou clicar em Conectar novamente.
Se o cofre estiver bloqueado, desbloqueie-o e use Conectar com o campo de token
vazio para tentar novamente. Token expirado precisa ser substituído.
Desconectar mantém a chave, mas desativa a reconexão até você conectar novamente.
Esquecer token também remove a entrada da conexão atual do cofre.

Para tokens com escopos, abra a configuração avançada e informe a URL
`https://api.atlassian.com/ex/jira/CLOUD_ID`. Tokens comuns usam o endereço do
próprio site. A integração implementada é Jira Cloud REST v3.

O painel começa com suas tarefas em aberto. Busque por título, conteúdo ou
chave; escolha status e abra o funil para combinar projeto, tipo, responsável,
história/épico e equipe. O seletor de equipes permite escolher entre campos
Team/Squad/Tempo Team disponibilizados pela empresa. A seleção de responsável
desmarca Somente minhas tarefas. O ícone de redefinição restaura os filtros
iniciais. Alterações são salvas automaticamente; vistas nomeadas permitem guardar
até20combinações. Configurações pertencem ao perfil da IDE.

Use as setas para navegar páginas de até50cards. Somente a página atual é mantida
em memória; comentários e apontamentos usam páginas de20. Após64páginas na mesma
busca, refine os filtros ou atualize para recomeçar. A contagem do painel é da
página, sem estimar o tamanho do histórico completo.

Clique em uma tarefa para abrir o leitor ao lado do painel. Descrição apresenta
Markdown/ADF; Pessoas e vínculos mostra responsável, criador, pai e relações.
Avatares aparecem pequenos e redondos; imagens anexadas têm prévia sob demanda.
Imagens indisponíveis conservam um aviso/fallback e o link Abrir no Jira permite
consultar recursos específicos do servidor. Comentários e Horas são carregados
ao abrir a seção. As abas Jira ficam na mesma faixa das abas de arquivos;
clique diretamente no arquivo para voltar. Escape retorna ao arquivo do grupo,
Ctrl+Tab alterna recursos e Ctrl+W fecha somente a aba ativa.

Em **Horas → Registrar horas…**, informe duração, início com fuso e descrição;
revise e confirme. A IDE cria um worklog nativo do Jira, mantendo a estimativa
restante. Não é uma implementação do produto Tempo Timesheets. Se a resposta
falhar depois do envio, confira os registros no Jira antes de tentar novamente.
O aplicativo não repete automaticamente a gravação.

Para consultar suas horas, abra o painel **Jira** na barra direita e clique no
**ícone de relógio** do cabeçalho (**Abrir Minhas horas**). Só esse acesso cria a
aba Minhas horas na mesma faixa dos arquivos. Ativar o Jira ou abrir uma tarefa não abre essa aba automaticamente.
Ela sempre usa a conta conectada, sem opção de consultar outras pessoas.

Escolha **Mês**, **Semana** ou **Dia** e navegue pelas setas, **Hoje** ou
**Escolher data…** (dd/mm/aaaa). Veja horas, dias com apontamentos e cards do
período. Clique num dia do calendário para ver seus registros; clique no card
para abri-lo e usar **Horas → Registrar horas…**. Totais usam a data de início
do trabalho no fuso do computador; um registro fica no dia em que começou.
A seção Horas de um card também mostra apenas registros da conta conectada,
com subtotal da página. Para conferir o período inteiro, use a aba Minhas horas.

**Localizar cards para apontar** aplica o filtro **Criados no mês** da data
selecionada, atribuídos a você e com todos os status, incluindo concluídos.
O funil permite digitar outro mês em aaaa-mm; o X remove o mês e Redefinir
restaura os filtros iniciais. O filtro é salvo automaticamente e pode compor
uma vista nomeada. Responsável significa o atual, não um histórico de atribuições.
O relatório de horas continua incluindo seus registros de cards transferidos.

Históricos grandes são conferidos em segundo plano, em lotes de120requisições.
A consulta busca apenas sua conta e as datas do mês/semana/dia selecionado.
**Horas encontradas** indica valor parcial; use **Continuar carregamento** até
concluir ou **Tentar novamente** após erro. Interromper/erro não significa zero horas. **Atualizar horas** reinicia
a leitura. Após carregar todas as páginas da consulta filtrada, o título passa a
**Horas no período**. Não há uma busca global posterior ao recorte; os resultados
seguem o índice JQL e as permissões do Jira. Não há armazenamento do conteúdo dos apontamentos em disco.

Desmarcar a integração oculta o painel e descarta seus dados de sessão. O token
guardado permanece no cofre até ser removido explicitamente. A IDE não importa
tokens de arquivos de projeto. A reconexão automática usa somente a chave do cofre
da conta configurada, uma tentativa por restauração, sem abrir abas remotas.

Contrato, fontes, limites e cenários: [jira-contract.md](jira-contract.md).
Resultados efetivamente executados: [status.md](status.md).
