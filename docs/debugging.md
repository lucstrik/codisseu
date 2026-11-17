# Depurar no Codisseu

Abra o projeto e use **Executar → Debug…**, **Shift+F9** ou o ícone de debug
na barra superior. A configuração mostra o executável, arquivo de entrada,
diretório e argumentos antes da execução. Cada início cria uma sessão separada;
é possível manter PHP e JS/TS pausados simultaneamente.

## Página no navegador integrado

Abra **Exibir → Navegador integrado…** e o endereço da aplicação já servida.
O botão Debug da barra dessa aba ou Shift+F9 com foco nela abre a revisão do
mesmo depurador, conectada à página existente. Salve as fontes e confira a raiz
web/aba antes de autorizar. DevTools (F12) e debug do editor são alternados por
aba; Stop mantém a página aberta. Navegação, perfis e source maps têm os limites
registrados no [guia do navegador](browser-usage.md). A opção Chrome externa
preexistente continua disponível nas configurações normais.

## Uso diário

1. Salve os arquivos antes de iniciar um programa. O depurador executa os arquivos
   do disco; não há gravação automática dos seus buffers.
2. Clique no espaço entre o número da linha e a seta de recolhimento, ou use
   **Ctrl+F8**, para marcar/remover um breakpoint. O centro da bolinha foi
   preservado. Vermelho preenchido indica habilitado; uma marca interna indica
   confirmação do adaptador da sessão selecionada. Contorno indica pendência,
   cruz indica ausência de confirmação com mensagem e cinza riscado indica mute.
   O tooltip e **View Breakpoints** explicam o estado e a mensagem do adaptador.
3. Em Debug, escolha a sessão. **Threads & Variables** mostra pilha à esquerda e
   variáveis à direita. Arraste o divisor para ajustar as larguras. Clique em um
   frame para navegar e expandir objetos. O cabeçalho informa o motivo da pausa.
4. **F9** continua, **F8** avança uma linha, **F7** entra na chamada,
   **Shift+F8** sai da chamada e **Shift+F5** para a sessão selecionada.
   **Alt+5** mostra/oculta o painel. Ctrl+F2 continua pertencendo à edição.
5. **Alt+F8** foca o campo Evaluate; **Enter** avalia somente com a sessão pausada.
   O rascunho da expressão pertence à sessão. Watches também são
   expressões executáveis e pertencem à sessão selecionada. Os consoles separam
   resultado de expressões e saída real do programa.
6. **Exception Breakpoints** oferece os filtros que o adaptador efetivamente
   anuncia. Alterações são aplicadas à sessão e confirmadas pelo adaptador;
   os nomes diferem entre PHP e Node. Mute controla o conjunto de breakpoints
   de linha, não desativa a política de exceções.

As configurações e breakpoints são lembrados por workspace no perfil do Codisseu,
fora do projeto. Valores de ambiente e autorização de execução ficam na sessão
atual da IDE. Nenhum programa inicia somente por abrir/restaurar a pasta.

## Condições e logpoints

Botão direito na bolinha → **Edit Breakpoint…** abre as propriedades. Em uma
linha vazia do gutter, use **Add Conditional Breakpoint…** ou **Add Logging
Breakpoint…**. **Ctrl+Shift+F8** edita o ponto da linha do cursor; na lista
**View Breakpoints**, a engrenagem edita e o checkbox controla **Enabled**.

- **Condition**: expressão booleana, como `$i === 3` em PHP ou `i === 3` em JS/TS.
- **Hit count**: por exemplo `3`, para parar na terceira passagem. Operadores
  adicionais dependem do adaptador; a sintaxe inválida aparece como erro.
- Desmarque **Suspend** e preencha **Log message** para registrar sem pausar.
  `i={$i}` em PHP ou `i={i}` em JS/TS inclui o valor. Mensagem vazia gera a
  identificação do breakpoint. Condição/contagem também podem filtrar o log.
- **Alt+clique** no ponto liga/desliga sem apagá-lo; **Shift+clique** prepara um
  logpoint. Apply ou Enter confirma; Escape/Cancel preserva a configuração.

