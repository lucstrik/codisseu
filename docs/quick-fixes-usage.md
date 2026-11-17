# Correções rápidas PHP/JavaScript/TypeScript

Com o projeto confiável e seu provider iniciado, coloque o cursor no problema e
pressione **Alt+Enter**. O menu do editor oferece a mesma ação em arquivos PHP,
JS/JSX e TS/TSX. O catálogo vem do servidor real e depende do código/projeto.

A lista abre junto do cursor, com a prévia ao lado. Setas e hover selecionam a
correção; a prévia é preparada automaticamente. **Antes/Depois** alterna o texto.
**Enter**, clique na ação já selecionada ou **Aplicar correção** aplica a proposta
cuja prévia está disponível. **Escape**, **Cancelar** ou clique fora fecha.
Enquanto a prévia estiver sendo preparada, Enter não aplica uma proposta antiga.

Aplicar altera somente os buffers; Ctrl+Z/Ctrl+Shift+Z desfaz/refaz a transação,
e Ctrl+S salva. Se o texto ou um arquivo envolvido mudar, consulte novamente.

Validados: correção de variável PHP, spelling JS/JSX/TS/TSX e import TypeScript,
Unicode/CRLF, buffer não salvo, revalidação e Undo/Redo/Save. O provider PHP atual
oferece parte de suas sugestões como comandos; essas aparecem indisponíveis
quando não há edição verificável. Nenhum workspace/executeCommand é enviado.
O adapter da versão6.0.2 do servidor TypeScript descarta somente seu wrapper
auxiliar comprovadamente vazio, conservando a edição para revisão.

Rust mantém seu fluxo anterior. Não há promessa de reproduzir todo o catálogo de
inspeções/intentions do PhpStorm nem de executar commands arbitrários de plugins.
Contrato: [quick-fixes-contract.md](quick-fixes-contract.md).
