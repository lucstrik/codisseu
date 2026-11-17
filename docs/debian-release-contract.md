# Pacote Debian e publicação — 16/11/2026

O pacote Linux x86_64 é uma versão alpha para distribuições que atendam às
dependências calculadas pelo executável (glibc2.39 no build desta entrega).
A variante pública `core` transporta o aplicativo em `/opt/codisseu`, com comando em
`/usr/bin/codisseu`, entrada desktop e ícones do projeto. Não executa instaladores
de ferramentas, scripts de projetos, downloads ou alterações de PATH global.
Não inclui PHP/Node/Rust/Python/CEF nem bibliotecas privadas do SO. Providers
vazios são um manifesto válido e autoritativo; não devem ativar fallback para
`.tools` do computador de build. Ferramentas explícitas permanecem configuráveis.

Antes de empacotar, conferir integralmente `MANIFEST.sha256`, caminhos relativos,
links confinados, ausência de arquivos extras, metadados do binário e recursos
necessários. Uma árvore incompleta ou modificada deve falhar antes de produzir
o `.deb`. O controle Debian deriva os requisitos ELF usando `dpkg-shlibdeps` e
acrescenta as bibliotecas gráficas carregadas por `dlopen`. Linguagens, Git e
Docker não são dependências obrigatórias para abrir e editar arquivos.

O pacote inclui checksum SHA256 externo, metadados de versões/fontes e avisos de
terceiros. A licença/corresponding source necessária à distribuição pública deve
ser fornecida junto à release; a existência do `.deb` não fecha essa auditoria.
O bundle completo antigo tem obrigações separadas e não é o artefato público
desta entrega.
Novas versões são explícitas, sem atualizador/download automático no primeiro uso.

Validação: testes do empacotador, inspeção/extracão do `.deb`, hashes depois da
extração, CLI e editor sem motores configurados. Teste gráfico usa
display/perfil/fixture próprios e não altera a
instalação pessoal. Não anunciar suporte a Wayland, todas as GPUs, outros Linux
ou arquiteturas sem cenários próprios. Compilar não substitui teste funcional.

```sh
cargo build -p codisseu --release --locked --jobs 2 --target-dir target/portable-release
python3 tools/test_deb_package.py -v
python3 tools/build_deb.py \
  --core-binary target/portable-release/release/codisseu \
  --notices DIRETORIO_DE_AVISOS_AUDITADOS \
  --version '0.1.0~alpha.1' \
  --maintainer 'Lucas Torres <18267941+lucstrik@users.noreply.github.com>'
```

`--bundle` continua disponível para empacotar uma árvore completa já preparada;
esse modo não dispensa as obrigações de redistribuição dos motores que ela contém.
