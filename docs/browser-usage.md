# Navegador integrado

Clique no **globo ao lado do martelo Build**, no topo da IDE, ou abra
**Exibir → Navegador integrado…**. Ambos abrem uma nova aba com o endereço
selecionado para digitar. Digite o endereço e pressione Enter.
`localhost:5173`, HTTP e HTTPS são aceitos. O servidor da aplicação deve estar
em execução; inicie-o pelo terminal do projeto ou pelo fluxo de execução que
você já utiliza. Abrir o navegador não instala dependências nem inicia scripts.

As páginas aparecem na mesma faixa de abas dos arquivos. Você pode manter
código e página lado a lado nos grupos de editor. Voltar, avançar, recarregar,
parar, endereço e fechar usam os controles nativos e as cores do Codisseu.
O conteúdo é renderizado por Chromium real em processo auxiliar isolado.

| Ação com o navegador ativo | Atalho Linux |
|---|---|
| Selecionar endereço | Ctrl+L |
| Voltar / avançar | Alt+Left / Alt+Right |
| Recarregar | F5 ou Ctrl+R |
| Ferramentas do desenvolvedor | F12 |
| Revisar depuração desta aba na IDE | Shift+F9 ou botão Debug da barra |
| Alternar abas de código/API/navegador | Ctrl+Tab / Ctrl+Shift+Tab |
| Fechar aba atual | Ctrl+W |
| Terminal / configurações | Alt+F12 / Ctrl+Alt+S |

A seleção, cópia, recorte, colagem e digitação da página pertencem ao navegador.
O código aberto atrás mantém seu buffer e histórico. O menu de contexto nativo
oferece clipboard e ferramentas do desenvolvedor. O endereço aceita somente
navegação HTTP/HTTPS e `about:blank`; não executa comandos de shell ou URLs
`file:`, `javascript:` e `data:`.

## Favoritos e acessos

Na barra do navegador, use a **estrela** (ou Ctrl+D) para salvar a página e a
**pasta** para abrir os favoritos. Informe um nome e um projeto/grupo. A busca
considera nome, URL e grupo; Somente projeto atual filtra pelo nome do workspace.
Os itens podem ser editados ou excluídos, e abrem em uma nova aba. Links salvos
ficam disponíveis depois de reiniciar, sem carregar páginas automaticamente.

O botão de **chave** abre Acessos do navegador para a origem da página atual.
Você pode digitar usuário/senha ou clicar em **Ler formulário**, conferir os
campos e usar **Salvar no cofre**. A senha fica no cofre do sistema operacional;
se o cofre estiver indisponível ou bloqueado, o diálogo informa a falha.

Um acesso salvo oferece **Preencher** e a lixeira para excluí-lo. Preencher
coloca usuário/senha no formulário; você confere e envia o login. Protocolo,
host e porta fazem parte da origem: localhost:3000 não recebe a senha de
localhost:5173. Não existe importação de senhas ou sessões de navegadores pessoais.

Leitura/preenchimento são gestos explícitos no formulário principal da aba.
Use o cadastro manual em páginas com login em iframe, troca de senha, controles
personalizados ou mais de um campo de senha. A leitura/preenchimento usa o canal
privado da aba: feche DevTools/inspetor ou encerre o Debug dessa aba para usá-lo.
Essas ferramentas não são encerradas automaticamente pelo gerenciador de acessos.
Navegar/fechar cancela a operação; não há envio automático do formulário.

Favoritos e metadados dos acessos pertencem ao perfil da IDE, fora do workspace.
Cookies e storage continuam no perfil temporário do motor; reabrir uma página
pode exigir novo login, com o acesso salvo disponível para preencher novamente.
Até512favoritos/256acessos, sujeitos ao limite agregado1MiB da biblioteca. O cofre
e a lista de metadados são gravações distintas; o erro informa quando é necessário
conferir uma operação que pode ter terminado após o tempo de espera.

## Inspecionar a página

Clique no botão **Inspecionar página** (alvo) na barra do navegador para abrir o
**Inspetor frontend**, um painel nativo abaixo da página. Arraste o divisor para
ajustar sua altura. Em áreas estreitas, os controles da barra ocupam duas linhas
e as abas usam os nomes curtos DOM, Dados e JS. Não precisa configurar um projeto
para usar este painel.

- **Rede**: captura requests novos; recarregue a página após abrir o inspetor.
  Filtre por URL/método e selecione uma linha para ver headers, payload e response.
  **Capturar resposta** obtém o corpo depois que a request termina; não repete a
  chamada. Histórico limitado a 200 entradas; corpos pequenos são carregados sob
  demanda, até 1 MiB, quando o tamanho pode ser determinado com segurança.
- **Componentes / DOM**: atualize a árvore e selecione o elemento para destacar
  na página e consultar atributos, estilos e propriedades. Inclui custom elements
  e shadow DOM aberto; não representa props/state internos de React/Vue/Angular. A enumeração
  de propriedades não executa getters.
