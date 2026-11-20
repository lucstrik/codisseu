# codisseu-draw

Modelo Rust de cenas `.excalidraw` versão 2, independente da GUI. O Codisseu
implementa seu próprio renderer egui; este modelo não incorpora React,
JavaScript, CSS ou o motor Excalidraw. A interface usa o símbolo oficial MIT
como ícone da ferramenta, com procedência e licença registradas separadamente
em `reference/public/excalidraw-icon`.

Formato consultado em 20/11/2026 (`documented`):
[JSON Schema oficial](https://plus.excalidraw.com/docs/sdk/codebase/json-schema).
Os campos desconhecidos da cena e dos elementos, `appState` e os dados de `files`
são preservados semanticamente na serialização JSON. Espaços e ordem das chaves
podem mudar. Tipos desconhecidos permanecem na cena sem serem convertidos em um
tipo conhecido. Isso permite salvar sem apagar imagens, frames ou metadados de
outros editores; não comprova renderização ou edição dessas funcionalidades.

O modelo expõe retângulo, elipse, losango, seta, linha, traço livre e texto.
`Scene::export_svg` produz uma representação vetorial desses elementos, com
rotação, transparência, cor, texto Unicode e escape XML. A representação usa
formas regulares e fontes genéricas: não reproduz o desenho RoughJS, fontes
Excalidraw ou todas as regras de bindings, alinhamento e grupos do editor
original. SVG é rejeitado se houver um tipo visível não suportado, evitando uma
exportação que descarte silenciosamente esses elementos. Cores remotas/CSS e
HTML não são executados.

Limites: arquivo de até 16 MiB, 20.000 elementos e 200.000 pontos. Geometria e
estilos numéricos precisam ser finitos e validados. O histórico de
`DrawingDocument` mantém até 64 estados e 32 MiB por pilha. A interface do
Codisseu pode usar o histórico comum do editor em vez deste adaptador.

I/O é síncrono e deve ser invocado pelo worker do aplicativo. Arquivos novos
usam gravação sem sobrescrita. Arquivos existentes comparam os bytes lidos,
preservam permissões, verificam conflitos antes e depois da preparação e são
trocados por arquivo temporário no mesmo diretório. Symlinks finais e múltiplos
hard links são recusados. Isso não fornece trava entre processos concorrentes,
preservação de ACL/xattrs nem garantia testada de queda de energia ou
Windows/macOS. Falha de sincronização do diretório após commit retorna aviso
em `DiskSnapshot::durability_warning`, sem fingir que nada foi gravado.

Validação local do crate: `cargo test -p codisseu-draw --locked` e
`cargo clippy -p codisseu-draw --all-targets --locked -- -D warnings`.
