# Fixture própria Vitest/Jest

Código criado para o corpus Codisseu. Dependências fixadas em package.json/lock:
Vitest5.0.5, Jest30.5.2 e TypeScript5.9.3. Instalar apenas em cópia descartável,
com `npm ci --ignore-scripts`, nunca como efeito de abrir a IDE.

Vitest:5casos TS (3passam,1falha,1ignorado). Jest:6casos JS/TS
(4passam,1falha,1ignorado). Há datasets, Unicode e metacaracteres de regex.
O transform TypeScript próprio usa transpileModule para provar integração real;
não faz parte do runtime da IDE nem substitui a configuração de outros projetos.
`CODISSEU_JS_TEST_FIXTURE` aponta à cópia com node_modules para testes opt-in.
