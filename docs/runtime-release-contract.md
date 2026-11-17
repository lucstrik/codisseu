# Ferramentas ausentes na instalação pública — 16/11/2026

O Codisseu deve abrir e permitir editar/salvar arquivos mesmo sem PHP, Node.js,
Python ou Rust no sistema. O pacote completo fornece motores privados de análise;
interpretadores, extensões e dependências necessários para executar a aplicação
do usuário continuam sendo configuração do projeto. Os motores privados não
substituem comandos nem versões já instaladas pelo usuário.

A distribuição pública básica pode omitir motores privados. Nesse caso inclui
`runtime.json` com schema 1 e listas vazias de tools/providers. O manifesto vazio
é uma instalação válida: nenhuma ferramenta do checkout de desenvolvimento,
`.tools` ou PATH é acrescentada à configuração automaticamente. O usuário pode
configurar um servidor de linguagem já instalado; escolhas explícitas e suas
aprovações exatas são preservadas. O mesmo limite vale para pacotes completos:
um motor opcional ausente não é substituído pelo da máquina de build.
Instalar apenas um interpretador no host não acrescenta adaptadores ou helpers
privados ao pacote básico. Recursos que dependem desses componentes permanecem
indisponíveis com diagnóstico próprio. LSP com comando explícito e Git do host
são caminhos separados, já implementados.

O JSON carregado pela interface/CLI aceita os quatro servidores PHP, TypeScript,
Rust e Python, com o mesmo limite da configuração em memória. O cenário de
ferramentas ausentes carrega as quatro entradas pelo arquivo, sem iniciar
processos durante a leitura, antes de pedir Start explicitamente.

Configuração explícita de servidor continua tendo precedência. Se o executável
não existe, falha ou não responde à inicialização, seu estado deve ser Failed,
com motivo disponível no painel de providers. A falha não torna o documento
somente leitura, não perde buffers e não impede consultas SQLite. Não há
download, instalação ou repetição automática infinita para tentar corrigir isso.
O handshake LSP tem prazo de 15 segundos; trabalho de arquivos e banco usa
workers independentes e precisa continuar durante a espera. Stop continua
possível e servidores são recolhidos pelo ciclo de vida existente.

O manifesto ausente/inválido ou um arquivo privado removido deve produzir erro
de configuração/reinstalação, sem usar uma ferramenta da pasta aberta como
substituta. Abrir um projeto não autoriza scripts npm/Composer, execução da
aplicação nem conexão remota. Confiança continua vinculada à pasta/configuração.

Regressão: `crates/codisseu-core/tests/missing_runtimes.rs` exercita quatro
executáveis inexistentes, edição/undo/redo/salvamento e uma consulta SQLite real.
Em Unix, também usa processo controlado sem resposta LSP para verificar que
salvamento e banco concluem enquanto o provider permanece Starting, seguido da
falha por prazo real. Fixtures e perfis são temporários, fora dos dados pessoais.
Os testes de codisseu-runtime e provider_startup verificam manifesto básico,
rejeição de manifesto parcial/inválido, autoridade da lista empacotada mesmo
vazia, ausência de fallback de desenvolvimento e preservação de escolhas manuais.
Esses testes comprovam o serviço compartilhado; janela nativa e pacote offline
exigem validação própria do artefato final. Não comprovam todas as distribuições,
hardware gráfico, projetos ou combinações de versões de runtimes.
