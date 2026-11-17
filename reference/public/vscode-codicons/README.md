# VS Code Codicons — folding

Ativo original, sem modificação, copiado da instalação local autorizada:
`/usr/share/code/resources/app/out/media/codicon.ttf`.

- Produto observado: Visual Studio Code 1.143.1 stable, Linux x64.
- Commit: `e646868227c3e3a9e3f24277c8c7220be42c0be7`.
- Data do build: `2026-10-22T17:23:22Z`.
- Pacote declarado no `ThirdPartyNotices.txt` instalado:
  `@vscode/codicons 0.0.46-36`, licença **CC-BY-4.0**.
- Fonte: <https://github.com/microsoft/vscode-codicons>.
- Licença: <https://creativecommons.org/licenses/by/4.0/>.
- Atribuição: **Codicons © Microsoft Corporation, usados sob CC BY 4.0**.
- Nenhuma alteração à fonte. Tamanho: 153676 bytes.
- SHA-256: `fe9082f0a5b971345a5321ddabb6e710020692f254155e20712242cac7c91858`.

O `LICENSE.txt` conserva integralmente a seção do pacote Codicons no
`ThirdPartyNotices.txt` instalado. A licença do aplicativo VS Code não é
atribuída à fonte; a evidência da licença do componente está nessa seção.

## Glifos utilizados

| Estado | Nome do ícone do editor | Glifo Codicons | Codepoint |
|---|---|---|---|
| Expandido | `folding-expanded` | `chevron-down` | `U+EAB4` / decimal 60084 |
| Recolhido | `folding-collapsed` | `chevron-right` | `U+EAB6` / decimal 60086 |

O bundle instalado `out/vs/workbench/workbench.desktop.main.js` registra os
aliases acima; o DOM executado confirma `font-family: codicon` e os mesmos
codepoints. No cenário Dark Modern/DPR1, a fonte estava em 16 CSS px, cor
`rgb(197, 197, 197)`; a área de clique era 26×19 CSS px. Essas medidas são do
cenário e não definem tamanho universal.

Relatório: `reference/observations/json-folding-vscode-20261026.md`.
