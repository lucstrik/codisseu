# Como contribuir

O Codisseu está em alpha, e o objetivo é ter um ambiente de desenvolvimento
integrado que funcione bem de verdade. Pode ajudar com código, testes, correções
na documentação ou contando o que não funcionou no seu Linux.

## Encontrou um problema?

Abra uma [issue](https://github.com/lucstrik/codisseu/issues/new/choose) e informe:

- A versão do Codisseu e como instalou.
- Sua distribuição Linux, arquitetura e se usa X11 ou Wayland.
- Os passos para reproduzir, o que esperava e o que aconteceu.
- Um projeto pequeno ou trecho de código que demonstre o problema, quando possível.

Remova tokens, senhas, dados de clientes e caminhos privados dos exemplos e logs.
Se for uma falha de segurança, use o canal descrito em [SECURITY.md](SECURITY.md).

## Vai mexer no código?

Leia o [guia de desenvolvimento](docs/development.md), o [AGENTS.md](AGENTS.md)
e o [plano do projeto](STORM_PLAN.md). Antes de uma mudança grande, abra uma
issue explicando o problema e a solução que tem em mente. Para um ajuste pequeno,
pode enviar o pull request direto.

Algumas decisões guiam o projeto:

- A interface e o núcleo são Rust + egui/eframe, com um único editor compartilhado.
- Tarefas de disco, linguagem, banco e processos não devem bloquear a interface.
- Edições e operações de banco precisam preservar os dados e explicar falhas.
- Dependências, ícones, fontes e código adaptado precisam ter origem e licença.
- Uma funcionalidade só deve ser anunciada como validada com um cenário real.

Mantenha a mudança focada. Teste o comportamento alterado e os casos de erro que
fazem sentido. Se mudar a interface, confira também o resultado visual e a
navegação por teclado. Use fixtures descartáveis, sem bancos ou projetos pessoais.

No pull request, explique o problema, o resultado e quais verificações executou.
Se algo ficou sem testar, escreva isso. Confira a licença vigente do repositório
antes de contribuir e preserve os avisos dos componentes de terceiros.

O Codisseu tem implementação e identidade próprias. Não envie código proprietário,
plugins comerciais ou ativos sem autorização de redistribuição.
