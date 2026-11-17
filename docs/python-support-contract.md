# Python no Codisseu — contrato,02/11/2026

Pedido: suporte profissional Python no editor único e serviços/painéis nativos,
com a identidade Codisseu e sem regressão PHP/JS/TS/SQL/Rust. Não usar PyCharm,
Pylance proprietário, webview ou respostas inventadas como motor do produto.

Escopo de aceitação desta entrega:

- .py/.pyi/.pyw reconhecidos; lexer Python incremental (docstrings/triple strings,
  f-strings, decorators, funções/classes/keywords/strings/comentários), templates,
  indentação/comentário Python e editor comum, Unicode/CRLF/Undo/Redo/Save.
- Basedpyright aberto/pinado como provider: tipos/diagnósticos/completion/imports,
  hover, assinatura, definição/referências/rename e inlay hints negociados. Mesmos
  limites de versão/cancelamento/filas/read-only e confiança dos outros providers.
  Seleção explícita de interpretador/venv por projeto; configuração e versões
  reais registradas. Não instalar dependências ou executar projeto ao abrir.
- Ruff aberto/pinado para formatar buffer completo com Ctrl+Alt+L e inspeções/
  correções seguras/organização de imports com prévia/Undo, sem Save implícito.
  Configurações pyproject/ruff respeitadas dentro do escopo implementado. Edições
  locais revalidadas; comandos arbitrários/ResourceOperations não anunciados.
- Executar arquivo/módulo, argumentos/cwd/ambiente e REPL no terminal nativo;
  revisão/confiança explícitas, dirty guards, cancelamento/status reais. Python
  não habilita martelo de compilação Cargo. Interpretador persistente, caminho do
  ambiente resolvido em worker, sem executar o interpretador na inspeção; criar venv/instalar pacotes apenas por ação
  explícita e comando revisável, sem pip global ou download na abertura.
- Testes pytest e unittest no Run compartilhado: suite/arquivo/caso/falhos,
  resultados estruturados reais, navegação, saída/exit, cancelamento/limites.
- Debug Python via debugpy aberto: launch com interpretador selecionado,
  breakpoints/condições, pilha/locals/evaluate/watches/steps/continue/Stop nos
  mesmos painéis. Sem tratar .py como Node; nenhum attach remoto silencioso.
- Recursos privados para desenvolvimento e portátil Linux: Python/stdlib,
  Basedpyright/Ruff/debugpy/pytest com hashes/notices/origem. Projeto e venv podem
  precisar de dependências/versões próprias; seleção explícita preservada.
- Modals seguem Tokens e controles atuais; largura/altura/scroll/rodapé não podem
  perder botões em100%/125%. Prévias usam as cores do editor; hints nunca entram
  em fonte/copiar/Save/Undo. Arquivos grandes e processos ficam em workers.

Corpus próprio multi-arquivo, Unicode/CRLF e venv com dependência de teste: provider
real, rename e ausência de alterações em homônimos/comments/strings, quickfix real,
format+Undo/Redo+Save, run com argumentos/output, pytest/unittest (pass/fail/skip),
debug breakpoint/locals42/evaluate42/step/output/Stop e GUI100%/125%. Regressões
anteriores, pacote movido/offline, hashes e entrega do launcher e dist registrados
em status.md. Compilação não substitui teste funcional.

Não declarar equivalência integral com PyCharm nem cobertura de todos frameworks,
notebooks/Jupyter, debug remoto, cobertura de testes ou plataformas não exercitadas.
Erros, limites e funcionalidades não implementadas ficam explícitos no guia/status.
