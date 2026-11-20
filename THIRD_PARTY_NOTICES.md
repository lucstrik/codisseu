# Componentes de terceiros

O código próprio do Codisseu é distribuído sob a [licença MIT](LICENSE).
Bibliotecas, fontes, ícones e outros componentes mantêm suas próprias licenças
e os créditos de seus autores; a MIT do Codisseu não substitui esses termos.

O instalador público inclui os avisos em `/opt/codisseu/share/codisseu/notices`.
O arquivo `cargo-notices.json` nessa pasta registra a seleção Linux normal/build,
as licenças declaradas e os hashes dos textos preservados. As atribuições dos
ativos ficam em `assets/` dentro dos notices e em
`reference/public` no código-fonte.

O fork de `egui-winit` conserva MIT/Apache-2.0 e a descrição da modificação em
[`vendor/egui-winit/CODISSEU-PATCH.md`](vendor/egui-winit/CODISSEU-PATCH.md).
Fontes tipográficas, Codicons e ícones têm avisos próprios, incluindo OFL,
Ubuntu Font Licence e CC BY 4.0 conforme cada componente.

O símbolo Excalidraw usa a geometria oficial `LogoIcon`, Copyright (c) 2020
Excalidraw, sob a [licença MIT preservada](reference/public/excalidraw-icon/LICENSE).
É aplicado em uma única cor, sem fundo, para identificar a ferramenta de desenho.
A [procedência e os hashes](reference/public/excalidraw-icon/sources.json) registram
a revisão, a adaptação do SVG e a rasterização usada no aplicativo.

Este primeiro `.deb` contém o núcleo do aplicativo. As bibliotecas do sistema
são dependências instaladas pelo `apt`; não são cópias privadas no pacote.
O pacote completo de motores usado no desenvolvimento não acompanha esta release.

Para reproduzir a coleta após preparar as dependências Cargo:

```sh
python3 tools/release_notices.py --output artifacts/release-notices
```

O coletor falha se faltar um texto de aviso. Seu resultado é um inventário
reproduzível, não uma certificação jurídica ou uma análise de vulnerabilidades.
