# Codisseu Git worker

Implementação própria em Rust que usa o Git instalado como processo externo. Não contém código nem recursos de interface extraídos de IDEs. Desenvolvimento e testes locais realizados com Git 2.47.3 em Linux.

## Contrato do serviço

`GitService::dispatch(GitCommand)` aceita uma operação por vez; a fila possui capacidade um. `poll()` recebe o resultado sem bloquear a GUI. `is_busy()` inclui todo o processamento. `Cancel` usa sinal compartilhado, invalida a prévia e interrompe o grupo de processos Unix. Nenhuma resposta significa rollback implícito: `GitReport.outcome_unknown` exige conferir o snapshot antes de repetir uma mutação que foi interrompida.

`Open { path, trusted: false }` apenas descobre/lê o repositório. `Trust(true)` é autorização explícita para comandos de escrita e execução dos hooks/filtros/assinatura normalmente configurados no Git. Preparações de ações locais, workflows e resolução de conflitos produzem uma única `GitPreview`, ligada ao identificador, geração e estado observado. `Apply` consome uma única prévia. Trocar repositório, revogar confiança ou cancelar invalida a autorização anterior. O núcleo da aplicação deve bloquear mutações enquanto há buffers de editor não salvos ou operações de arquivos pendentes.

O snapshot inclui HEAD, branch, índice, conteúdo dos caminhos alterados, refs locais/remotas/tags e configurações locais do repositório. A leitura repete status/índice/refs ao final e rejeita alterações detectadas durante a captura. A aplicação repete a captura antes de escrever. Isso detecta mudanças externas e evita aplicar uma prévia já obsoleta; não constitui isolamento transacional contra todo programa externo que deliberadamente ignore os locks normais do Git.

## Operações locais

Status usa porcelain v2 com separação NUL; caminhos usam `OsString` e pathspecs literais. Stage/unstage de arquivos e hunks trabalham em índice temporário e publicam sob lock do índice real. Commits `Paths` usam exclusivamente as versões selecionadas do índice. `WorkingPaths` usa os arquivos selecionados da árvore de trabalho, incluindo novos arquivos; preserva alterações previamente preparadas em arquivos excluídos da seleção. `AllStaged` usa as alterações preparadas. Amend pode alterar só a mensagem; `CommitDetails.message` preserva o corpo inteiro da mensagem anterior. Hooks e assinatura configurados continuam sendo respeitados.

O commit utiliza índice temporário; falha de hook não publica esse índice no índice real. Após um commit bem-sucedido, o índice real é reconciliado apenas nos caminhos selecionados, mantendo a preparação dos demais. Se ocorrer falha depois de criar o commit, a resposta não promete desfazê-lo. Não existe push automático.

`Discard` exige confirmação de prévia e restaura versões existentes no HEAD observado. Não apaga arquivos untracked ou adições novas. Checkout normal mantém as recusas de segurança do Git. Checkout de revisão usa HEAD destacado explicitamente. Branch delete usa `-d`, sem force. Tags podem ser criadas, listadas e excluídas explicitamente.

Histórico pagina até 200 entradas, com deslocamento máximo de 5000. `HistoryReference` filtra uma branch/tag/revisão no backend, resolvendo-a para um OID imutável. A interface deve reutilizar `HistoryPage.resolved_reference` nas páginas seguintes e voltar ao nome selecionado ao atualizar ou trocar o filtro. Assim a mesma cadeia de ancestrais é paginada mesmo que a branch se mova externamente. O comando `History` anterior mantém o histórico de todas as referências. O grafo é um algoritmo próprio de trilhas calculado sobre o prefixo topológico, mantendo as mesmas arestas ao paginar. Blame retorna até 500 linhas. Diffs incluem arquivos untracked e payload binário, mas seleção de hunks se restringe a alterações textuais de conteúdo; renomeações, modos, adições e remoções são preparadas como arquivo inteiro.

## Processo e orçamentos

Não há shell para construir comandos do produto. Argumentos são separados; stdin, stdout e stderr são lidos/escritos com pipes não bloqueantes. O processo tem grupo próprio para cancelamento. Saída máxima: 8 MiB; diagnóstico: 256 KiB; entrada: 8 MiB. Prazos padrão por processo: 20 segundos; descoberta 15 segundos; clone/commit e operações com índice alternativo até 120 segundos. O snapshot aceita até 10000 caminhos alterados, 32 MiB por arquivo e 64 MiB agregados de conteúdo alterado; índice até 256 MiB. Estourar o orçamento produz erro explícito.

Leituras desabilitam fsmonitor, external diff, textconv e filtros de conteúdo configurados. Status automático usa `--ignore-submodules=all`: alterações de gitlink preparadas no índice permanecem visíveis, mas mudanças internas do submódulo exigem abrir o próprio submódulo como workspace. Não há varredura automática dos arquivos internos nem execução de seus filtros locais. Isso impede helpers externos durante detecção/leitura automática, mas pode mostrar alterações conservadoras em repositórios que dependem de filtros. Operações explicitamente autorizadas usam os filtros normais. Prompts de terminal/askpass e editor são não interativos. SSH usa BatchMode e verificação estrita de hosts já conhecidos; wrappers SSH personalizados não são usados. `ext::` é proibido. URLs em diagnósticos/listas ocultam userinfo e query; clone recusa credenciais inline. Arquivos/diffs não são reescritos para redigir seu conteúdo.

O runner funcional é Unix. Outras plataformas preservam a API, mas recusam execução até existir implementação equivalente de cancelamento por grupos/jobs e pipes; não foi declarada validação Windows.

## Evidências

`tests/local_real.rs` usa repositórios e arquivos descartáveis próprios: índice vs árvore de trabalho, seleção e exclusão de arquivos, hunks/reversão, amend, conflitos de versão do snapshot, cancelamento de hook, hooks que falham, clone local, orçamentos, Unicode/não UTF-8/newline/leading dash/CRLF, renomeações, worktree vinculado, histórico/grafo, tags/branches/blame/detalhes e symlinks ancestrais. `tests/workflows_real.rs` valida os workflows e conflitos do módulo correspondente com repositórios bare locais; não usa remotos de empresa, credenciais ou serviços de produção.

Documentação primária consultada: [Git status](https://git-scm.com/docs/git-status), [Git update-index](https://git-scm.com/docs/git-update-index), [Git log](https://git-scm.com/docs/git-log), [Git commit](https://git-scm.com/docs/git-commit). A ferramenta Git é executada externamente; suas fontes não foram incorporadas. Dependências Rust: sha2 0.10.9 (MIT/Apache-2.0), tempfile do workspace (MIT/Apache-2.0), nix 0.28.0 (MIT).
