# Testes JavaScript/TypeScript

Use **Executar → Testes JavaScript/TypeScript · Vitest/Jest…**. Escolha o framework
e a configuração do projeto, ou mantenha Auto. Auto prefere Vitest quando ambos
estão instalados. Node vazio usa o runtime privado do Codisseu; runner vazio usa
`node_modules/vitest/vitest.mjs` ou `node_modules/jest/bin/jest.js`.

**Executar suite JS/TS** executa o projeto; **Executar arquivo JS/TS** limita ao
arquivo ativo. O framework e seus transforms precisam estar instalados no projeto.
Jest/TypeScript usa o transform configurado pelo projeto; a IDE não o substitui.
Abrir configuração não instala pacotes nem executa JavaScript. Salve as alterações
antes de executar; arquivos dirty impedem Run/Debug.

O painel **Run** compartilha o renderer PHPUnit/Pest. Árvore por arquivo/suite/caso,
ícones, duração, Show Passed/Show Ignored, ordenação, expandir/recolher e splitter.
Selecione um teste para ver mensagem/stack; duplo clique/contexto/link abre a fonte.
**Run selected test**, **Rerun Failed Tests** e **Ctrl+F5** usam identidades do
relatório, arquivo e nomes literais; nomes ambíguos no mesmo arquivo são recusados.
O console recebe saída durante execução; os casos vêm do relatório JSON final.

Após uma execução, ative **Rerun Automatically · after Save** na toolbar. Um Save
concluído com mudança no mesmo projeto agenda a última seleção, após300ms.
Saves sucessivos são agrupados; buffers dirty adiam a execução. O controle não
observa saves externos e não usa o processo watch permanente do framework.
**Stop / Ctrl+F2** desarma e interrompe a execução própria. Não rearma ao reiniciar.

Selecione um caso e use **Debug selected test**, ou **Debug suite JS/TS** na
configuração. A ferramenta Debug existente oferece breakpoints, frames, variáveis,
expressões e **Resume Program / F9**. Ao terminar, clique **Run** para os resultados.
Jest roda in-band; Vitest em fork único com auto-attach e source maps. O tempo de
teste Vitest é desativado no debug, mas o prazo total configurado continua valendo.
O estado final combina relatório com código de saída real de um wrapper privado.

Validados em Linux: Vitest5.0.5, Jest30.5.2 e Node24.22.0; TS5.9.3 no transform da
fixture própria. Não certifica todas as versões/plugins, Browser Mode, projetos
remotos, cobertura, snapshots por botão ou descoberta de testes antes de executar.
Configurações ficam no perfil `session.js-tests.json`, fora das fontes do projeto.