Mute é temporário e preserva o Enabled de cada ponto. Opções e pontos desligados
são lembrados por workspace. Abrir o projeto não avalia expressões; elas só
podem ser executadas no programa de uma sessão de debug iniciada explicitamente.
Adaptadores sem a capacidade necessária rejeitam o ponto, sem convertê-lo em
parada incondicional. A posição da bolinha e o folding permanecem iguais.

## PHP

Para um script, selecione PHP / Launch, o interpretador e o arquivo de entrada.
O padrão `php` usa o PHP privado com Xdebug no pacote completo. Um caminho
explícito continua usando o interpretador escolhido. Antes de executar a fonte,
a IDE verifica a extensão/modo debug com uma sonda limitada e cancelável. Um
interpretador sem Xdebug ou `XDEBUG_MODE=off` falha com mensagem, sem executar o
programa selecionado. Nesta máquina foram
exercitados PHP8.4.27 e Xdebug3.5.3; outros interpretadores precisam de sua própria
extensão compatível. O modo Launch usa loopback e porta dinâmica por padrão.

Para uma aplicação web ou Docker, escolha **Listen for PHP Debug Connections**.
Configure a porta, geralmente9003, e o endereço de escuta. Se a porta estiver ocupada por outro listener, use outra
porta nas duas pontas ou encerre esse listener. O padrão127.0.0.1 atende
processos locais; um container precisa alcançar o host pelo endereço de rede
apropriado. Um endereço como0.0.0.0 aceita conexões nas interfaces de rede e só
deve ser escolhido explicitamente quando necessário à sua configuração.

No PHP/container configure `xdebug.mode=debug`, `xdebug.start_with_request=trigger`,
`xdebug.client_host` com o endereço do host acessível pelo PHP e a mesma
`xdebug.client_port`. Ative o trigger Xdebug na requisição/CLI. A IDE não altera
seu php.ini, Docker ou aplicação automaticamente.

Em **Path mappings**, informe uma associação por linha, por exemplo
`/var/www/html=/caminho/local/projeto`. O lado esquerdo é o caminho visto pelo PHP;
o direito é a cópia local aberta no Codisseu. O listener permanece disponível
para requisições seguintes. Pause depende do control socket oferecido pelo Xdebug
e acessível ao adaptador; uma execução remota/container pode não disponibilizá-lo.
Quando indisponível, o erro do adaptador é mostrado; breakpoints e `xdebug_break()`
continuam sendo opções para interromper a execução.

## JavaScript e TypeScript

**Node.js / TypeScript → Launch** recebe o arquivo de entrada e os argumentos
separados, um por linha. Não executa npm install ou scripts de preparação.
Para Attach, inicie o seu processo com Inspector e informe a porta, geralmente9229.
Parar a sessão Attach desconecta o depurador; não toma posse do processo externo.

Para TypeScript compilado, habilite source maps na compilação do projeto e
selecione o JavaScript gerado como entrada. Mantenha **Enable source maps**
ativado e os breakpoints no `.ts` original. Não há compilação automática implícita;
o corpus próprio exercita `tsc --sourceMap`, arquivo/linha originais e variáveis.

Para frontend no navegador, adicione **JavaScript Debug (Chrome)** e a URL local
da aplicação já iniciada. O diretório corresponde à raiz dos fontes. Escolha
um navegador Chromium compatível instalado ou deixe sua descoberta automática.
Attach permite usar uma instância explicitamente iniciada com depuração remota;
a IDE não altera nem se conecta automaticamente ao seu perfil normal do navegador.

## Ferramentas e limites

Os adaptadores são externos e abertos: PHP Debug1.40.2 e js-debug1.140.0.
`python3 tools/install_debug_adapters.py --verify` verifica a instalação local;
sem `--verify`, o instalador explícito instala somente os pacotes fixados no
manifesto, conferindo seus hashes. Node local existente executa os adaptadores.

Suporte validado e limites exatos ficam em [status.md](status.md). Condições de
breakpoint, logpoints, políticas de exceções e toda a integração específica de
Laravel não são equivalência integral ao PhpStorm. Alterar código durante uma
pausa não recarrega automaticamente o programa: salve e reinicie para alinhar
fonte, linhas e código executado. Arquivos dirty não exibem linha de execução
como se correspondessem exatamente ao disco.
