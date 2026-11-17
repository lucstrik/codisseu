# Instalar no Linux

A versão pública vem em um `.deb` para **Linux x86_64 (amd64)**. O Codisseu
está em alpha: dá para acompanhar, testar e contar o que precisa melhorar.
O ambiente validado nesta entrega fica registrado no relatório da release.

## Baixar e instalar

1. Abra as [releases](https://github.com/lucstrik/codisseu/releases).
2. Baixe `codisseu_0.1.0-alpha.1_amd64.deb` e o arquivo `.deb.sha256` ao lado.
3. Na pasta do download, confira e instale:

```sh
sha256sum -c codisseu_0.1.0-alpha.1_amd64.deb.sha256
sudo apt install ./codisseu_0.1.0-alpha.1_amd64.deb
```

Abra pelo menu de aplicativos ou pelo terminal:

```sh
codisseu .          # abre a pasta atual
codisseu           # retoma a última sessão
codisseu -n        # abre uma janela vazia
codisseu --help
```

O comando libera o terminal. Use `codisseu --wait .` se quiser aguardar a janela
fechar. Veja também o [guia do comando](cli-usage.md).

O pacote registra as bibliotecas do sistema de que precisa. O `apt` verifica
versões e instala essas bibliotecas pelos repositórios do seu Linux. O binário
desta entrega exige glibc 2.39 ou mais recente; distribuições antigas, incluindo
Ubuntu22.04, não atendem esse requisito. Não force a instalação ignorando
as dependências. Linux de outra arquitetura, Windows e macOS não têm pacote
validado nesta release.

## E se eu já tiver PHP, Node ou outras ferramentas?

Pode manter tudo como está. Este `.deb` instala o aplicativo e usa as bibliotecas
do sistema; ele não substitui PHP, Node, Rust, Python, Git nem suas configurações.
Também não roda Composer/npm, instala dependências do projeto ou faz downloads
no primeiro uso.

Você consegue abrir, editar e salvar arquivos sem instalar todas essas ferramentas.
Para rodar código, testes, depurar ou usar inteligência de uma linguagem, configure
as ferramentas necessárias no próprio Codisseu. A ausência ou falha de uma
ferramenta deve aparecer como diagnóstico daquele recurso, mantendo o editor
utilizável. Uma configuração explícita do projeto tem prioridade.

| O que você quer usar | O que precisa preparar |
| --- | --- |
| Editar arquivos e trabalhar com SQLite | Já vem no aplicativo |
| Inteligência PHP | PHP compatível e Phpactor, configurados em Providers |
| Inteligência JavaScript/TypeScript | Node e typescript-language-server/TypeScript |
| Executar uma aplicação PHP/Node | A versão e extensões exigidas pelo seu projeto |
| Rust e Python | Toolchain/interpretador e ferramentas escolhidos pelo projeto |
| Git e Docker | Os programas correspondentes instalados no sistema |
| IA | Um provedor e autenticação próprios, conforme o [guia](ai-chat-usage.md) |

O [guia de ferramentas opcionais](optional-tools.md) explica a configuração.
Não é preciso instalar outro IDE para abrir o Codisseu.
O navegador CEF e os motores privados do pacote portátil de desenvolvimento não
fazem parte deste `.deb` público. O navegador integrado não está disponível
nesta distribuição.

Na primeira abertura de um projeto, confirme a confiança antes de permitir
execução de ferramentas. O aplicativo não prepara nem executa sua aplicação
só porque você abriu uma pasta.

## Atualizar ou remover

Baixe o `.deb` da nova release, confira seu checksum e use novamente
`sudo apt install ./NOME_DO_PACOTE.deb`.

```sh
sudo apt remove codisseu
```

Preferências, projetos e credenciais pessoais não são incluídos no pacote.
A remoção do aplicativo preserva seus projetos e os dados do perfil do usuário.
Senhas ficam no serviço de credenciais do sistema quando esse recurso é usado.

## Para quem prepara uma release

O contrato e os comandos ficam em [debian-release-contract.md](debian-release-contract.md).
O empacotador deriva as dependências ELF do executável real com `dpkg-shlibdeps`
e acrescenta as bibliotecas gráficas carregadas dinamicamente. O pacote não tem
scripts de instalação/remoção nem transporta bibliotecas de sistema privadas.

O pacote portátil completo descrito em portable-runtime-report.md
foi uma entrega anterior para uso pessoal. Seus hashes, testes e limites são
históricos e não equivalem à validação deste `.deb` público.
