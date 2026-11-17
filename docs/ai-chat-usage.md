# Usar AI Chat

Abra um projeto ou diretório e clique no símbolo de chat no rail direito.
O chat trabalha nessa pasta. Nova conversa cria outra sessão; **Conversas**
reabre chats do projeto atual, inclusive depois de fechar e abrir o Codisseu.
Uma pasta diferente tem seu próprio histórico. Tarefas já iniciadas continuam
na pasta de origem enquanto você trabalha em outro projeto.

Você pode iniciar uma tarefa no chat A, clicar em **Nova conversa AI** e pedir
outra tarefa no chat B. As duas trabalham em paralelo. **Conversas** permite
voltar a qualquer uma enquanto executam; a lista indica trabalho, conclusão,
falha ou aprovação pendente. Uma contagem discreta mostra tarefas em segundo
plano. **Interromper** atua apenas no chat selecionado.

Cada chat conserva suas mensagens, sessão Codex, modelo, raciocínio, permissões
e tokens. Durante a troca, rascunhos e anexos ficam separados. Ao reiniciar a
IDE, mensagens enviadas, configurações e texto do rascunho voltam ao chat
correto. Anexos/contextos ainda não enviados ficam apenas em memória.

Os agentes locais compartilham os arquivos do projeto. Conversas separadas
separam o contexto e a execução, sem criar cópias do código automaticamente.
Se duas tarefas editarem o mesmo arquivo, suas alterações podem se cruzar;
para isolamento de arquivos, abra diretórios/worktrees diferentes.

Escreva e clique na seta para enviar; Ctrl+Enter também envia. Enter insere
uma nova linha, exceto quando escolhe um arquivo na lista de menções.
Prompts longos aumentam a caixa enquanto ela tem foco, até um limite; ao sair
do campo, ele recolhe. O conteúdo continua rolável e preserva texto/seleção.
Controles e rodapé de permissões/cota continuam visíveis. Detalhes de comandos
abrem em uma área com scroll próprio; **Copiar detalhes completos** conserva
o comando e toda a saída disponível, inclusive linhas limitadas na prévia.

- **Cérebro:** escolher Codex/Claude, modelo e raciocínio. O catálogo Codex vem
  da instalação e conta reais. Padrão do CLI conserva a escolha do próprio CLI.
- **Escudo:** Agente é o padrão e pode alterar o projeto. **Acesso total (Full access)** permite também arquivos externos e rede sem pedir aprovação; é uma escolha explícita. Autorizações quando
  necessárias aparecem na conversa. Somente leitura impede alterações. A escolha
  fica salva para novas conversas deste projeto e provedor e depois de reiniciar.
  Conversas anteriores mantêm seu modo; outro projeto/provedor começa em Agente.
- **Contexto do editor:** inclui caminho do arquivo ativo e posição da seleção.
  O botão alterna esse contexto. Para conteúdo ainda não salvo, use o anexo
  explícito da seleção/arquivo; anexar não grava o documento.
- **+:** anexar seleção/arquivo aberto, escolher arquivos e imagens no diálogo
  do sistema ou buscar arquivos do projeto. A escolha explícita pode ser externa.
- **@nome:** buscar arquivo do projeto; setas e Enter escolhem uma referência.
  O arquivo aparece como anexo removível. Escape fecha as sugestões.
- **Ctrl+V com imagem:** com foco no campo do chat, cola uma miniatura removível.
  Texto colado continua texto. Codex recebe a imagem como imagem, sem usar só
  seu caminho no prompt. PNG/JPEG/GIF/WebP são aceitos; animações usam o1º frame.
- **Arquivos copiados e arraste:** Ctrl+V também aceita arquivos copiados pelo
  gerenciador de arquivos. Solte arquivos/imagens sobre o painel para anexar.
  Texto comum, incluindo um caminho escrito, continua texto.
- **/comando:** `/` no início abre ações locais com filtro conforme você digita.
  Setas escolhem; Enter/Tab executam a ação; Escape fecha preservando o texto.
  Estão disponíveis `/new`, `/model`, `/permissions`, `/status`, `/attach`,
  `/context`, `/settings` e `/stop`, conforme o estado da conversa. O catálogo
  é do Codisseu; não importa automaticamente comandos/skills dos CLIs.

