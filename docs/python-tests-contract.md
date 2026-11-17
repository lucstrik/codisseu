# Python — pytest e unittest no Run compartilhado,02/11/2026

Contrato anterior à implementação. Service nativo independente da GUI reutiliza
Outcome/RunReport/Scope/State/TestCase e processo limitado/cancelável de php_tests.
Frameworks Auto/Pytest/Unittest; Auto seleciona pytest, cujo motor também reconhece
classes unittest. Configuração por raiz contém interpretador e timeout1..3600s;
seleção do ambiente usa python_project::resolve_interpreter, sem pip/install.

Inspect lê somente perfil/metadados. Executar suíte/arquivo/caso e repetir falhos/
última execução exigem ação explícita; nenhum teste é coletado ao abrir projeto.
Um helper próprio com hashes no pacote roda no interpretador selecionado. pytest
precisa estar nesse ambiente; ausência é erro explícito, nunca fallback silencioso.
O Python privado inclui pytest; unittest usa a stdlib. Venv explícito prevalece.

Helper produz JSON estruturado de resultados reais (pass/fail/error/skip, duração,
nome exato, hierarquia e localização). Relatórios10.000casos/16MiB, output1MiB e
campos16KiB; paths externos não recebem navegação. Cases/failed usados em rerun
vêm de cópia privada do relatório, sem confiar em mutações do estado público.
Exitcodes são preservados; saída com palavras como passed não aprova uma execução.
Ausência/malformação de relatório e zero testes são erro; cancel/timeout recolhem
somente o grupo próprio e não publicam sucesso ou resultados antigos.

Argumentos são vetores, sem shell. Helper privado -I remove imports implícitos do
cwd para o engine, depois adiciona explicitamente a raiz revisada para executar
testes confiáveis; pycache e cacheprovider pytest ficam desativados. Configurações/
plugins dos testes podem executar código como parte da ação de execução autorizada.
Configuração/sessão não inicia testes. debug_selected devolve configuração Python
para o caso real via -m pytest/unittest, com trustfalse, para o diálogo existente.

Aceitação: fixtures próprias pass/fail/skip nos dois frameworks, seleção e falhos,
falha corrigida e repetida, path/Unicode, venv com dependência própria, exceção de
coleta e pacote ausente, cancel/timeout com PID ausente, relatório forjado sem
bypass de paths/exit e persistência sem execução. GUI é integrada pelo módulo
compartilhado do produto e tem QA próprio. Linux testado; cobertura, parametrizações
complexas de plugins, Jupyter e todas plataformas não são certificados aqui.

Integração com o interpretador central: set_project_interpreter recebe o caminho
já resolvido pelo worker do projeto. É usado apenas quando o override dos testes
está vazio; o perfil conserva esse campo vazio. Alterar o ambiente central não
converte silenciosamente a configuração de testes em um caminho fixo, e um
interpretador explícito dos testes continua prevalecendo. Ausência de pytest no
venv escolhido gera erro estruturado, sem usar outro Python por conveniência.
