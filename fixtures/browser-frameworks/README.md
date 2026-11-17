# Fixtures próprias React, Vue e Angular

Aplicações mínimas REAIS para validar as ferramentas gerais do navegador Codisseu.
React usa createRoot/useState; Vue usa createApp/ref/render; Angular usa JIT via
@angular/compiler, Component e bootstrapApplication zoneless. Não usa CLI Angular,
extensões, React Fiber privado, Vue internals ou APIs privadas Angular.

Pins completos e integridades transitivas: package.json + package-lock.json.
React19.3.1, Vue3.5.44, Angular22.2.4 e Vite8.3.5. Node privado24.22.0.
Angular usa JavaScript e compiler JIT; não precisa TypeScript/Angular CLI.

Instalação executada somente neste diretório, com HOME/npmrc/cache privados:

    npm install --package-lock-only --ignore-scripts --no-audit --no-fund
    npm ci --ignore-scripts --no-audit --no-fund

Use o Node/npm privado em `.tools/node-v24.22.0-linux-x64`; nunca exige instalação
global. Os módulos ficam em node_modules, ignorado pelo Git. O cache isolado desta
prova está em `.tools/framework-fixtures/npm-cache`.

Validação real (gera os próprios Vite/Xvfb e encerra ambos):

    cargo build --manifest-path tools/browser-sites-probe/Cargo.toml --bin frameworks --target-dir target/browser-sites --locked --offline --jobs 2
    python3 tools/browser_frameworks_smoke.py

O probe usa BrowserService + InspectorService públicos, CEF sandboxed e Xvfb
privado autenticado. Lê DOM/atributos/CSS, calcula o retângulo real do botão e
ENVIA MouseMove/MouseClick CEF (não chama element.click). Confirma contador42→43,
POST real /api/echo com19+23, resposta201/JSON42, corpo lazy pelo inspetor, leitura
e edição localStorage confirmadas na página, avaliação JavaScript42, OnPaint e
cleanup do PID real. Repete nos três frameworks. `window.fixtureFramework` e
`fixtureVersion` são marcadores próprios com versão importada do framework real.

O resultado NÃO certifica árvore própria de componentes/props/state/hooks,
profiler, SSR ou adapters específicos. Esses recursos têm critérios separados.
A tela da fixture explicita esse limite. O aviso não fatal do build esm-bundler
Vue sobre feature flags é preservado em vite.log; a renderização/reatividade real
foram verificadas.

Corpus: artifacts/browser-inspector-20261102/frameworks/results.json,
driver-result.json, probe.log, npm-install.log, build.log e3PNGs OnPaint reais.
