# Comando Codisseu no terminal — 11/11/2026

> Atualização de13/11: o pedido posterior do usuário substitui a entrada sem
> alvo vazia por restauração do último projeto. `-n` mantém janela vazia; dados
> por projeto usam perfil estável. Contrato vigente: [sessão por projeto](project-session-contract-20261113.md).

Pedido: instalar em outro notebook e usar `codisseu .` para a pasta atual ou
`codisseu` para uma janela sem projeto. O comando pertence à instalação, sem
alias manual ou caminho do computador de build. Referência documental: VS Code
CLI, consultada11/11/2026; sem nova inspeção de instalação proprietária.

Aceitar pasta/arquivo existente e .code-workspace como argumento posicional,
caminhos relativos ao cwd do terminal, espaços/Unicode e `--` para nomes que
começam por hífen. Rejeitar alvo posicional ausente/argumento desconhecido/múltiplas
pastas ambíguas antes de abrir GUI, com exit não zero e mensagem legível.
Preservar --workspace/--open e opções existentes. --help/--version não abrem GUI.

Launcher instalado ativa modo CLI: abre janela independente, retorna ao terminal
sem manter stdout/stderr/TTY abertos, respeita cwd e não substitui a sessão de
outra janela. Sem alvo começa vazio, sem restaurar projeto anterior. --wait
permite aguardar o encerramento da janela. Entrada desktop e executável direto
conservam restauração normal. Erro de spawn/ausência de display deve ser legível;
logs privados de inicialização, sem afirmar prontidão só pelo spawn.

Instalação Linux por usuário registra launcher gerenciado e PATH quando faltar,
preservando conteúdo externo de arquivos de shell, repetição idempotente e
destinos estrangeiros. Bash/login POSIX e zsh usam fragmento próprio; fish usa
conf.d quando aplicável. Não executar dotfiles durante instalação. Novo terminal
carrega PATH; um instalador filho não altera o ambiente do terminal pai.
Mesmo mecanismo no instalador desktop de desenvolvimento e pacote portátil.
Nenhum sudo, download, sobrescrita do comando code ou perfil pessoal no pacote.

Aceite: parser/quoting e cwd; noarg vazio mesmo com sessão anterior; retorno
assíncrono e --wait, erros; instalação repetida em HOME limpo/prefixo com espaços,
PATH inicialmente mínimo, shell configs preservadas, conflito de launcher;
janela nativa e pacote relocável atual conferidos, com manifesto/hash e testes
em ambiente Linux isolado. Não certificar plataformas não exercitadas.
