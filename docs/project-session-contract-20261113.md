# Sessão, dados por projeto e saída discreta — 13/11/2026

Requisito do usuário: abrir sem alvo retoma o último diretório; alvo explícito
vence a restauração. Conexões, consultas, conversas e requests/coleções pertencem
ao projeto em que foram criados. Abrir outro projeto não os apresenta.

Identidade é o caminho canônico da pasta ou do arquivo `.code-workspace`, sem
confundir dois arquivos no mesmo diretório. Perfis explicitamente selecionados
para laboratório continuam isolados. Preferências globais e credenciais não
se tornam arquivos dentro dos projetos. Dados legados são preservados; somente
um vínculo previamente conhecido permite atribuí-los a um projeto. Nunca
atribuir automaticamente o cadastro antigo ao primeiro projeto novo aberto.

Entrada desktop e `codisseu` sem alvo retomam a sessão. `codisseu .`, outro alvo
e `--workspace` abrem o destino explicitamente escolhido. `--no-restore` e Nova
janela mantêm a intenção de iniciar vazio. Janelas adicionais podem ter sessão
própria e usar o perfil estável do aplicativo para os dados de seus projetos.
Este requisito substitui o comportamento sem alvo vazio documentado em 11/11.

Fechar sem documentos alterados persiste metadados e aguarda ACK sem exibir
modal instantânea. Progresso aparece somente se o fechamento realmente demora.
Confirmações de alterações não salvas, transações e falhas de gravação continuam
explícitas. O atraso visual não é prazo para abandonar I/O ou simular sucesso.
Nenhuma alteração no perfil normal para executar validação.

Corpus: A→B→A, reinício, alvo explícito, sem alvo, Nova janela, `.code-workspace`,
perfil legado, conversa/coleção em voo, saída limpa sem modal, progresso demorado,
Save falho e saída com documento alterado. Usar projetos/HOME/display descartáveis;
conexões reais de produção permanecem fora da prova.

## Acabamento posterior —13/11/2026

Por pedido do usuário, o Database não apresenta bloco de conexões anteriores nem
botão ou etapa de vínculo. Cadastro novo continua automaticamente no projeto
ativo; arquivos antigos e associações existentes permanecem preservados.
