# Página do Codisseu

Página estática em https://lucstrik.github.io/codisseu/, publicada pelo workflow
`.github/workflows/pages.yml`. HTML, CSS e JavaScript sem dependências de build,
sem backend, analytics, cookies próprios ou fontes carregadas de terceiros.

Para visualizar localmente, na raiz do repositório:

```sh
python3 -m http.server 4173 --bind 127.0.0.1 --directory website
```

Abra http://127.0.0.1:4173. Os caminhos relativos também funcionam sob `/codisseu/`
no GitHub Pages. Para publicar mudanças, abra um PR para `main`; depois do CI e
do merge, o workflow envia apenas o conteúdo de `website/` ao Pages.

O canvas é decorativo. O efeito de deformação usa ponteiro preciso e preferência
de movimento normal; celulares e movimento reduzido recebem a grade estática.
O desenho pausa quando converge e quando a aba está oculta. Navegação, download
e conteúdo funcionam sem JavaScript. Não há dependência de serviços de IA para
exibir a página.

## Ativos e procedência

- Logo: cópia dos ícones fornecidos pelo autor, já presentes em `assets/icons/`;
  procedência original em `assets/icons/provenance.json`. O autor pediu a página
  pública do próprio projeto nesta sessão. Nenhum redesenho do logo.
- Captura: cópia sem alteração de `docs/images/codisseu-workspace.png`, gerada
  em 17/11/2026 pelo próprio renderer do Codisseu (`--capture`), com o binário
  da alpha pública v0.1.0-alpha.1 core. Apenas a área cliente, sem painel do
  sistema, recorte ou recompressão. Projeto de demonstração sintético e perfil
  isolado. Resolução 1920×1200, escala 1.5. Procedência e hashes completos em
  `docs/images/README.md`. Substituição autorizada expressamente pelo autor.
- Inter: subset latino WOFF2 de `reference/public/fonts/InterVariable.ttf`,
  convertido com fontTools (fonttools[woff] em contêiner Python oficial isolado).
  Inclui português, pontuação e setas, preservando o eixo variável.
  SIL Open Font License 1.1 em `assets/Inter-OFL.txt`.
- Ícone GitHub: contorno `github` do [Lucide 0.468.0](https://github.com/lucide-icons/lucide/blob/0.468.0/icons/github.svg) (ISC), com aviso em
  `assets/lucide-LICENSE.txt`. Os demais ícones geométricos são SVG próprios.
- A grade, órbitas e efeitos foram implementados em CSS/Canvas para esta página.

Atualize os links de download, versão visível e JSON-LD juntos ao lançar uma nova
alpha. Sitemap e URL canônica usam o endereço público do GitHub Pages. O arquivo
robots.txt dentro do projeto não substitui o robots.txt da raiz do domínio
`lucstrik.github.io`; não é necessário para permitir a indexação da página.
