# Apresentação pública do Codisseu no GitHub Pages

## Cenário e escopo

Pedido do autor em 17/11/2026: página única, gratuita no GitHub Pages, visual
moderno, fundo de papel quadriculado que reage ao mouse, efeitos de hover,
boa apresentação nos buscadores e acesso ao repositório pelo ícone do GitHub.
Não altera a interface Rust/egui do aplicativo nem seu pacote Linux.

## Contrato

- Fonte em `website/`, sem backend ou instalação de pacotes para servir a página.
- Publicação somente do diretório estático por Actions após merge na main
  protegida; regras de CI e revisão existentes preservadas.
- Links de download apontam para o asset canônico da alpha pública. Versão,
  arquitetura, tamanho e requisitos devem acompanhar o pacote realmente entregue.
- Sem prometer suporte não validado, motores ausentes, desempenho não medido,
  depoimentos ou contagens de usuários inexistentes.
- Conteúdo e ações permanecem acessíveis sem JavaScript; canvas decorativo,
  sem captura de teclado, sem impedir rolagem, com fallback estático.
- Deformação só em ponteiro preciso e sem preferência de movimento reduzido.
  Frames limitados, DPR limitado e pausa em repouso/aba oculta.
- Cabeçalhos semânticos, rótulos acessíveis, foco visível, navegação por teclado,
  layout responsivo, metadados sociais, URL canônica e dados estruturados.
- Sem rastreadores ou cookies próprios. Fontes e imagens servidas pelo próprio
  site; licenças e origem dos ativos registradas em `website/README.md`.

## Evidências esperadas

Inspeção real em navegador desktop e viewport móvel, interação do cursor,
acordeões e links; contraste/foco e movimento reduzido; validação dos caminhos
sob `/codisseu/`, sintaxe JavaScript, HTML/metadados/JSON-LD/sitemap. CI nativo
obrigatório e deploy Pages aprovados, HTTP público e captura da página publicada.
Isso verifica a apresentação, sem provar compatibilidade extra da IDE nativa.
