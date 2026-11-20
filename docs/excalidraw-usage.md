# Desenhos no Codisseu

Abra um projeto e clique em **Desenhos · Excalidraw** na barra esquerda.
Informe um nome, como `arquitetura`, e clique em **Criar desenho**. O arquivo
será criado em `drawings/arquitetura.excalidraw`. Para escolher outra pasta,
informe um caminho relativo, como `docs/fluxo.excalidraw`.

Arquivos `.excalidraw` e `.excalidraw.json` também abrem pela árvore ou pelo
comando Abrir arquivo. O desenho ocupa uma aba normal, junto do código e das
outras ferramentas. **Código JSON** permite inspecionar ou editar sua fonte;
**Desenho** volta ao canvas. Arquivos vazios são cenas novas; JSON inválido
exibe o erro e continua disponível para correção.

Use a barra flutuante central para selecionar, mover a tela, desenhar retângulos,
elipses, losangos, setas, linhas, traços livres e textos, ou usar a borracha.
O cadeado mantém a ferramenta ativa para desenhar várias formas; desativado,
a seleção volta após concluir a forma. Selecione um objeto
para editar estilo, mover, redimensionar, duplicar, excluir ou alterar a ordem.
As propriedades aparecem no painel contextual. Texto é editado no próprio
canvas e pode ser aberto novamente pela seleção. Espaço e arraste movem a tela;
Ctrl+roda ajusta o zoom; **Enquadrar** mostra o desenho inteiro.

O menu do canto superior esquerdo reúne **Salvar desenho**, **Código JSON**,
**Copiar SVG**, aparência e grade. **Salvar desenho** ou Ctrl+S grava o arquivo. Ctrl+Z desfaz e Ctrl+Shift+Z ou
Ctrl+Y refaz usando o histórico comum do documento. Fechar com alterações segue
a confirmação normal dos arquivos. Conflitos com mudanças externas também usam
o fluxo comum do editor. **Copiar SVG** exporta a cena para a área de transferência.
Zoom, enquadramento e desfazer/refazer ficam no rodapé do canvas.

A aparência **Sistema** acompanha o Codisseu, escuro por padrão. No menu,
**Claro** e **Escuro** permitem escolher a exibição desse desenho. A preferência
é lembrada ao salvar. A troca de aparência preserva as cores originais dos
objetos e do arquivo; SVG usa essas cores originais. A grade fica desligada
por padrão e pode ser ativada no mesmo menu.

O catálogo pertence ao projeto aberto. Ao voltar à pasta ou ao workspace, seus
desenhos registrados existentes são reabertos automaticamente, inclusive os
que tiveram a aba fechada antes. Outro projeto tem outro catálogo; dois arquivos
`.code-workspace` na mesma pasta também têm identidades distintas. Arquivos
ausentes são preservados no catálogo, sem serem recriados automaticamente.
O limite comum de 32 documentos abertos permanece; desenhos restantes podem
ser abertos pelo painel depois de fechar outras abas.

Este editor é próprio, em Rust/egui, compatível com o formato JSON Excalidraw v2.
Não executa a extensão do VS Code nem o componente React original. Imagens,
frames e outros objetos sem renderer nativo são preservados no arquivo e
identificados na interface. A aparência de traços/fontes e recursos como
bibliotecas, colaboração, bindings e exportação com fonte embutida não têm
paridade integral com o Excalidraw. SVG recusa objetos sem suporte em vez de
produzir uma exportação silenciosamente incompleta. Não há acesso à nuvem.
