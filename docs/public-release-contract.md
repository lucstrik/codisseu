# Publicação do Codisseu — 16/11/2026

M7, fatia de distribuição Linux. O usuário autorizou publicar o aplicativo e o
código em sua conta GitHub, preparar a página, usar as três fotos fornecidas e
distribuir o instalador. A licença MIT foi escolhida expressamente nesta sessão.

## Cenário e critérios

Uma pessoa acessa `lucstrik/codisseu`, entende o estágio alpha, baixa o `.deb`
amd64 e seu SHA-256, instala com `apt` e abre o aplicativo. Pode editar e salvar
PHP/JS/TS e outros textos e usar SQLite sem PHP, Node, Rust ou Python externos.
Ferramentas configuradas manualmente continuam funcionando. Falha de provider
é comunicada e não impede edição; inicialização sem resposta tem timeout.

O primeiro pacote público é **core**. Não inclui o pacote privado completo de
motores, PHP/Node/Python/Rust, Phpactor, CEF ou bibliotecas ELF copiadas. O `apt`
instala as bibliotecas de sistema declaradas. Um manifesto instalado vazio é
válido e impede fallback para o checkout do desenvolvedor; configurações
explícitas de providers continuam prioritárias. Integrações que dependem de
helpers privados permanecem indisponíveis, com diagnóstico. Não se anuncia que
instalar apenas Node/PHP habilita essas integrações.

## Código e procedência

- Publicar snapshot revisado, em repositório Git separado, preservando worktree
  e histórico locais. Não copiar `.git`, `.tools`, perfis, credenciais, projetos
  privados, artefatos ou registros de referências proprietárias.
- O exportador usa allowlist, rejeita symlinks e verifica padrões fortes de
  segredos; achados são classificados sem imprimir valores. Isso não é prova
  de ausência universal de dados sensíveis. Transformações e hashes constam no
  manifesto do snapshot.
- A fixture Mongo de agregação usa banco/coleção e IDs sintéticos; testes e
  gerador são atualizados juntos. As fotos originais foram autorizadas pelo
  usuário para o README; a procedência acompanha os JPGs.
- Código próprio sob MIT; fontes, ícones, dependências Cargo e o fork de
  egui-winit conservam avisos próprios. Não substituir suas licenças pela MIT.
- Gerar notices a partir do grafo Cargo normal/build Linux selecionado para o
  aplicativo, incluindo fontes incorporadas e outras atribuições. O manifesto
  registra licença declarada, checksum e avisos preservados. A alternativa
  Apache-2.0 está disponível para self_cell; nenhuma licença GPL/LGPL obrigatória
  foi identificada no grafo selecionado. Não se declara auditoria jurídica total.

## Validação e publicação

Testes Rust de runtimes ausentes, timeout e regressões; pacote Debian sem hooks;
extração e hashes; instalação/remoção em contêiner próprio, com rede desligada
nos smokes; janela X11 em perfil/display isolados, sem alterar instalação normal.
Registrar resultados e limites reais em `docs/status.md` e relatório público.

README, guias, licença, issues, política de segurança, CI, descrição, tópicos e
release com instalador/checksum precisam de links reais. CI criado não equivale
a CI aprovado: conferir conclusão remota e registrar separadamente do teste local.
Não anunciar Ubuntu22.04, Windows/macOS/ARM ou Wayland como validados pelo .deb.
O binário atual exige glibc2.39+, conforme ELF e dependências calculadas.
