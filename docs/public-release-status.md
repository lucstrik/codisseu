# Codisseu 0.1.0-alpha.2 — desenhos no projeto

Editor nativo Rust/egui de arquivos `.excalidraw` e `.excalidraw.json`, integrado
às abas e ao histórico do editor. Abra **Desenhos · Excalidraw** no rail esquerdo
para criar arquivos; arquivos abertos pela árvore também entram no catálogo.
Reabrir a pasta ou o `.code-workspace` restaura os desenhos registrados
existentes, mesmo após fechar suas abas. Projetos têm catálogos separados.

Canvas com barra flutuante de ícones oficiais, propriedades contextuais,
controles inferiores e aparência Sistema/Claro/Escuro. Sistema acompanha o
Codisseu, escuro por padrão; cores originais do arquivo são preservadas.

Formas, setas, linhas, desenho livre e texto Unicode; seleção/movimento/resize,
estilos, duplicar/excluir, camadas, pan/zoom, enquadramento, Undo/Redo, salvar,
fonte JSON e copiar SVG. Cadeado mantém a ferramenta ativa; borracha e texto
inline participam do histórico. Save/Close aguardam o texto desse frame e
comandos de abas ocultas não ficam latentes. Não há runtime web nem download
para desenhar.
[Guia](excalidraw-usage.md) e [contrato](excalidraw-contract.md).

O renderer é próprio e compatível com JSON v2; não incorpora a extensão VS Code
nem o React Excalidraw. Imagens e tipos desconhecidos são preservados com
indicação na interface; roughness, hachuras, fontes/bindings avançados,
colaboração e importação de fonte embutida PNG/SVG não têm paridade integral.
SVG recusa objetos visíveis não suportados. O rail usa o símbolo oficial
Excalidraw, MIT, monocromático e sem fundo, nas proporções comuns do Codisseu.

Validação local desta implementação: 43 testes Rust de desenho/catálogo/core/UI,
13 regressões de workspace/HTTP/runtimes e 14 testes Python de empacotamento e
exportação aprovados. Clippy e formatação aprovados. O corpus XTest de release
passou 24 verificações em oito janelas X11, escalas 100/125%, com Save/Undo/Redo
e reabertura A/B/A por perfil comum e sessões distintas. Isso não comprova
Wayland, outros sistemas ou equivalência integral ao Excalidraw. A release
publica o instalador com checksum e metadados; o CI obrigatório também reconstrói
e verifica o pacote.

Requisitos da distribuição core e ferramentas opcionais permanecem descritos
no [guia Linux](install-linux.md). A edição de desenhos funciona offline e sem
Node.js, PHP, servidor de linguagem ou conta Excalidraw.

---

## Histórico da primeira alpha

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
