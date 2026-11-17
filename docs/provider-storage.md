# Armazenamento do provider PHP

Contrato/cenário de 24/10/2026: abrir projetos PHP/Laravel repetidamente e
encerrar um provider sem acumular cópias descompactadas do PHAR. Preservar
autocomplete, navegação, vendor e licenças; não instalar outro servidor nem
executar Composer, hooks ou código da aplicação.

## Diagnóstico observado

O runtime original é Phpactor **2026.10.07.0**, PHAR oficial de **4.815.663 bytes**,
SHA-256 `b28bf6166e016d539fc2f2aec18ef918b12590e436a6f1ccbdf30b5bbdbdb16b`.
Seu tamanho não corresponde aos 69 GB relatados pelo usuário. Não foi possível
reconstituir retrospectivamente essa medida.

Encontrados nesta rodada: **2.798 temporários**, **11.363.808.042 bytes**, em
`.tools/phpactor-temp-quarantine-20261023`. O inventário da rodada anterior
registrava sua transferência reversível de /tmp, sem exclusão. Todos os nomes,
tamanhos e cabeçalhos foram conferidos novamente; todos começam pelo namespace
`HumbugBox440\KevinGH\RequirementChecker`. Não são o índice semântico nem os
arquivos de instalação. Também havia 129 arquivos /tmp/php* somando
510.715.365 bytes; não foram removidos por esta investigação.

A amostra controlada iniciou e interrompeu quatro processos por formato com
SIGKILL em diretórios temporários próprios, PHP local 8.4.27 e Xdebug desligado:

| Distribuição | Temporários remanescentes | Bytes remanescentes |
|---|---:|---:|
| PHAR original | 8 | 17.639.944 |
| Mesmo conteúdo extraído uma vez | 4 vazios | 0 |
| Launcher compatível no caminho antigo | 4 vazios | 0 |

É evidência de reprodução local após interrupção forçada; não identifica um
defeito específico upstream nem mede todo tipo de encerramento ou projeto.
Um `--version` normal do PHAR saiu0 e não deixou temporários na amostra própria.
O prefixo RequirementChecker identifica o primeiro conteúdo do arquivo, sem
provar que o checker implementou a criação desse temporário.

## Implementação

`tools/install_providers.py` valida o mesmo download e extrai seu conteúdo uma
única vez para `.tools/phpactor-2026.10.07.0/`. A configuração gerenciada executa
PHP sobre `bin/phpactor`, evitando o acesso repetido aos arquivos comprimidos
do PHAR. Nenhuma fonte upstream foi alterada; vendor, arquivos ocultos e
notices são preservados. O diretório extraído mediu **21.089.787 bytes** nesta
instalação, incluindo o manifesto próprio de hashes por arquivo.

A extração usa staging privado, valida entrypoint/autoload/licença e publica
por rename. Falha remove apenas o staging próprio. Um diretório estrangeiro
não é substituído; uma instalação gerenciada existente é reutilizada sem
reescrever arquivos potencialmente em uso. O PHAR verificado permanece no
cache de downloads para reinstalação offline. O caminho `.tools/phpactor.phar`
agora contém um **launcher PHP próprio de235 bytes**, que carrega o mesmo runtime
extraído. Deve ser mantido: perfis/sessões antigos podem continuar apontando a
esse nome. Apesar da extensão, esse arquivo não é mais um PHAR comprimido.

A auditoria do integrador encontrou três configurações normais persistidas
apontando ao caminho antigo. O launcher evita editar sessões, configurações ou
aprovações do usuário, mantém seus argumentos e impede que continuem usando a
distribuição comprimida. O `providers.json` novo continua apontando diretamente
a `bin/phpactor`; nenhum perfil normal foi regravado. O launcher e o entrypoint
direto responderam **Phpactor2026.10.07.0** ao `--version` real.

A migração só aceita arquivo regular com SHA-256 do PHAR fixado ou um launcher
próprio com marcador e corpo exatos. Arquivo estrangeiro, symlink e hardlink são
recusados. No Linux, `renameat2(RENAME_EXCHANGE)` troca nomes atomicamente e
mantém o inode anterior até uma nova conferência. Se outro processo colocou um
arquivo estrangeiro ou symlink durante a migração, os nomes são restaurados sem
apagar esse item. Se um terceiro escritor impede a reversão, ambos os itens são
preservados e o erro informa o caminho de recuperação. Sem suporte à operação
atômica, a substituição é recusada e o arquivo antigo permanece intacto.

O perfil gerenciado mantém as correções de atualização/reflexão anteriores.
O índice agora exclui `.tools`, `target`, `artifacts`, `node_modules`,
`storage/framework` e `bootstrap/cache`, preservando exclusões upstream de
testes/vendor/composer. Dependências PHP normais em `vendor` continuam
disponíveis. Providers personalizados não recebem flags injetadas.

Índices reais continuam dependendo do conteúdo e quantidade de projetos. Esta
mudança não impõe cota absoluta ao cache semântico nem apaga configurações ou
índices de outros aplicativos. A configuração exata gerenciada muda e segue o
fluxo de confiança já existente da IDE.

## Validação e limpeza

```sh
python3 tools/test_provider_storage.py -v
python3 tools/probe_provider_storage.py
```

Doze testes de instalação passaram: quatro de extração e oito de compatibilidade,
incluindo arquivo estrangeiro/marcador falso, symlink, duas corridas de troca,
terceiro escritor, operação atômica ausente e idempotência. O probe acima é independente
dos testes semânticos reais e seus processos/temporários ficam em fixtures
privadas, removidas ao concluir.

O matcher real upstream `FileList::includeAndExclude` foi exercitado com o
perfil gerenciado: Model/Builder Eloquent, Query Builder, Request e App\Models
ficaram incluídos; as seis categorias novas de artefatos/caches ficaram
excluídas. Evidência `filter-probe.json`; essa verificação de filtros não
substitui o teste semântico Laravel da aplicação.

Artefatos desta investigação:
`artifacts/m7-provider-storage-20261024/forced-stop-probe.json` e
`quarantine-audit.json`. Migração local e probe dos três entrypoints estão em
`compatibility-install.json` e `compatibility-probe.json`; testes em
`installation-tests.log`. A auditoria não encontrou handles abertos nos processos
acessíveis. Seis daemons normais do usuário negaram acesso ao diretório fd
(systemd, sd-pam, ssh-agent, kwin, polkit e powerdevil); essa inspeção não é uma
prova universal de ausência de handles.

A quarentena é material gerado, antigo e conferido contra o manifesto anterior;
foi indicada ao integrador para limpeza autorizada. Este documento não declara
que ela já foi excluída nem soma espaço potencial como espaço efetivamente
liberado. O relatório final de armazenamento deve registrar bytes antes/depois.

## Fontes

- Fonte instalada e hash acima: `observed`, distribuição oficial fixada de
  [Phpactor 2026.10.07.0](https://github.com/phpactor/phpactor/releases/tag/2026.10.07.0).
- [Phar no PHP](https://www.php.net/manual/en/book.phar.php): `documented`,
  arquivos comprimidos, instalação em arquivo único e leitura via Phar.
- [Índice Phpactor](https://phpactor.readthedocs.io/en/master/reference/indexer.html)
  e [configuração](https://phpactor.readthedocs.io/en/master/reference/configuration.html):
  `documented`, cache por projeto e filtros; documentação master consultada em
  24/10/2026 não é tratada como prova de toda opção disponível na versão fixada.
- [Box Requirement Checker](https://box-project.github.io/box/requirement-checker/):
  `documented`, checker incluído no PHAR; não é atribuição causal do vazamento.