São até8 anexos,24MiB agregados e64KiB de contexto textual por mensagem.
Arquivos devem ser texto UTF8 ou imagem suportada; erros aparecem no painel.
Imagens temporárias ficam privadas e são removidas após encerrar o processo.
Ferramentas do agente trabalham no disco: um buffer não salvo só é enviado
quando anexado explicitamente. Conforme o agente grava, arquivos abertos sem
alterações locais são atualizados automaticamente no editor, inclusive com o
painel oculto; arquivos criados aparecem na árvore. A atualização acompanha
as gravações no disco, sem esperar o fim da resposta. Se houver texto seu ainda
não salvo, ele é preservado e um conflito externo é informado. Salvar mantém
a proteção contra sobrescrever mudanças externas.

Ocultar o painel conserva todas as tarefas.
Fechar o aplicativo interrompe as execuções e aguarda a gravação do histórico;
elas não continuam codando com a IDE fechada. Você pode reabrir o chat e enviar
outra mensagem para retomar sua sessão.

Na engrenagem, configure o executável/modelo e consulte a conta ou faça login
ChatGPT pelo link do Codex. Credenciais continuam sob controle do CLI.
Nesta máquina, Codex0.164.0 está em `/home/usuario/.local/bin/codex`, instalado
para a conta por npm global. Esse caminho pode ser informado quando um lançador
gráfico tiver PATH restrito. Claude real não está instalado nesta entrega;
o adaptador usa `print/stream-json` e recusa imagens explicitamente. Foi
verificado com processo controlado, ainda sem conta Claude real.

Preferências, transcrição e IDs de sessão ficam no perfil privado adjacente
à sessão da IDE (`session.ai.json` para `session.json`), fora do projeto.
No Linux, o padrão é `~/.local/state/codisseu/session.ai.json`; se
`XDG_STATE_HOME` ou um perfil explícito estiver configurado, esse diretório muda.
Histórico limitado a64 projetos/16 conversas por projeto/16MiB totais; ao atingir
o limite de conversas, o painel pede remover uma antes de criar outra e não
apaga uma conversa antiga automaticamente. Até8 execuções simultâneas por
instância da IDE (incluindo operações auxiliares do CLI). A cota do provedor
continua compartilhada pela conta e pode limitar as tarefas.
Perfil inválido é preservado, com aviso e histórico só em memória. Não são
armazenadas credenciais nem cópias persistentes dos anexos. O CLI mantém também
seu próprio estado. Markdown ainda tem renderização básica de texto e código.
A lista do Codisseu contém seus próprios registros por projeto; importação
automática de todos os chats criados no VS Code/terminal ainda não faz parte
deste fluxo. Imagens pendentes dos vários rascunhos compartilham um orçamento
de24MiB de pixels, além dos limites de cada mensagem.

[Contrato](ai-chat-concurrency-contract.md), referência de paralelismo,
referência visual anterior
e [protocolo oficial](https://developers.openai.com/codex/app-server/).

## Uso e exclusão de conversas

Ao lado do status aparece discretamente a quantidade de tokens da conversa e a
cota restante da conta, quando fornecidas pelo Codex. Passe o mouse ou clique
para ver entrada, cache, saída, raciocínio, capacidade de contexto, janelas de
cota e tempo até a renovação. **Atualizar limites** consulta o CLI; limites
também são atualizados ao abrir o catálogo, após turnos e periodicamente com o
painel aberto. Dados antigos ou indisponíveis não são tratados como saldo zero.
Contexto livre é aproximação da última chamada; não é saldo de tokens do plano.
Esses números não representam um preço em dinheiro. Claude ainda não fornece
essas métricas pelo adaptador desta entrega.

Em **Conversas**, clique na lixeira ao lado do item: o botão passa a mostrar
**Confirmar** em vermelho. Clique nele novamente para apagar. Não há modal;
clicar fora ou fechar o menu cancela a confirmação. Codex remove também a sessão
vinculada; Claude remove só o registro do Codisseu, com esse escopo no tooltip.
Se a exclusão no Codex não for confirmada, a conversa é mantida com
erro visível. Não apaga arquivos de código nem desfaz alterações feitas pelo
agente. A opção só atua em uma conversa do projeto aberto. Para apagar uma
conversa em execução, interrompa seu turno primeiro; outras conversas podem
continuar trabalhando durante a exclusão.

Contrato e fontes desta continuação: [ai-chat-access-contract.md](ai-chat-access-contract.md).
