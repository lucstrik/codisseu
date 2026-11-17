# Laravel intelligence fixture

Synthetic App classes/templates authored for Codisseu. Framework dependencies remain in ignored vendor and are pinned in composer.lock (Laravel12.70.1 in this corpus). No application bootstrap, Artisan, database, .env or external services are required.

Prepare only this fixture:

```sh
composer install --working-dir=fixtures/laravel --no-plugins --no-scripts --no-interaction --prefer-dist --no-progress
CODISSEU_PROVIDER_CONFIG="$PWD/.tools/providers.json" CARGO_INCREMENTAL=0 cargo test -p codisseu-language --test laravel_real --locked --jobs 1 -- --ignored --nocapture --test-threads=1
```

The root composer.json deliberately includes a harmless post-autoload sentinel. The test requires that HOOK_EXECUTED never exists: installation/indexing must not run project hooks. Composer metadata is read as JSON; no Composer PHP autoloader is loaded by the Codisseu Eloquent adapter. The configured Phpactor retains its existing trusted-provider policy.

This corpus exercises real PSR-4 classes/methods, inherited Eloquent/Request methods, Route facade metadata, source-derived static Model builder completion and navigation. It does not certify Blade semantic analysis, dynamic runtime macros/custom builders, model database columns, container bindings, query chains of arbitrary length or all Laravel versions. Live templates are original finite examples, separate from semantic results.

M3.29 acrescenta config/weather.php, traduções PHP/JSON, layout Blade, CSS e
rota com prefixo de grupo. editor_probe.php é apenas destino de sincronização
de buffers de teste: nunca executar. O teste restaura o snapshot do controlador
antes de consultar seu método a partir desse arquivo separado. As respostas
reais do provider e os adaptadores estáticos são verificados separadamente.
