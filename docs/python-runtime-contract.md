# Python privado e portátil —02/11/2026

Complementa python-support-contract.md e portable-runtime-contract.md. Recursos
pertencem à instalação; nenhum pip global, projeto pessoal ou perfil é usado.
Preparação explícita do build baixa somente artefatos oficiais com versão/SHA
fixos. Abrir/instalar o pacote não baixa nem instala ferramentas/dependências.

CPython3.13.18 x86_64 GNU do python-build-standalone Astral release20261026,
com stdlib/pip/venv; Basedpyright1.40.2 MIT (Node existente), Ruff0.17.3,
debugpy1.8.22 e pytest9.2.0 com dependências pinadas. Licenças/notices e
manifestos upstream preservados. Não copiar caches/credenciais/config pessoais.
Outras versões, dependências de projeto e bibliotecas nativas exigem preparação
explícita. Venv criado depois de mover pacote usa o prefixo novo; venv existente
pode precisar ser recriado depois de mover/remover a instalação Python base.

APIs Rust python()/ruff()/python_debug_adapter() resolvem instalação
ou .tools/python ancorado ao checkout de desenvolvimento, nunca ao projeto.
O adapter usa Python privado com debugpy; launch.python usa interpretador do
projeto explicitamente selecionado, mesmo se venv não contiver debugpy. Não
forçar PYTHONHOME/PYTHONPATH global nem substituir interpretador configurado.
python_environment fornece apenas defaults para processos privados; callers
preservam seleção explícita. Provider Python é opcional em manifestos antigos;
quatro providers aceitos, limite de ferramentas32, PHP/TS obrigatórios mantidos.
python_test_runner() aponta ao helper próprio que gera resultados reais de
pytest/unittest. python_library_roots() confina stdlib/typeshed como readonly.
python_environment() não injeta variáveis: o Python relocável deriva seu prefixo
do executável, e tooling privado usa -I explicitamente. Assim nem flags de site
do adapter vazam para o interpretador alvo escolhido pelo projeto.

Aceitação antes do tar: prefixo com espaços movido; CPython/stdlib/ssl/sqlite/
ctypes/pip/venv reais, venv sem debugpy, instalação offline de dependência própria,
run/pytest/unittest, Basedpyright real, Ruff e debugpy breakpoint/evaluate/continue.
Container Ubuntu22offline sem Python do host exposto ao produto e sem checkout,
perfis ou credenciais montados. Regressões PHP/TS/Rust preservadas; tar final
somente após release final, extração/manifesto e recibo dos arquivos substituídos.

Fontes oficiais e hashes: tools/python-tools.lock.json; release Astral20261026,
registro npm Basedpyright e metadados PyPI. PYTHON.json/licenças completas das
dependências do CPython vêm do full archive oficial separado, sem empacotar
objetos/builds. pip26.2.2 faz parte do artefato CPython pinado. Console scripts
pip/pytest/debugpy são wrappers relativos; nenhum shebang da área temporária de
instalação permanece nessas entradas. Debugpy wheel requer glibc2.34+; o alvo
exercitado é Ubuntu22 x86_64, sem certificação universal.
Gramática compilada tree-sitter-python0.25.0 e unicode-normalization0.1.26
preservam MIT/Apache/Unicode/COPYRIGHT pertinentes, checksums Cargo.lock e
hashes dos avisos em tools/portable_python_notices e BUILD.json do pacote.

Antes do pacote integrado: instalação limpa e verificação de10.889arquivos
passaram;6testes runtime e Clippy sem warnings. Aceitação separada dos motores
em árvore movida com espaços passou em6,31s, ampliada depois em6,70s com runner
estruturado pytest/unittest e caso selecionado. Ambos incluem venv/pip offline,
wheel própria, venv semdebugpy, source intacto, Ruff seguro e Basedpyright/
debugpy reais. A ampliação final de paths/linhas de todos os casos, incluindo
decorators de skip, passou em6,83s; um primeiro seletor do harness esperava a
linha de def em pytest, corrigido para a linha real do decorator e preservado
em python-source-line-assertion-first.log. Nenhuma alteração de produção para
esse ajuste. Verificação privada final abrange10.890arquivos e25referências a
licenças declaradas pelo PYTHON.json. Configurações PIP_TARGET/PIP_PREFIX/
PIP_USER/PIP_REQUIRE_VIRTUALENV/PYTHONHOME herdadas e inválidas não redirecionaram
a instalação, conforme install-inherited-env-check.json.

Entrega integrada final concluída no corpus
artifacts/python-integration-20261102/runtime. final-verification/verification.json
passou em Ubuntu22.04.5 offline, imagem Docker registrada por ID, sem engines
PHP/Node/Rust/cc/Python do host, projeto ou perfis pessoais montados: regressão
PHP/TS/API/DAP/formatadores11,55s; Rust7,36s; Python10,76s. São4providers e
17recursos confinados, com notices de gramática/Unicode/ícone Python e os dez
tamanhos de ícones instalados conferidos. Venv/pip/roda própria, pytest/unittest
com casos pass/fail/skip e rerun, source/line exatos, Basedpyright/Ruff/debugpy
reais e fontes preservadas passaram; nenhuma instalação ou download implícito.

dist/codisseu-linux-x86_64 e seu tar.xz/checksum foram atualizados. Tar final:
430709488bytes; SHA256
5228cda01c57019c7742ce3e19484859d0681404a950bd98cc4e22c6d8266b67.
Extração real verificou21.044arquivos do manifesto,4providers/17recursos e
versões Python/pip/debugpy/pytest no prefixo extraído. Payload igual ao release
aprovado:5b6733eed8334d05f25c1206bb57802c84a39c65703a08a3e325d2f7b3eed02a.
Helper final713419ecb6dec72e03eff70e50c0c1fdd9d1b185747f1d657797cfcc8e1d0d61.

delivery.json/archive-verification.json registram origem, hashes e publicação;
previous-dist.json guarda o hash anterior aa61467d… e metadados. Aliases dos
artefatos anteriores foram conferidos inalterados. cleanup.json registra a
remoção apenas das cópias de teste/instalação desta tarefa e backup gerenciado
anterior comprovadamente idêntico ao seu tar:10.641.041.486bytes lógicos, sem
alegar os mesmos blocos físicos liberados. Dist final e .tools/python necessário
ao desenvolvimento foram preservados, assim como notices/fontes/locks/recibos/
logs em delivery-metadata, previous-dist-metadata e candidate-metadata.
Não resta pendência deste contrato; limites de versões/nativos/plataforma acima
continuam explícitos, sem certificação de frameworks ou distribuições não testados.
