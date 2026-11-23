# Codisseu 0.1.0-alpha.4 — edição de células no resultado

Duplo clique abre a edição na própria célula de uma consulta simples em SQLite,
PostgreSQL, MySQL ou MariaDB. Enter aceita a proposta local e Escape cancela a
edição atual. O tipo da coluna determina os valores aceitos, incluindo números,
booleanos, datas, texto e NULL; tipos sem tratamento seguro ficam protegidos.
A edição exige uma tabela de origem comprovada, chave primária completa e
escrita habilitada. Consultas com joins ou identidade ambígua continuam somente
leitura. A identificação da linha usa as colunas originais mesmo com filtros ou
ordenação na apresentação.

As propostas ficam pendentes até envio explícito. **Commit** envia e confirma o
lote; **Submit** envia mantendo a transação aberta, e **Rollback** desfaz mudanças
enviadas ainda não confirmadas. **Revert** descarta propostas locais. O worker
valida tipos, usa comandos parametrizados, verifica conflitos e a quantidade de
linhas afetadas antes de confirmar. SQLite inicia somente leitura e oferece
**Habilitar edição**; as conexões remotas preservam sua opção **Read-only**.

O [contrato de edição](grid-edit-contract-20261123.md) descreve os limites e o
corpus de validação. A distribuição permanece alpha core para Linux amd64;
integrações opcionais e requisitos do sistema estão no [guia Linux](install-linux.md).
Aurora precisa de teste próprio na AWS. MongoDB, Redis e Memcached não recebem
este fluxo de UPDATE/transação SQL; não se anuncia cobertura de todos os drivers
JDBC ou paridade integral com DataGrip.

Validação local do recurso: 77 testes Rust de banco aprovados (42 de unidade,
17 de tipos, 11 de SQLite e 7 cenários remotos). PostgreSQL18.6, MySQL8.0.46 e
MariaDB11.4.10 foram exercitados em contêineres descartáveis. Duas variantes
Aurora usam apenas os protocolos em servidores locais; não validam AWS.
Oito testes da interface passaram, incluindo os três bancos remotos. Clippy dos
módulos DB/core/UI e 14 testes Python de pacote/exportação passaram.

O corpus nativo do aplicativo passou 18 verificações em duas janelas X11,
nas escalas 100% e 125%:
editar por duplo clique, manter proposta local, Commit da linha correta,
Submit/Rollback com leitura independente, rejeição de letras em BIGINT e NULL
numérico. São dados sintéticos em perfil/display próprios; isso não comprova
Wayland ou todos os tipos de todos os bancos. O CI obrigatório reconstrói o
instalador e roda os cenários declarados; o pacote público exige sua validação
própria antes da publicação.

---

## Histórico da alpha.3

# Codisseu 0.1.0-alpha.3 — tema global Noturno

Novo tema padrão escuro inspirado na referência visual do autor. Em
**Configurações › Aparência › Tema do aplicativo**, escolha Noturno ou Clássico.
A troca acompanha toda a IDE, incluindo o editor, abas, painéis, menus,
configurações e prévias; a preferência é restaurada nas próximas aberturas.

Sintaxe verde/laranja/azul, seleção azul e gutter Git com verde para adições,
laranja para modificações e vermelho para remoções. O cálculo usa mudanças
reais; Save e troca de tema preservam os arquivos e o estado Git. Cores autorais
dos desenhos e conteúdos ANSI explícitos são preservadas.
[Guia de aparência e temas](themes.md).

Validação local:794 testes Rust distintos aprovados,41 testes opt-in não
executados;24checks em6janelas X11/llvmpipe, escalas100/125%, com troca por
clique, reabertura, gravação exata e marcas Git reais. Formatação e Clippy dos
módulos afetados aprovados. Isso não comprova Wayland, outros SO ou todas as
combinações de integrações. O CI reconstrói e verifica o instalador core, com
checksum, metadados e avisos de terceiros. Requisitos e limites da distribuição
permanecem no [guia Linux](install-linux.md).

---

## Histórico da alpha.2

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
