# Fixture PHPUnit/Pest

Fontes de teste próprias para Codisseu. Falhas/erros/skip são intencionais.
PHPUnit12.5.33 e Pest4.7.8, PHP8.4+, dependências fixadas em composer.lock.
Vendor é infraestrutura de testes e não acompanha o produto.

Prepare uma cópia fora desta fixture e instale apenas nessa cópia:

```sh
mkdir -p .tools/php-tests
cp -r fixtures/php-tests/. .tools/php-tests/
composer install --working-dir=.tools/php-tests --no-plugins --no-scripts --no-interaction
CODISSEU_PHP_TEST_FIXTURE="$PWD/.tools/php-tests" cargo test -p codisseu-core --test php_tests --locked -- --include-ignored --test-threads=1
CODISSEU_PHP_TEST_FIXTURE="$PWD/.tools/php-tests" CODISSEU_PHP_TEST_CAPTURES=1 cargo test -p codisseu-ui --lib php_tests --locked -- --include-ignored --test-threads=1
```

PHPUnit usa phpunit.xml e tests/PhpUnit. Pest usa pest.xml e tests/Pest.
Cada suite entrega6casos:3aprovados,1falha,1erro e1ignorado. Datasets
incluem texto acentuado/emoji. Não há banco, rede ou projeto empresarial.
Instalar Composer/dependências é uma preparação explícita do laboratório;
o painel Codisseu não instala nada. Dependências mantêm licenças upstream
MIT/BSD nos arquivos vendor e no lock, sem importar seus códigos na IDE.
