# Configurar as ferramentas do seu projeto

O primeiro `.deb` público leva o editor e o núcleo do Codisseu. PHP, Node,
servidores de linguagem, formatadores e adaptadores de debug são opcionais e
não são instalados junto com ele. Você pode começar editando arquivos e preparar
as outras ferramentas conforme precisar.

## O que é cada coisa?

O **interpretador ou runtime** executa sua aplicação: PHP, Node, Python ou a
toolchain Rust, por exemplo. A versão e as extensões precisam atender ao projeto.

O **servidor de linguagem**, também chamado de provider, analisa o código e
oferece sugestões, diagnósticos e navegação. Ter PHP instalado não instala o
Phpactor automaticamente; ter Node instalado não instala o servidor TypeScript.

O **adaptador de debug** liga o depurador do Codisseu ao runtime. PHP também
precisa de uma instalação compatível do Xdebug para as sessões que o utilizam.
Os adaptadores e os motores privados ainda não fazem parte do primeiro `.deb`.

O Codisseu não prepara as dependências da aplicação ao abrir uma pasta.
Composer, npm e as ferramentas de teste continuam sendo escolhas do projeto.

## PHP e JavaScript/TypeScript: configurar a análise

Os providers integrados nesses fluxos são **Phpactor** e
**typescript-language-server com TypeScript**. Prepare essas ferramentas e os
runtimes compatíveis separadamente. Use os caminhos da sua instalação.

Crie um arquivo JSON, por exemplo `providers.json`, fora das pastas públicas
do projeto. O formato aceito é uma lista como esta:

```json
[
  {
    "language": "Php",
    "program": "/usr/bin/php",
    "args": ["/caminho/do/phpactor/bin/phpactor", "language-server"]
  },
  {
    "language": "TypeScript",
    "program": "/usr/bin/node",
    "args": [
      "/caminho/do/typescript-language-server/lib/cli.mjs",
      "--stdio",
      "--tsserver-path",
      "/caminho/do/typescript/lib/tsserver.js"
    ]
  }
]
```

Os caminhos são exemplos. Troque todos eles pelos arquivos reais da sua
instalação; o seu PHP ou Node também pode estar em outro lugar. Os argumentos
são itens separados, sem montar uma linha de shell. Se você só precisa de uma
linguagem, mantenha apenas o item dela na lista. JavaScript, JSX, TypeScript e
TSX usam a entrada `TypeScript`.

No Codisseu:

1. Abra o projeto e **Configurar providers** na barra de ferramentas.
2. Informe o caminho do JSON no campo **Arquivo de configuração**.
3. Use **Carregar somente para este projeto**, se quiser limitar essa escolha
   à pasta aberta, ou **Carregar configuração** para a configuração geral.
4. Confira os executáveis apresentados e use **Iniciar providers**.

Iniciar autoriza esses executáveis para essa pasta. A autorização fica no
perfil do Codisseu; uma mudança de configuração exige nova autorização. Um
projeto confiável pode iniciar os providers automaticamente nas próximas
aberturas, conforme a opção de inicialização automática. **Parar providers**
interrompe a análise; o editor continua aberto.

Também é possível carregar o JSON pelo terminal:

```sh
codisseu --workspace /caminho/do/projeto --providers /caminho/providers.json
```

Carregar a configuração não instala as ferramentas. Se houver erro de caminho,
permissão ou inicialização, consulte o detalhe no diálogo e no painel de
problemas. Corrija a configuração e inicie de novo. Um provider parado ou com
falha não impede a edição básica.

## Executar, testar e depurar

Você pode executar os comandos do seu projeto no terminal integrado, usando
as ferramentas que já instalou. Use a versão adequada, inclusive quando o projeto
roda em um venv ou container. Salve os arquivos para executar a versão do disco.

O `.deb` core não descobre os motores privados a partir do PHP ou Node do sistema.
Os fluxos de debug, scripts JavaScript do cliente HTTP, formatação PHP e ferramentas
Python privadas precisam do pacote de motores correspondente, ainda não publicado.
Os guias abaixo descrevem a implementação e a preparação de desenvolvimento;
não significam que esses motores já acompanham a alpha pública.

- [Testes PHP com PHPUnit/Pest](php-tests-usage.md)
- [Testes JavaScript/TypeScript](js-tests-usage.md)
- [Rust e Cargo](rust-usage.md)
- [Python, venv e testes](python-usage.md)
- [Depuração e adaptadores](debugging.md)

Alguns guias também registram o pacote portátil completo usado no desenvolvimento,
com ferramentas privadas. Esse pacote é diferente do primeiro `.deb` público.
Ter o painel de um recurso na IDE não significa que o motor externo dele já está
instalado. O Chromium/CEF do navegador integrado também não acompanha esse `.deb`.

## Ver o que a instalação fornece

```sh
codisseu --version
codisseu --runtime-info
```

`--runtime-info` descreve os recursos gerenciados disponíveis nessa instalação,
sem abrir a janela. Ele não é um inventário de todas as ferramentas do seu Linux.
Se você configurou um programa manualmente, confira também seu caminho e versão
na configuração do recurso.

[Voltar à instalação](install-linux.md) · [Voltar ao README](../README.md)
