# Navegador integrado — contrato,02/11/2026

Pedido: navegar/visualizar frontend e páginas da internet em abas do Codisseu,
com ferramentas reais de inspeção/depuração, seguindo o comportamento observado
no VSCode. Usar organização PhpStorm onde houver equivalente comprovado; caso
contrário manter os Tokens/abas/controles Codisseu. Referência e confiança em
browser-reference-20261102.md. Não se anuncia paridade integral sem corpus.

## Arquitetura e isolamento

A IDE continua Rust/egui/eframe e o editor não muda de motor. Somente o conteúdo
web usa Chromium Embedded Framework aberto, via bindings Rust cef-rs pinados.
Não carregar IDE proprietário, Electron, Tauri ou frontend HTML para a interface
do produto. CEF roda em processo auxiliar Rust próprio, iniciado apenas ao abrir
explicitamente um navegador. Suas falhas não encerram o editor.

CEF OSR entrega os pixels por OnPaint; egui apresenta a textura e encaminha
input ao motor. Não usar screenshots periódicas/CDP como substituto de renderer,
X11 reparenting, browser externo disfarçado ou HTML fingindo controles nativos.
DevTools verdadeiro deve operar na página integrada, inclusive Console, Elements,
Network e Sources/breakpoints; sua superfície é conteúdo do motor. Toolbar,
abas, endereços, erro/foco/fechamento continuam controles nativos Codisseu.

codisseu-browser mantém protocolo, serviço e transporte sem egui/CEF. O helper
apps/codisseu-browser-engine mantém dependência/SDK CEF fora do processo da IDE.
IPC local privado, diretório0700, frames tipados e limitados. CEF não abre
endpoint TCP de depuração. Apenas após Debug/autorização, uma ponte temporária
para js-debug pode escutar loopback em porta efêmera, com nonce256bit em caminho
WS, validação Host/Origin, uma aba/epoch e um cliente, sem listar outras páginas
ou expor o nonce na descoberta HTTP. O token não persiste nem aparece nos logs.
Stop/fechamento derrubam o listener. IDs de roteamento Target nessa ponte são
virtuais explícitos; Runtime/Debugger/Page e seus resultados vêm do CEF real. Fluxos/filas limitados, coalescência de frames e descarte de
eventos de abas fechadas; nenhum trabalho de rede/engine bloqueia a GUI.

Sandbox de renderers deve permanecer ativo para conteúdo remoto. Não aceitar
`--no-sandbox`, desabilitar TLS ou copiar o perfil do navegador do usuário.
Não instalar binário setuid nem alterar sysctl/permissões globais para conseguir
rodar. Falha do ambiente de sandbox precisa de erro explícito/reprodução e uma
solução suportada antes de liberar internet. Bibliotecas, origem/versão/hash/
notices do CEF e bindings pertencem à instalação privada do Codisseu.

## Comportamento

- Aba tipada Browser junto das abas de código/API, sem criar documento artificial
  ou mudar Save/Undo/LSP. Até8 páginas. Editor pode permanecer ao lado numa divisão.
- Abrir navegador pelo menu Exibir → Navegador integrado ou pelo ícone de globo
  imediatamente à esquerda do Build na barra superior (03/11/2026). Os dois
  acessos usam `open_browser_url("about:blank")`, focando o endereço, sem iniciar
  build/debug nem alterar a fonte. Botão nativo30×30; arte centrada na caixa16px
  como os demais ícones da barra, com traço1,2px. Tooltip/nome acessível
  "Abrir navegador integrado", disponível também sem arquivo/projeto aberto.
  Informar HTTP/HTTPS (localhost/devserver incluídos),
  editar URL, Enter, Back/Forward/Reload/Stop, estado carregando/erro e título real.
  Ctrl+L foca endereço; F5/Ctrl+R recarregam; Alt+Left/Right navegam; F12 abre
  DevTools; Ctrl+W fecha a aba ativa. Atalhos/texto do navegador não editam fonte.
- Página real com CSS/JS, fetch/formulários, rolagem, seleção, teclado, Unicode,
  foco e resize100%/125%. Contexto de inspeção abre DevTools da própria página.
  Apenas superfícies visíveis pintam; páginas ocultas são sinalizadas ao motor.
- Contexto web não tem API privilegiada para arquivos, shell, banco, credenciais
  ou comandos da IDE. Navegação externa fica restrita a HTTP/HTTPS/about:blank;
  file/javascript/data/custom protocols não são executados pelo campo de URL.
  Popups compatíveis podem virar outra aba dentro do limite; não abrir aplicativo
  externo nem aceitar downloads/permissões de dispositivo silenciosamente.
- Cookies/storage pertencem a perfil efêmero privado da sessão do motor. Não
  importar sessão Chrome/VSCode/PhpStorm nem persistir senhas no workspace.
  Nenhuma página é carregada ao simplesmente restaurar o projeto.
- Fechar última aba/IDE recolhe motor e subprocessos próprios com prazo limitado.
  Crash mostra erro e permite recuperação explícita, sem reinício em loop.
  Recursos ausentes não impedem PHP/JS/TS/SQL/Rust/Python/editor de funcionar.

## Aceitação e entrega

Fixture HTTP própria: DOM/CSS/JS e contador42, input Unicode, links/histórico,
reload confirmado no servidor, erro, resize/foco/clipboard, abas/divisão, fechamento
e recuperação. DevTools real: inspeção DOM, Console42, fetch/Network e breakpoint
em Sources com variável conhecida/Continue. HTTPS público sem autenticação e
certificado inválido recusado. Verificar sandbox real nos processos, não somente
uma flag de configuração. Perfil e processos do navegador normal preservados.

GUI100%/125% normal/compacta, screenshots reais inspecionadas e testes dos atalhos
com fonte dirty preservada. Regressões do editor e das linguagens/painéis comuns.
Release/launcher e pacote portátil incluem runtime/helpers/notices/manifests; mover
e extrair pacote não pode depender do checkout/IDE comercial. Documentar limites
de plataforma, accessibility/IME, codecs/DRM, permissões e protocolos de debug
que não forem efetivamente exercitados. Sources/DevTools não equivalem sozinhos
a integração de breakpoints do editor por DAP; registrar distintamente esse estado.

## Biblioteca de navegação —03/11/2026

Favoritos por grupo/projeto e acessos no cofre do SO complementam o perfil CEF
efêmero. Nada é importado de outros navegadores ou salvo no workspace. Leitura/
preenchimento explícitos, contexto isolado por documento e origem exata; contrato
browser-library-contract.md e instruções browser-usage.md. Modais nativas com
margem20, campos uniformes e alinhados, cartões e corpo/status/rodapé estáveis.
Refinamento solicitado pelo usuário em screenshot desta implementação; não é
nova medição de IDE externa. Persistência não restaura páginas nem scripts.
