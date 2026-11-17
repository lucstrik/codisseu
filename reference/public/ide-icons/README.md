# Public IDE icon corpus

03/11/2026: o globo do botão "Abrir navegador integrado" é desenho vetorial
próprio em `crates/codisseu-ui/src/components.rs` (`Icon::Browser`), usando o
traço/cor monocromáticos já existentes. Não é ativo extraído de navegador/IDE
proprietário e não altera a contagem/manifesto dos arquivos SVG abaixo.

86 SVG sources and corresponding transparent64×64 runtime PNGs. Each icon has its
own source record and hashes in [sources.json](sources.json). The public source
files retain original bytes and copyright comments. This directory contains no
assets extracted from a proprietary IDE installation.

- 75 platform assets: JetBrains/intellij-community at
  `666cd44ae2e77b67cf892c8f64ab199ad70e69ca`, Apache-2.0.
- 1 Rust file icon: intellij-rust/intellij-rust at
  `c6657c02bb62075bf7b7ceb84d000f93dda34dc1`, MIT.
- 1 Elixir file icon: KronicDeth/intellij-elixir at
  `504860a69dec01427ab7e012660c10d72c307946`, Apache-2.0.
- 4 Lucide1.60.0 ISC/MIT-Feather assets: brain, shield-check, arrow-up, trash. Revision02eb9161b57917ccaa262f1bf070507cddde3b08; original bytes retained as .upstream.svg.txt, complete ISC/Feather-MIT notice in LUCIDE-LICENSE. Only currentColor changes to neutral #c9c9ce.
- 1 sidebar AI symbol redrawn from the image explicitly supplied by the user on25/10/2026, transparent/monochrome as requested. Source attachment hash in manifest; no upstream license asserted.
- 4 independently drawn Codisseu fallback assets: PHP, TypeScript, JSX, TSX.

Preserved licenses/notices accompany this directory. The platform revision was
chosen because its LICENSE.txt is the complete Apache2 license. Later versions
of that file contain additional build terms; they are not the license source for
this corpus. Audited parent paths have no component-specific license override
for the selected icon files. Rust's MIT notice includes Aleksey Kladov, Evgeny
Kurbatsky, Alexey Kudinkin and contributors, and JetBrains.

Runtime PNGs are mechanically derived. IntelliJ/file asset geometry and palette remain unchanged; AI palette adaptation is recorded per asset.
Rendering uses system librsvg2.60.0/Cairo1.18.4 at development time only. Keep
these notices when distributing derived assets. No product-logo assets were
selected. Language identifiers/file icons are used to identify file formats,
without implying sponsorship.

Verify locally: `python3 tools/prepare_ide_icons.py --check`.
Rerender verified local SVGs: `python3 tools/prepare_ide_icons.py --render`.
Both operate without network access. Contract and fidelity limits:
docs/m6-file-icons-contract.md.

## Git Insights,25/10/2026

Seis ícones Codicons0.0.45 de Microsoft Corporation, iguais aos glifos públicos
usados pelo GitLens19.4.0, sob [CC BY4.0](https://creativecommons.org/licenses/by/4.0/).
Fonte pública: https://github.com/microsoft/vscode-codicons. Licença integral em
CODICONS-LICENSE. Geometria preservada; currentColor convertido para#c9c9ce e
rasterização transparente64px. Tarball, integridade e comparação com fonte
instalada em artifacts/gitlens-20261025/reference/codicons-provenance.json.
Não incorpora o glifo comercial customizado GitLens nem código plus.
