#!/bin/sh
# Managed by Codisseu portable installer
set -eu
codisseu_source=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
codisseu_data=${XDG_DATA_HOME:-"$HOME/.local/share"}
codisseu_bins=${CODISSEU_BIN_DIR:-"$HOME/.local/bin"}
codisseu_id=$(cat "$codisseu_source/BUNDLE-ID")
case "$codisseu_id" in codisseu-0.1.0-linux-x86_64-????????????) ;; *) echo 'Pacote Codisseu inválido' >&2; exit 1;; esac
if [ "$#" -gt 0 ]; then
    [ "$#" -eq 2 ] && [ "$1" = --prefix ] || { echo 'Uso: install.sh [--prefix PASTA]' >&2; exit 1; }
    codisseu_dest=$2
else
    codisseu_dest=$codisseu_data/codisseu/releases/$codisseu_id
fi
case "$codisseu_dest" in /*) ;; *) echo 'O destino precisa ser absoluto' >&2; exit 1;; esac
codisseu_launcher=$codisseu_bins/codisseu
codisseu_desktop=$codisseu_data/applications/io.codisseu.Codisseu.desktop
# Check ownership before installing or replacing anything.
for codisseu_file in "$codisseu_launcher" "$codisseu_desktop"; do
    if [ -e "$codisseu_file" ] || [ -L "$codisseu_file" ]; then
        [ -f "$codisseu_file" ] && head -c 200 "$codisseu_file" | grep -q 'Managed by Codisseu' || { echo 'Arquivo existente preservado; escolha outra pasta de instalação' >&2; exit 1; }
    fi
done
(cd "$codisseu_source" && sha256sum -c MANIFEST.sha256 >/dev/null)
sh "$codisseu_source/install-command-path.sh" --check "$codisseu_bins"
if [ -e "$codisseu_dest" ]; then
    [ -f "$codisseu_dest/MANIFEST.sha256" ] && cmp -s "$codisseu_source/MANIFEST.sha256" "$codisseu_dest/MANIFEST.sha256" || { echo 'Destino existente preservado' >&2; exit 1; }
    (cd "$codisseu_dest" && sha256sum -c MANIFEST.sha256 >/dev/null)
else
    mkdir -p "$(dirname -- "$codisseu_dest")"
    codisseu_stage=$(mktemp -d "$(dirname -- "$codisseu_dest")/.codisseu-install.XXXXXX")
    trap 'rm -rf -- "$codisseu_stage"' EXIT HUP INT TERM
    cp -R "$codisseu_source/." "$codisseu_stage/"
    (cd "$codisseu_stage" && sha256sum -c MANIFEST.sha256 >/dev/null)
    mv -- "$codisseu_stage" "$codisseu_dest"
    trap - EXIT HUP INT TERM
fi
mkdir -p "$codisseu_bins" "$(dirname -- "$codisseu_desktop")"
# A small owned launcher preserves spaces without an additional interpreter.
codisseu_quoted=$(printf '%s' "$codisseu_dest/bin/codisseu" | sed "s/'/'\\\\''/g")
codisseu_tmp=$(mktemp "$codisseu_bins/.codisseu-launcher.XXXXXX")
printf '#!/bin/sh\n# Managed by Codisseu portable installer\nexec '\''%s'\'' --cli "$@"\n' "$codisseu_quoted" > "$codisseu_tmp"
chmod 755 "$codisseu_tmp"
mv -- "$codisseu_tmp" "$codisseu_launcher"
codisseu_exec=$(printf '%s' "$codisseu_dest/bin/codisseu" | sed 's/\\/\\\\/g;s/"/\\"/g;s/`/\\`/g;s/\$/\\$/g;s/%/%%/g')
codisseu_tmp=$(mktemp "$(dirname -- "$codisseu_desktop")/.codisseu-desktop.XXXXXX")
cat > "$codisseu_tmp" <<DESKTOP
# Managed by Codisseu portable installer
[Desktop Entry]
Type=Application
Version=1.0
Name=Codisseu
Comment=IDE para PHP, JavaScript, TypeScript e bancos de dados
Exec="$codisseu_exec"
Icon=$codisseu_dest/share/codisseu/icons/256.png
Terminal=false
Categories=Development;IDE;
StartupNotify=true
StartupWMClass=io.codisseu.Codisseu
DESKTOP
chmod 644 "$codisseu_tmp"
mv -- "$codisseu_tmp" "$codisseu_desktop"
sh "$codisseu_dest/install-command-path.sh" "$codisseu_bins"
printf 'Codisseu instalado em %s\n' "$codisseu_dest"
