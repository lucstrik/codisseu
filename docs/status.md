# Codisseu 0.1.0-alpha.2 — desenhos no projeto

Editor nativo Rust/egui de arquivos `.excalidraw` e `.excalidraw.json`, integrado
às abas e ao histórico do editor. Abra **Desenhos · Excalidraw** no rail esquerdo
para criar arquivos; arquivos abertos pela árvore também entram no catálogo.
Reabrir a pasta ou o `.code-workspace` restaura os desenhos registrados
existentes, mesmo após fechar suas abas. Projetos têm catálogos separados.

Formas, setas, linhas, desenho livre e texto Unicode; seleção/movimento/resize,
estilos, duplicar/excluir, camadas, pan/zoom, enquadramento, Undo/Redo, salvar,
fonte JSON e copiar SVG. Não há runtime web nem download para desenhar.
[Guia](excalidraw-usage.md) e [contrato](excalidraw-contract.md).

O renderer é próprio e compatível com JSON v2; não incorpora a extensão VS Code
nem o React Excalidraw. Imagens e tipos desconhecidos são preservados com
indicação na interface; roughness, hachuras, fontes/bindings avançados,
colaboração e importação de fonte embutida PNG/SVG não têm paridade integral.
SVG recusa objetos visíveis não suportados.

Validação local desta implementação: 32 testes Rust de desenho/catálogo/core/UI,
13 regressões de workspace/HTTP/runtimes e 14 testes Python de empacotamento e
exportação aprovados. Clippy e formatação aprovados. O corpus XTest de debug
passou 24 verificações em oito janelas X11, escalas 100/125%, com Save/Undo/Redo
e reabertura A/B/A por perfil comum e sessões distintas. Isso não comprova
Wayland, outros sistemas ou equivalência integral ao Excalidraw. A release
publica o instalador com checksum e metadados; o CI obrigatório também reconstrói
e verifica o pacote.

Requisitos da distribuição core e ferramentas opcionais permanecem descritos
no [guia Linux](install-linux.md). A edição de desenhos funciona offline e sem
Node.js, PHP, servidor de linguagem ou conta Excalidraw.


---

# Capturas de produto sem desktop — 17/11/2026

Nova apresentação solicitada pelo autor: imagens somente do Codisseu, sem o
painel do sistema operacional. Geradas duas capturas reais pelo próprio
renderer da alpha pública v0.1.0-alpha.1 core (`--capture`), ELF SHA
cc28c8f87daf7654a3c8844a3cb75a3286dbe4d431c5819c0506a7ed84e0e9fd.
PNG1920×1200/escala1.5, sem recorte, recompressão ou edição de pixels.

Projeto codisseu-demo e perfil separados, em display Xvfb autenticado. Capturas
PHP e consulta SQLite com seis linhas reais de dados sintéticos; dois processos
encerraram0; display, cookie e diretório XDG_RUNTIME temporários foram limpos.
O perfil de demonstração permanece somente nas evidências locais.
Instalação/perfil normais preservados; não houve rebuild ou alteração de código
nativo. Verificação de SHA/PNG/dimensões/metadados/resultado SQL aprovada.

Arquivos: docs/images/{codisseu-workspace,codisseu-database}.png, procedência
em docs/images/README.md, README principal, website/assets/codisseu-workspace.png,
website/index.html (alt, legenda, dimensões e metadados sociais/JSON-LD),
website/README.md e docs/website-contract.md/status.md. JPGs antigos permanecem
como histórico; deixaram de ser exibidos no README/site. Fonte e pixels das
novas capturas são rastreáveis. Publicação segue PR, CI e Pages.

# Página pública no GitHub Pages — 17/11/2026

M7, apresentação do projeto. Página única em `website/`, com HTML/CSS/JavaScript
sem build ou backend, no endereço configurado https://lucstrik.github.io/codisseu/.
Identidade escura azul/violeta do logo, fundo quadriculado com deformação no
cursor, hover, imagem real do editor, recursos, contribuição, download e FAQ.
Texto informa alpha core, Linux amd64/glibc2.39+, ferramentas opcionais e limites;
foto anterior identificada, sem promessas de compatibilidade/performance novas.

Canvas decorativo com fallback CSS; navegação/conteúdo sem JS, movimento reduzido
e ponteiro coarse estáticos, pausa em repouso/aba oculta, DPR1.5 e teto30fps.
Fonte Inter convertida para subset WOFF2 latino174.308bytes (TTF879.708bytes),
licença OFL preservada; logo/foto sem alterações e ícone Lucide com licença ISC.
SEO: title/description, canonical, OpenGraph/Twitter, JSON-LD SoftwareApplication,
sitemap e robots auxiliar do subdiretório. Nenhuma garantia de indexação/posição.

