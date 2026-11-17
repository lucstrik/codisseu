# APIs no Codisseu

Abra um projeto e clique no ícone APIs do rail esquerdo. Collections,
Environments, Specs, Mocks e History pertencem a esse projeto. Os dados ficam
no perfil do Codisseu, fora do código; fechar a ferramenta não apaga coleções.

Use + para criar Collection, Environment ou Request. O menu de contexto da árvore
permite criar pastas/subpastas, duplicar, renomear e excluir. Clique em uma request
para abri-la na faixa de abas junto dos arquivos PHP/JS/SQL. Ctrl+W fecha a aba;
Ctrl+S salva definições; Ctrl+Enter envia a request. Arquivos mantêm seus buffers.

Escolha método, URL e ambiente; preencha Params, Authorization, Headers ou Body.
Send envia; Cancel interrompe a espera. A divisão entre request e response pode
ser arrastada. Resposta mostra status, duração, tamanho, corpo, headers, resultados
dos testes e console. Corpos grandes são limitados com indicação de truncamento.
Body → binary aceita um caminho de arquivo. Em form-data, escolha Text ou File
por campo. Caminhos relativos usam a raiz do projeto; o worker lê arquivos
regulares somente no Send, com limite total de8MiB por envio. Importar cURL
ou coleção apenas armazena referências, sem ler esses arquivos.

O histórico da sessão permite voltar às requests. Cancelar uma chamada não desfaz
ações que o servidor já realizou.

Cole uma linha cURL no campo URL para converter a requisição. O botão cURL copia
um comando da request atual; como no envio, variáveis resolvidas podem incluir
credenciais. Esse clipboard é uma ação explícita. A importação não executa shell.
Use Importar para colar ou ler uma collection Postman v2.1, environment, cURL ou
spec OpenAPI3 JSON/YAML. Exportar gera JSON ou grava pelo diálogo. Exportação
normal de coleções/ambientes omite os valores marcados como secretos.

## Login e token automático

1. Crie e selecione um Environment, por exemplo Local.
2. Adicione `base_url` e use `{{base_url}}` nas URLs.
3. Na request de login, abra Scripts → Post-response e escreva:

```javascript
const data = pm.response.json();
pm.environment.set("token", data.data.access_token);
pm.test("Login OK", () => pm.response.to.have.status(200));
```

Ajuste `data.data.access_token` ao campo real da resposta; por exemplo,
`data.token` ou `data.access_token`.

4. Habilite “Executar scripts desta request e da collection” e clique Send.
5. Nas outras requests, Authorization → Bearer Token: `{{token}}`.

Scripts não executam ao abrir/importar um projeto. A habilitação vale para a
sessão atual do projeto e permite execução no Send ou Run collection. O token
capturado é secreto por padrão e fica apenas na sessão, evitando gravá-lo no
JSON do perfil. Ao reabrir o Codisseu, execute o login novamente. Use a coluna
Secret também para senhas e outras variáveis. Corpos, scripts e specs são fontes
escritas pelo usuário: literais nelas podem persistir; prefira variáveis secretas.

Scripts exigem Node.js22.13+ disponível no PATH, com suporte a `--permission`.
Sem Node, editar coleções e enviar HTTP sem scripts continuam disponíveis. O
Codisseu inicia um processo separado por fase, limita tempo/saída/heap e recolhe
o processo no cancelamento. No Linux também limita dados/CPU/core. O conjunto
`pm.*` implementado é um subconjunto: response.json/text/code/headers,
environment/collectionVariables get/set/unset, variables.get/replaceIn,
pm.test e asserções simples de pm.expect. Não carrega módulos npm nem oferece
pm.sendRequest ou toda a API Sandbox/Chai do Postman.

## Specs, mocks e coleções

Specs importa OpenAPI3 JSON/YAML e gera uma coleção de operações. Parâmetros,
exemplos de body e autenticação Bearer/Basic/API Key suportada são aproveitados.
Referências externas não são baixadas. OAuth2 e combinações de esquemas não
suportadas são recusados com erro, sem importar uma request sem autenticação.

Mocks permite editar rotas de exemplo e iniciar um servidor local. Start só
escuta em127.0.0.1 com porta livre; copie sua URL para uma request. Stop, troca
de projeto e encerramento recolhem o listener. Não é hospedagem cloud.

Run collection executa requests em ordem, incluindo pastas, sobre o ambiente
selecionado. Scripts do login podem alimentar chamadas seguintes. O progresso,
testes e resultados aparecem na coleção e em History. Cancel run interrompe as
próximas chamadas; erro de transporte/script interrompe a rodada. Uma falha de
asserção fica identificada no resultado.

## Cobertura

