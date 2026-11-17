#!/bin/sh
# Managed by Codisseu command PATH installer
# Install a data-only PATH fragment; never execute a user's shell startup files.
set -eu
codisseu_check=false
if [ "${1-}" = --check ]; then codisseu_check=true; shift; fi
[ "$#" -eq 1 ] || { echo 'Uso: install-command-path.sh [--check] PASTA_BIN' >&2; exit 1; }
codisseu_bin=$1
case "$codisseu_bin" in /*) ;; *) echo 'A pasta do comando precisa ser absoluta' >&2; exit 1;; esac
case ":${PATH-}:" in *:"$codisseu_bin":*)
    $codisseu_check || echo 'Comando codisseu disponível no PATH.'
    exit 0;;
esac
case "${HOME-}" in /*) ;; *) echo 'HOME precisa ser absoluto para configurar o comando' >&2; exit 1;; esac
codisseu_config=${XDG_CONFIG_HOME:-"$HOME/.config"}
case "$codisseu_config" in /*) ;; *) echo 'XDG_CONFIG_HOME precisa ser absoluto' >&2; exit 1;; esac
codisseu_fragment=$codisseu_config/codisseu/path.sh
codisseu_fish=$codisseu_config/fish/conf.d/codisseu.fish
set -- "$HOME/.bashrc"
if [ -e "$HOME/.bash_profile" ] || [ -L "$HOME/.bash_profile" ]; then
    set -- "$@" "$HOME/.bash_profile"
elif [ -e "$HOME/.bash_login" ] || [ -L "$HOME/.bash_login" ]; then
    set -- "$@" "$HOME/.bash_login"
else
    set -- "$@" "$HOME/.profile"
fi
codisseu_zsh=${ZDOTDIR:-"$HOME"}/.zshrc
case "${SHELL-}" in */zsh) set -- "$@" "$codisseu_zsh";;
    *) if [ -e "$codisseu_zsh" ] || [ -L "$codisseu_zsh" ]; then set -- "$@" "$codisseu_zsh"; fi;;
esac
for codisseu_file in "$@"; do
    if [ -L "$codisseu_file" ] || { [ -e "$codisseu_file" ] && [ ! -f "$codisseu_file" ]; }; then
        echo "Arquivo de shell preservado (não é arquivo regular): $codisseu_file" >&2; exit 1
    fi
    if [ -f "$codisseu_file" ] && ! awk '
        $0 == "# >>> Codisseu command PATH >>>" { if (inside) exit 1; inside=1 }
        $0 == "# <<< Codisseu command PATH <<<" { if (!inside) exit 1; inside=0 }
        END { if (inside) exit 1 }
    ' "$codisseu_file"; then
        echo "Bloco de PATH incompleto preservado: $codisseu_file" >&2; exit 1
    fi
done
codisseu_use_fish=false
case "${SHELL-}" in */fish) codisseu_use_fish=true;; esac
if [ -d "$codisseu_config/fish" ]; then codisseu_use_fish=true; fi
for codisseu_file in "$codisseu_fragment" "$codisseu_fish"; do
    if [ "$codisseu_file" = "$codisseu_fish" ] && ! $codisseu_use_fish; then continue; fi
    if [ -L "$codisseu_file" ] || { [ -e "$codisseu_file" ] && { [ ! -f "$codisseu_file" ] || ! head -c 200 "$codisseu_file" | grep -q '# Managed by Codisseu command PATH'; }; }; then
        echo "Configuração não gerenciada preservada: $codisseu_file" >&2; exit 1
    fi
done
$codisseu_check && exit 0
mkdir -p "$(dirname -- "$codisseu_fragment")"
codisseu_stage=$(mktemp -d "$(dirname -- "$codisseu_fragment")/.path-install.XXXXXX")
trap 'rm -rf -- "$codisseu_stage"' EXIT HUP INT TERM
codisseu_quote() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }
{
    echo '# Managed by Codisseu command PATH'
    printf 'codisseu_command_bin=%s\n' "$(codisseu_quote "$codisseu_bin")"
    cat <<'FRAGMENT'
case ":${PATH-}:" in
    *:"$codisseu_command_bin":*) ;;
    *) export PATH="$codisseu_command_bin${PATH:+:$PATH}";;
esac
unset codisseu_command_bin
FRAGMENT
} > "$codisseu_stage/fragment"
chmod 644 "$codisseu_stage/fragment"
mv -- "$codisseu_stage/fragment" "$codisseu_fragment"
{
    echo '# >>> Codisseu command PATH >>>'
    printf 'if [ -r %s ]; then . %s; fi\n' "$(codisseu_quote "$codisseu_fragment")" "$(codisseu_quote "$codisseu_fragment")"
    echo '# <<< Codisseu command PATH <<<'
} > "$codisseu_stage/hook"
for codisseu_file in "$@"; do
    mkdir -p "$(dirname -- "$codisseu_file")"
    codisseu_tmp=$(mktemp "$(dirname -- "$codisseu_file")/.codisseu-shell.XXXXXX")
    if [ -f "$codisseu_file" ]; then
        awk '
            FILENAME == ARGV[1] {
                if ($0 == "# >>> Codisseu command PATH >>>") { skip=1; next }
                if ($0 == "# <<< Codisseu command PATH <<<") { skip=0; next }
                if (!skip) print
                next
            }
            { print }
        ' "$codisseu_file" "$codisseu_stage/hook" > "$codisseu_tmp"
        chmod --reference="$codisseu_file" "$codisseu_tmp"
    else
        cat "$codisseu_stage/hook" > "$codisseu_tmp"
        chmod 644 "$codisseu_tmp"
    fi
    mv -- "$codisseu_tmp" "$codisseu_file"
done
if $codisseu_use_fish; then
    mkdir -p "$(dirname -- "$codisseu_fish")"
    codisseu_tmp=$(mktemp "$(dirname -- "$codisseu_fish")/.codisseu-fish.XXXXXX")
    {
        echo '# Managed by Codisseu command PATH'
        printf 'if not contains -- %s $PATH\n' "$(codisseu_quote "$codisseu_bin")"
        printf '    set -gx PATH %s $PATH\n' "$(codisseu_quote "$codisseu_bin")"
        echo end
    } > "$codisseu_tmp"
    chmod 644 "$codisseu_tmp"; mv -- "$codisseu_tmp" "$codisseu_fish"
fi
echo 'Comando codisseu configurado. Abra um novo terminal para carregar o PATH.'
