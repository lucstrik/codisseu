# AI Chat — conversas independentes e execução paralela

Atualização10/11/2026: diretório efetivo multirraiz, checkpoint do rascunho da
conversa estacionada e lista compacta em
auditoria da sessão. A ordenação usa última
atividade de mensagens; seleção/configuração/rascunho não mudam sua posição.
Perfis antigos sem timestamp preservam idade desconhecida.

Cenário AI-07,25/10/2026. Continuação do painel nativo existente; não replaneja
M3–M8. Pedido: revisar a extensão Codex instalada e permitir iniciar B enquanto
A continua trabalhando, alternar entre ambas e retomar cada histórico depois.

## Diagnóstico anterior à alteração

`ChatService.running` era único. `NewConversation` e `SelectChat` recusavam
execução durante um turno; mudar workspace cancelava o turno anterior. Perfis
já separavam conversas e projetos e guardavam thread ID, transcript, modelo,
esforço, permissões e uso. Portanto persistência existia, paralelismo entre chats
não. Origem: código local anterior, confiança observed por inspeção de fonte.

## Contrato desta fatia

- Novo chat e seleção de histórico permanecem disponíveis durante um turno.
  Cada conversa tem identidade estável, diretório de origem, thread ID Codex,
  estado, transcript, uso, modelo/permissões, aprovações e execução próprios.
  Poll recebe também eventos das conversas que não estão visíveis. Eventos nunca
  são aplicados à conversa selecionada por coincidência de item/approval ID.
- Duas conversas podem trabalhar simultaneamente no mesmo projeto. Interromper
  cancela somente a conversa visível. Abrir outra conversa ou esconder o painel
  não interrompe trabalho. Trocar projeto conserva tarefas anteriores no seu
  diretório de origem; a nova conversa utiliza exclusivamente o projeto aberto.
  Fechar o aplicativo cancela todas as execuções e aguarda recolhimento/ACK de
  exclusão e gravação do perfil antes de sair. Reiniciar recupera o histórico,
  sem fingir que o processo anterior continuou trabalhando com a IDE fechada.
- O composer preserva rascunho e anexos por conversa/projeto durante alternância.
  Texto não enviado também persiste no perfil, conforme rascunho observado no
  reinício da referência. Texto/anexos de A não aparecem em B. Anexos/contextos
  ainda não enviados são transitórios; bytes de imagens não entram no perfil.
  Preferências persistidas continuam por conversa. Não modificar
  credenciais nem copiar auth do CLI.
- Histórico indica execução/aprovação/falha de cada conversa. Contagem discreta
  informa trabalho em segundo plano. Botões de envio, interrupção e permissões
  operam na conversa selecionada; remover chat em execução exige interrompê-lo
  primeiro. Exclusão continua com dois cliques, Confirmar vermelho, sem modal.
  Exclusão de chat ocioso não afeta turnos de outras conversas.
- Recursos têm limites explícitos e falha visível; nenhuma conversa em execução
  pode ser expulsa silenciosamente pela poda do histórico. Manter isolamento de
  projeto, limites UTF8/bytes/filas, worker de persistência e gravação atômica.
  Cota da conta é compartilhada; contadores de tokens são da conversa.
- Chats locais compartilham os arquivos do diretório aberto. Não criar worktree
  automaticamente nem prometer isolamento de arquivos entre tarefas. Modelo e
  regras do próprio Codex/conta continuam aplicáveis.

## Referência e equivalência

Referência instalada e observações controladas serão registradas em
`ai-chat-concurrency-reference.md`, com build/cenário/confiança. A documentação
oficial do App Server define thread/start, thread/resume, turn/start direcionado
por threadId e turn/interrupt; isso documenta identidade e retomada, sem por si
só provar fidelidade visual ou comportamento completo da extensão.
Fonte: https://learn.chatgpt.com/docs/app-server (consultada25/10/2026).

Implementação própria Rust sobre protocolo do CLI, sem incorporar bundle privado
da extensão. Não declarar paridade integral: separar o que foi observado,
implementado/testado e o que permanece fora desta fatia.

## Evidência exigida

1. Processos controlados: A bloqueada em barreira enquanto B conclui; retornar A,
   saída/uso/config/aprovação isolados, cancelar A sem parar B, cwd estável ao
   trocar projeto, apagar ociosa enquanto outra roda, persistência/reinício,
   limites e recolhimento de subprocessos.
2. Widgets: Novo/Conversas habilitados, rascunhos/anexos distintos, indicadores e
   interrupção por conversa; validação dos controles anteriores preservada.
3. Interface nativa em fixture/perfil/display próprios, escalas1 e1,25: duas
   conversas, alternância durante execução, conclusão visível e retomada após
   reinício. Registrar screenshots reais, hash do binário e limpeza dos perfis.
4. Turnos Codex reais mínimos em arquivos exclusivos da fixture para verificar
   integração além do simulador; não confundir compilação com teste funcional.