O contrato e os testes são descritos em api-client-contract.md e docs/status.md.
Este módulo atende aos fluxos REST de backend do pedido, sem certificar toda a
plataforma Postman. OAuth2 interativo, WebSocket/gRPC/MQTT, colaboração/cloud,
monitores, Vault, datasets/Flows, SDK completo de scripts, geração abrangente
de clientes e testes universais OpenAPI continuam fora da cobertura declarada.

## Pastas e organização de microserviços

O **+** da barra API Client usa o mesmo ícone/controle do Database. Escolha
Collection ou Pasta. Uma pasta é criada dentro da collection/pasta ativa;
quando uma request está ativa, usa sua pasta atual. Há também Nova pasta no
editor/menu da collection e Nova subpasta no editor/menu de cada pasta.

Clique no nome da pasta para editar nome, autorização e scripts; a seta recolhe
os filhos. Duplo clique na collection abre seu editor. Você pode montar, por
exemplo, `FATURAMENTO → 8.4 → DEV → requests`. Nova request na pasta cria ali o
item. O menu de contexto permite Duplicar (preserva a pasta), Mover para… e
Mover acima/abaixo; duplicar pasta copia toda a subárvore com novas identidades.
A movimentação recusa destinos ausentes, a própria pasta e seus descendentes.

Authorization = Inherit nas requests aproveita a pasta e depois a collection.
Scripts são executados da collection às pastas e à request; cada fonte tem
escopo lexical próprio. A habilitação de scripts permanece explícita.
**Executar pasta** usa somente suas requests e mantém o contexto herdado.

URL e Query Params agora editam a mesma query. Alterar uma linha atualiza a URL;
digitar/colar query na URL atualiza as linhas. Repetições e Unicode são mantidos;
linhas desabilitadas não são enviadas. Em `/customers/:id`, preencha Path Variables
(a seção pode exigir rolagem). Um valor com `/` é codificado como parte do
segmento, sem criar outro caminho. cURL usa essa mesma resolução.

## Variáveis e execução com dados

VARIÁVEIS DO PROJETO armazena valores comuns entre collections **desse projeto**.
Precedência: projeto/global → collection → environment → dados do runner → local.
Segredos continuam só na sessão. `pm.globals` manipula o escopo do projeto;
`pm.variables.set/get/unset/has/toObject/replaceIn` suporta valores locais, que
não são gravados nas definições. Eles duram um Send ou a execução inteira do
runner, incluindo as iterações, e são descartados no encerramento/cancelamento.

Em Opções de execução da collection/pasta, escolha iterações ou cole um array
JSON de objetos. Com dados preenchidos, cada objeto define uma iteração e a
quantidade de objetos determina o total de iterações. Exemplo:

```json
[{"customer_id":"1","base_url":"http://localhost:8001"},
 {"customer_id":"2","base_url":"http://localhost:8002"}]
```

Use `{{customer_id}}` ou `pm.iterationData.get("customer_id")`. Os dados são
temporários e não entram no perfil. Máximo256 envios no total; JSON até1MiB.
Sem CSV neste incremento. Uma sessão executa um Send/run por vez; projetos em
janelas separadas têm serviços próprios, sem certificação de gravação concorrente
por duas instâncias no **mesmo** perfil/projeto. Trocar raiz salva/carrega as
definições de cada projeto; abas/respostas continuam sendo da sessão atual.

Scripts também dispõem de `pm.info.requestId/requestName/iteration/iterationCount`,
`pm.execution.location`, headers `.add/upsert/remove/get/has/all/toObject/each`
e query `.add/upsert/remove/get`. Essas alterações são aplicadas apenas ao envio,
sem regravar a request original. `pm.request.body` é somente leitura.

```javascript
pm.request.headers.upsert({key: "X-Service", value: pm.variables.get("service")});
pm.test("Resposta OK", () => {
  pm.response.to.have.status(200);
  pm.expect(pm.response.json()).to.have.property("id");
});
```

`pm.execution.skipRequest()` no pre-request ignora o envio e os scripts seguintes
da request. `pm.execution.setNextRequest("Nome ou ID")` escolhe o próximo item
do runner; `null` encerra a iteração. Não escapa da pasta selecionada e não altera
a ordem ao usar Send. O limite256 também impede ciclos indefinidos.

Asserções síncronas incluem igualdade/deep equality, not, property, include,
tipos, empty/exist/ok, lengthOf, comparação numérica/within, match e oneOf.
Response aceita status/header/jsonBody e verificações JSON/2xx; não é Chai inteiro
nem JSON Schema. Valores dinâmicos `{{$timestamp}}`, `{{$guid}}`, `{{$randomUUID}}`
e `{{$randomInt}}` são para dados de teste, não credenciais criptográficas.