Arquivos: website/**, .github/workflows/pages.yml, README.md e
 docs/website-contract.md/status.md. Workflow estático com Actions oficiais
fixadas por SHA, permissões por job, artifact somente website/, environment
restrito à main. Pages habilitado via API com build_type=workflow e HTTPS.
Proteções de main/tag, CI Rust e instalador da alpha preservados.

Validação local: node --check, git diff --check, Actionlint1.7.12 em contêiner,
HTML/JSON-LD/XML/links e recursos HTTP200 sob /codisseu/,12checks isolados do
canvas aprovados. Navegador real: grade reage ao ponteiro, foco/skiplink visível,
FAQ expande, layout desktop e telas320/390/768px inspecionados. Overflow de
órbitas decorativas corrigido. Publicação usa o fluxo normal PR→CI→merge→Pages;
a verificação do endereço público ocorre no deploy. Esses checks são do site,
sem testar novamente a interface nativa ou fechar M7 global.

# Codisseu 0.1.0-alpha.1 — Linux

Primeira distribuição pública do núcleo da IDE, sob licença MIT.
O instalador é para Linux x86_64/amd64 com glibc2.39 ou mais recente.
Leia o [guia de instalação](install-linux.md) e as
[ferramentas opcionais](optional-tools.md) antes de configurar providers.

## O que acompanha este pacote

Editor único de PHP, JS/TS, HTML, CSS, JSON, SQL, Rust e Python, arquivos e abas,
busca, terminal, integração Git, cliente HTTP e ferramentas de banco, incluindo
SQLite incorporado. Integrações que executam ferramentas externas dependem de
configuração e do motor correspondente. O pacote core não inclui runtimes,
adaptadores, formatadores privados ou o navegador CEF.

O editor permanece utilizável sem PHP/Node/Rust/Python. Providers explícitos
podem ser configurados por projeto, com confiança antes de executar. A falha ou
ausência desses processos não deve interromper a edição básica ou SQLite.

## Verificações desta entrega

- 27 testes Rust distintos do corpus de runtime/provider aprovados: manifesto
  mínimo, prioridade de configuração explícita, falhas dos4providers,
  edição/undo/redo/save e consulta SQLite durante falhas e timeout de LSP.
- Diagnósticos reais PHP e TypeScript com emoji, correção e encerramento também
  passaram em perfil temporário. Repetição com PATH vazio e manifesto core
  vazio passou; repetições não foram somadas à contagem de testes.
- Clippy dos módulos alterados e formatação aprovados; build release concluído.
  Compilação não substitui os cenários funcionais.
- 4 testes do empacotador e 10 testes do exportador de código aprovados.
- 1.075 hashes conferidos após extrair o `.deb`. Instalação via `apt` em Debian13
  descartável aprovada; CLI e leitura de PHP UTF-8 passaram com rede desligada,
  sem PHP/Node/npm/Composer/Rust/Python/Git/Docker instalados no contêiner.
- 4 janelas nativas X11 do aplicativo extraído, em100/125% e com PATH vazio/com
  ferramentas, passaram16 verificações de abertura, edição, gravação exata e
  encerramento. Todas saíram com código0. Perfis/projetos/displays eram próprios
  e foram removidos. A ausência de provider não disparou instalação ou confiança.

O ZIP foi atualizado de8.6.0 para9.0.5 após a revisão dos alertas do GitHub.
O leitor de XLSX valida explicitamente a consistência dos cabeçalhos locais e
centrais;19 testes de transferência/XLSX passaram, incluindo o novo arquivo
malformado. Check, Clippy e formatação passaram. Instalação offline e as quatro
janelas foram repetidas no `.deb` reconstruído; o candidato anterior não foi
distribuído. As fixtures JS/PHP também tiveram seus locks corrigidos; npm audit
e Composer audit dos locks ficaram sem alertas naquela execução. Os frameworks
JS preservaram suas contagens esperadas, incluindo falhas intencionais.

Pacote: `codisseu_0.1.0-alpha.1_amd64.deb`,53.868.940bytes.
SHA-256: `4bad393912512311fb0e77fc0892393785b2d62c5a5ee6b2caae322c623b548f`.
Os avisos acompanham o instalador; a coleta contém539pacotes Cargo normal/build,
com os componentes próprios identificados separadamente, sem textos faltantes.
As três fotos do README são imagens anteriores fornecidas e autorizadas pelo
autor; as oito capturas dos testes desta entrega não são essas fotos.

## Limites conhecidos

Alpha Linux com foco em Debian13/X11; Wayland, outras GPUs/distribuições,
Windows/macOS e ARM ainda precisam de validação própria. Ubuntu22.04 não atende
ao requisito de glibc deste binário. Não há recuperação integral de texto não
salvo após um crash. Recursos sem motores privados permanecem indisponíveis;
instalar PHP/Node por si só não habilita todas as integrações.

Próximo trabalho: ampliar o corpus Wayland e compatibilidade Linux, preparar a
distribuição auditada dos motores opcionais e avançar a recuperação após falhas.
