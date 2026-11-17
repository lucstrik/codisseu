# Abrir Codisseu pelo terminal

Depois de instalar o pacote Linux, use:

```sh
codisseu .                       # pasta atual do terminal
codisseu                         # retomar o último projeto
codisseu -n                      # nova janela vazia
codisseu "/projetos/Minha pasta"  # outra pasta
codisseu arquivo.php              # arquivo existente
codisseu --wait .                 # aguardar a janela encerrar
codisseu --help
codisseu --version
```

Caminhos relativos partem da pasta do terminal. Espaços e acentos funcionam;
use aspas quando necessário. Para nome iniciado por hífen, use `codisseu -- ./-pasta`.
Aceita uma pasta, ou um arquivo .code-workspace, e arquivos existentes.
Para várias pastas, use um .code-workspace. Alvo posicional ausente e opção desconhecida
retornam erro antes da janela; esta fatia não cria arquivo inexistente por argv.

O comando abre a janela e libera o terminal. Sem caminho, retoma o último projeto
salvo ao fechar, assim como a entrada do menu/dock. Um caminho explícito prevalece
sobre essa restauração. `--wait` aguarda a janela inteira encerrar. `-n` ou
`--new-window` inicia uma janela vazia com sua própria sessão; `--no-restore`
também permite iniciar sem retomar abas. Dados de banco, IA e API pertencem ao
projeto. Janelas do perfil normal usam o mesmo perfil estável; `--session`
explícito continua isolando laboratórios, salvo um `--profile` compartilhado
explicitamente fornecido. Se várias janelas comuns estiverem abertas, a última
a fechar define os metadados retomados na próxima abertura.

O `.deb` público cria o comando em `/usr/bin/codisseu` e registra o aplicativo
no menu do desktop. Não precisa criar alias, editar o PATH ou reiniciar o shell.
Esse instalador não escreve nas configurações de Bash, zsh ou fish. O comando
`code` continua pertencendo ao VS Code.

O instalador portátil usado no desenvolvimento tem outro funcionamento: registra
o comando por usuário em `~/.local/bin` e pode preparar um fragmento de PATH.
Ele não é o instalador `.deb` desta primeira release pública.

Logs privados de inicialização das janelas desacopladas ficam em
`$XDG_STATE_HOME/codisseu/cli-logs` ou `~/.local/state/codisseu/cli-logs`.
Um erro imediato aponta para o log. `--help` e `--version` funcionam sem display;
a janela exige uma sessão gráfica do desktop. Não inicia downloads ou scripts
de projeto para preparar o comando.

[Instalação em outro Linux](install-linux.md),
[contrato e critérios](cli-launch-contract-20261111.md).
