# Python no Codisseu

Abra um projeto e um arquivo `.py`, `.pyw` ou `.pyi`. O mesmo editor das outras
linguagens reconhece Python, aplica cores, comentários com `#`, indentação e
templates próprios. `.pyi` recebe análise, mas não é um programa executável.

Autorize os providers do projeto para iniciar Basedpyright. Completion inclui
autoimports; Ctrl+Q mostra documentação, Ctrl+B vai à definição, referências usam
o painel existente e Shift+F6 abre rename com prévia e aplicação transacional.
Tipos e parâmetros aparecem como dicas visuais; podem ser desligados nas
preferências. Essas dicas não entram no arquivo nem no texto copiado.
Fontes de stdlib, typeshed e bibliotecas reconhecidas do ambiente abrem somente
para leitura. O destaque é lexical; não se anuncia semantic tokens completo.

## Ambiente e execução

Use **Executar → Python · Executar e ambiente…** ou o Play/Shift+F10 com um
arquivo Python ativo. O formulário permite escolher interpretador, diretório,
argumentos (um por linha) e variáveis `NOME=valor`. Seleção vazia usa `.venv`,
`venv` ou o Python privado do Codisseu, nessa ordem. Salvar configuração mantém
a escolha por projeto e atualiza o analisador, os testes e a depuração padrão.
Cada configuração de testes/debug também pode declarar seu interpretador.

Escolha executar arquivo, módulo (`-m`), console REPL, verificar sintaxe ou lint.
O terminal integrado recebe argumentos separados e mostra a saída real. A
verificação de sintaxe usa o parser sem executar o arquivo. Lint usa Ruff.
O martelo de compilação continua cinza no contexto Python. Salve fontes alteradas
antes de executar; a IDE não salva silenciosamente para rodar o projeto.

**Criar ambiente virtual** cria uma pasta nova, sem substituir ambiente existente.
**Instalar pacotes** e **Instalar requirements** usam somente o ambiente virtual
selecionado. Essas ações são explícitas e exigem revisar/autorizar o formulário;
podem baixar dependências e executar builds de pacotes. Abrir projeto, configurar
ou cancelar o formulário não instala nem executa o projeto. Use o terminal para
fluxos específicos de Poetry, uv, Conda e gerenciadores semelhantes.

## Formatação e correções

Ctrl+Alt+L formata o arquivo inteiro com Ruff, preservando edição em memória,
Undo/Redo e salvamento explícito. Ruff respeita as configurações aplicáveis do
projeto. O limite desta operação é256KiB por fonte.

Com o provider pronto, Alt+Enter apresenta ações Ruff que realmente alteram o
buffer: correções seguras do arquivo e organização de imports. Revise a prévia
colorida e aplique; Ctrl+Z desfaz e Ctrl+S salva. Ações são do arquivo inteiro,
não um catálogo completo de correções por diagnóstico. Correções unsafe e
comandos arbitrários de servidor não são executados.

## Testes e depuração

**Executar → Testes Python · pytest/unittest…** abre a configuração e o painel
Run compartilhado. Escolha o framework, execute suite/arquivo e selecione casos
na árvore. Há repetição do caso, de falhas, saída, duração, navegação à fonte e
Stop. O Python privado inclui pytest; se escolher um ambiente próprio, instale
pytest nele ou selecione unittest. Importar/executar testes e plugins ocorre
somente por uma ação de execução. Não há watch automático ou cobertura neste
incremento.

Para depurar, abra a configuração Python no depurador existente. Configure
arquivo **ou** módulo, interpretador, argumentos, cwd e ambiente; marque um
breakpoint no gutter e inicie. Debugpy atende pilha, variáveis, watches,
avaliação, passos, Continue e Stop nos mesmos painéis. A árvore de testes também
oferece debug do caso selecionado, abrindo a configuração para revisão. O
adapter privado pode depurar um venv sem debugpy instalado nesse venv.

## Runtime e limites

O portátil Linux x86_64 inclui CPython3.13.18, Basedpyright1.40.2, Ruff0.17.3,
debugpy1.8.22 e pytest9.2.0. Não exige Python do sistema para os fluxos padrão.
O tooling de testes/debug desta entrega requer Python3.10+; a aceitação foi feita
com o Python3.13 privado e venvs criados por ele. Versões e dependências particulares
do projeto continuam sob sua escolha. Ao mover/remover a instalação Python base,
recrie venvs que dependam dela.

Rename usa bindings reais do provider e uma validação estrutural conservadora;
colisões, sintaxe inválida, versões obsoletas e limites do corpus bloqueiam Apply.
Introspecção, monkey patching, imports dinâmicos e todos os frameworks possíveis
não são certificados. Notebooks/Jupyter, attach remoto, cobertura e equivalência
integral com PyCharm ficam fora desta entrega.

Contratos: [escopo](python-support-contract.md), [análise e rename](python-language-contract.md),
debug, [testes](python-tests-contract.md) e
[runtime](python-runtime-contract.md). Resultados reais em [status](status.md).
