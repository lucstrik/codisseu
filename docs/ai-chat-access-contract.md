# AI Chat — acesso total, uso e exclusão,25/10/2026

Continuação do painel existente; não replaneja M3–M8.

Atualização03/11/2026: escolha explícita também persiste como padrão de novas
conversas por projeto/provedor; selecionar histórico não muda esse padrão.
Campo aditivo defaults no schema1, sem conceder permissões a provedores novos.
Contrato complementar: fluidez.

- Menu de permissões oferece Somente leitura, Agente e Acesso total (Full access).
  Padrão continua Agente. Seleção explícita FullAccess usa Codex
  danger-full-access/never, turn sandboxPolicy dangerFullAccess. Claude usa
  bypassPermissions/default tools. Limites do próprio provedor/administrador
  continuam válidos. Modo salvo por conversa e enviado novamente ao retomar.
- Indicador pequeno, cinza, na mesma linha do status: tokens totais informados
  pelo CLI e cota restante quando disponível. Hover/click mostra entrada/cache/
  saída/raciocínio, último contexto e janela do modelo, janelas de cota e reset.
  Contexto restante calculado da última chamada é aproximação identificada;
  não equivale a tokens restantes do plano nem preço em dinheiro. Ausência é —,
  jamais zero inventado; reset vencido requer nova consulta, sem supor recuperação.
  Usar eventos thread/tokenUsage/updated e account/rateLimits/updated, mais
  account/rateLimits/read; atualização manual. Contadores da conversa persistem,
  cota da conta é transitória. Falha opcional de cota não falha o turno concluído.
- Conversas tem lixeira por item. A orientação posterior do usuário remove a modal: primeiro clique troca a lixeira por Confirmar em vermelho, segundo clique apaga; fechar o menu/clicar fora cancela. Exclusão
  Codex remove o thread escolhido via thread/delete e depois o registro local;
  falha mantém a conversa para tentar novamente. Sem varrer/deletar arquivos do
  CLI manualmente. Sem sessionID (chat vazio), remoção só local. Claude remove
  registro Codisseu e informa escopo; histórico externo Claude não é manipulado.
  Só aceitar IDs pertencentes ao workspace atual. Chat ativo apagado abre outro
  ou estado vazio; não pode reaparecer após polling, nova mensagem ou reinício.
- Ícones seguem paleta cinza, recuos anteriores e procedência licenciada.

Fontes documented: https://learn.chatgpt.com/docs/app-server (eventos/limites/
thread/delete) e schemas gerados pelo Codex0.164.0 em artifacts/ai-chat-20261025/schema.
Referência visual observada anteriormente em docs/ai-chat-vscode-reference.md;
não houve nova medição do VS Code nesta fatia. Dados reais de conta só pelo CLI,
sem capturar identidade/credenciais. Verificar processos controlados, perfis,
widgets e janela nativa1/1,25; cenário real em fixture sem arquivos pessoais.