- **Armazenamento**: cookies da página (inclusive HttpOnly), localStorage e
  sessionStorage. Selecione uma entrada ou use **Nova entrada**, edite e clique
  **Salvar valor**. **Excluir valor** remove a entrada da página real. O escopo
  acompanha a origem ativa e muda junto com a navegação.
- **Variáveis**: pause, continue, avance passos e selecione frames/scopes. Avalie
  expressões ou adicione watches. Avaliar executa JavaScript na página; quando
  pausado, usa o frame selecionado. Breakpoints por URL/linha ficam na seção
  expansível; para breakpoints nas fontes JS/TS do projeto, use o Debug do editor.
  F7/F8/Shift+F8/F9 controlam a execução com o inspetor nativo ativo.
- **Tokens**: cole um JWT ou envie um valor de armazenamento por **Examinar como
  JWT**. A decodificação de header/payload acontece localmente e **não verifica
  a assinatura ou autenticidade**.

Valores de armazenamento, corpos e variáveis ficam ocultos por padrão. Use
**Revelar valores** para vê-los e os botões de cópia para enviar ao clipboard.
Os dados de inspeção ficam na memória da sessão e são descartados ao fechar.
Navegar invalida seleções, objetos, avaliações e formulários. Preservar histórico
conserva até 200 requests anteriores na sessão; desative/limpe para descartá-las.
O perfil temporário do navegador é independente
dos seus navegadores pessoais.

F12 continua abrindo o frontend DevTools original do mesmo Chromium dentro da
aba, com Elements, Console, Network e Sources. O divisor pode ser arrastado;
em uma área estreita, as ferramentas ficam abaixo da página. O inspetor nativo,
DevTools e o depurador do editor são modos exclusivos por aba: iniciar um modo
libera o anterior e retoma eventual execução pausada. Fechar as ferramentas
mantém a página aberta. Nenhum plugin de framework é instalado automaticamente.

## Depurar JavaScript/TypeScript pelo editor

Abra o projeto e a página da aplicação, salve as fontes que serão servidas e
clique em **Debug** na barra do navegador. A revisão usa o diálogo de depuração
existente: selecione a aba, confira a raiz web e o adapter JavaScript. Autorize
a configuração somente depois de revisá-la e use **Start Debug Configuration**.
A IDE conecta à aba aberta. Ela não inicia um Chrome externo.

Configure breakpoints nas fontes do projeto; o painel Debug apresenta pilha,
variáveis, avaliação e watches. Quando pausado, F8 avança, F7 entra, Shift+F8 sai,
F9 continua e Alt+F8 abre avaliação. Shift+F5 encerra a conexão e conserva a
página. TypeScript depende dos source maps publicados pelo seu servidor/build;
sem mapas corretos não há correspondência automática confiável entre TS e JS.
O corpus valida JavaScript simples e Source Map V3 local, sem certificar todos
os bundlers, frameworks ou destinos remotos.

A escolha da aba e o endereço privado de conexão não são restaurados nem salvos
no perfil. Ao reutilizar uma configuração, selecione novamente uma aba desta
sessão e revise a autorização. Abrir um projeto não inicia o navegador ou debug.

## Instalação e limites

O pacote Linux completo inclui o helper Rust, CEF/Chromium, assets oficiais do
DevTools e avisos de licença. No checkout de desenvolvimento, a preparação é
explícita:

```bash
python3 tools/install_browser_runtime.py
python3 tools/build_browser_engine.py --online
cargo run --locked -p codisseu -- --browser 'http://localhost:5173'
```

O download do SDK é pinado; `--online` permite preparar as dependências Cargo
somente durante esse build. Depois de preparadas, o build do helper funciona
sem esse argumento e verifica os recursos privados. A IDE principal não linka
`libcef`. Se os recursos estiverem ausentes ou houver falha do motor, a aba
mostra erro e permite tentar novamente; os documentos continuam disponíveis.

Até oito páginas. Cookies e storage usam perfil temporário privado, eliminado
quando o motor é encerrado; não há restauração de login entre sessões, importação
de perfil pessoal. Favoritos e acessos têm persistência independente conforme a seção acima. Downloads, upload por seletor de
arquivos, diálogos JavaScript e permissões de câmera/microfone/dispositivos não
são oferecidos neste recorte. Não é um substituto certificado para todo navegador
pessoal, DRM/codecs proprietários, OAuth por protocolos externos ou extensões.

Linux x86_64 é o alvo exercitado, com X11/XWayland e namespaces de usuário
habilitados para o sandbox dos renderers. Não há fallback `--no-sandbox`, alteração
de sysctl ou instalação setuid. O broker e o serviço de rede têm papéis diferentes
dos renderers; não se afirma que todo subprocesso Chromium tem o mesmo sandbox.
Wayland nativo, outras arquiteturas, leitores de tela sobre o DOM e a posição
da janela de candidatos de todos os IMEs ainda precisam de aceitação específica.
Digitação/commit Unicode foram exercitados; isso não certifica todo método de entrada.

Referências e confiança: comparação local.
Arquitetura e critérios: [contrato](browser-contract.md),
IPC. Resultados e limitações da entrega:
[status](status.md).
