# Codisseu — instruções para o Codex

## Missão e leitura obrigatória

Construir **Codisseu**, um único aplicativo desktop de desenvolvimento PHP, JavaScript/TypeScript e bancos de dados, em **Rust + egui/eframe**, priorizando Linux e preservando uma arquitetura multiplataforma. As referências funcionais são PhpStorm, WebStorm e DataGrip. O objetivo inclui alta fidelidade de organização visual, densidade, cores, atalhos e fluxos, com implementação própria e identidade Codisseu.

Antes de alterar código, leia `STORM_PLAN.md` integralmente, em blocos se necessário. Na primeira sessão, leia também os arquivos de instruções já existentes no repositório. Nas sessões seguintes, leia `docs/status.md`, a etapa vigente em `docs/milestones.md`, os contratos dos módulos afetados e as evidências pertinentes. Não substitua este projeto por Electron, Tauri, uma página web ou três aplicativos separados.

Este arquivo é intencionalmente curto. O plano completo está separado para não depender do limite de leitura automática das instruções do Codex. Se o plano estiver ausente, registre essa limitação; não invente seu conteúdo.

## Decisões de produto

- Texto sempre deve ter respiro nas margens de cima, baixo e laterais do componente; nunca colado às bordas. Preservar densidade e alinhamento com padding consistente, inclusive em código, cabeçalhos, botões e tooltips. Preferência explícita do usuário em04/11/2026.

- Não construir três motores de editor. O mesmo editor abre PHP, JS, TS, HTML, CSS, JSON e SQL; provedores acrescentam recursos por linguagem.
- Ícones devem seguir a referência com a maior fidelidade possível: reutilizar o ativo idêntico quando a licença permitir; nos demais casos, usar a alternativa visual mais próxima com procedência registrada. Para AI Chat, usar o símbolo enviado pelo usuário em25/10/2026, monocromático e sem fundo.
- GUI e núcleo de produto em Rust. Servidores de linguagem, depuradores e ferramentas como PHP, Node.js e Git podem ser processos externos, declarados e gerenciados. Isso não autoriza interface web nem dependência do IDE proprietário para o Codisseu funcionar.

A ideia e uma unica IDE funcionar de maneira que o desenvolvedor va ter tudo em um unico software funcionando com muita fluidez e muito bem otimizado para aguentar bem muitos dados ou projetos gigantescos.

## Fontes, evidências

Siga a seção de fontes e permissões do plano.

O usuário concedeu autorização expressa em 20/10/2026 para inspecionar e automatizar as instalações locais de referência, comparar suas interfaces e comportamentos e tomar decisões de implementação do Codisseu sem pedir confirmação repetida. Use a melhor versão efetivamente instalada de PhpStorm, WebStorm e DataGrip, identificando build, tema e escala antes da comparação. Registre essa autorização e o escopo em `reference/permissions.md`; não trate instalações ausentes como observadas. Preserve a instalação normal e use fixture/perfil isolados para testes. Continue a auditoria de procedência de cada código ou ativo incorporado.

Para cada conclusão sobre o original, informar origem, versão/build, cenário e confiança: `observed`, `documented`, `inferred` ou `unknown`. Não inventar screenshots, valores HEX, posições, atalhos, compatibilidade de plugins, testes aprovados ou percentuais. Uma medição local é diferente de uma página de marketing. Uma inferência deve permanecer identificada como tal.

## Como trabalhar em cada sessão

Escreva ou atualize o cenário e o contrato, implemente, execute testes, inspecione a interface quando houver alteração visual e registre os resultados. Não replaneje todo o produto a cada sessão. Não interrompa progresso independente por falta de uma ferramenta opcional.

Use os comandos de validação realmente implementados no repositório. O plano define contratos desejados de `cargo xtask`, mas eles não existem até serem criados e testados. Antes disso, use os comandos Cargo convencionais apropriados à estrutura presente.

Ao finalizar, atualizar `docs/status.md` com: arquivos alterados, funcionalidades que de fato funcionam, testes/comandos executados e seus resultados, screenshots reais quando disponíveis, falhas/bloqueios e próxima tarefa concreta. Resultados de compilação não são testes funcionais. Não escrever “100% igual” ou “pronto” sem corpus e critérios correspondentes.

