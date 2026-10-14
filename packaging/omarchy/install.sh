#!/usr/bin/env bash
# Install Orchard Download Manager on Omarchy (or any Arch-based system), per user,
# no root needed except for the runtime libraries:
#
#   curl -fsSL https://raw.githubusercontent.com/sandboxly/orchard-download-manager/main/packaging/omarchy/install.sh | bash
#   ... | bash -s -- --bind --free-f9      # also add a key and free F9 for the app
#   ... | bash -s -- --uninstall
#
# Options
#   --bind        Super+Ctrl+Shift+D opens Orchard Download Manager
#   --free-f9     unbind F9 from Omarchy's push-to-talk dictation, so F9 reaches the
#                 app (Settings); dictation keeps working if you rebind it
#   --version X   install release X (for example 1.0.0) instead of the latest
#   --uninstall   remove the app, the launcher entry and the key binding
#
# The app lands in ~/.local/opt/orchard-download-manager and is linked as
# ~/.local/bin/orchard-download-manager. It ships everything else, so
# nothing else is installed system-wide except the libraries listed in DEPS.
set -euo pipefail

REPO="sandboxly/orchard-download-manager"
ASSET="Orchard-Download-Manager-Linux.tar.gz"
PREFIX="$HOME/.local/opt/orchard-download-manager"
BIN="$HOME/.local/bin/orchard-download-manager"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"
BINDINGS="$HOME/.config/hypr/bindings.lua"
MARK="-- Orchard Download Manager (added by its installer)"
MARK_F9="-- Orchard Download Manager: F9 is its settings (added by its installer)"
# Window system, keyboard, audio, fonts and GPU loader. Most are on Omarchy already.
DEPS=(wayland libxkbcommon libx11 libxcursor libxi fontconfig vulkan-icd-loader mesa ca-certificates)

say() { printf '\033[1m%s\033[0m\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

uninstall() {
  rm -rf "$PREFIX"
  rm -f "$BIN" "$APPS/orchard-download-manager.desktop" "$ICONS/orchard-download-manager.png"
  if [[ -f "$BINDINGS" ]] && grep -qF -- "$MARK" "$BINDINGS"; then
    # The marker line and the bind line after it.
    sed -i "\|^$MARK\$|,+1d" "$BINDINGS"
  fi
  if [[ -f "$BINDINGS" ]] && grep -qF -- "$MARK_F9" "$BINDINGS"; then
    sed -i "\|^$MARK_F9\$|,+1d" "$BINDINGS"
  fi
  update-desktop-database "$APPS" 2>/dev/null || true
  say "Orchard Download Manager removed. Settings in ~/.config/orchard-download-manager stay."
}

install_deps() {
  local missing=()
  for d in "${DEPS[@]}"; do pacman -Qq "$d" &>/dev/null || missing+=("$d"); done
  [[ ${#missing[@]} -eq 0 ]] && return
  say "Installing libraries: ${missing[*]}"
  sudo pacman -S --needed --noconfirm "${missing[@]}"
}

install_app() {
  local version="$1" url tmp
  if [[ -n "$version" ]]; then
    url="https://github.com/$REPO/releases/download/v${version#v}/$ASSET"
  else
    url="https://github.com/$REPO/releases/latest/download/$ASSET"
  fi
  tmp="$(mktemp -d)"
  trap "rm -rf '$tmp'" EXIT
  say "Downloading Orchard Download Manager"
  curl -fL --progress-bar -o "$tmp/$ASSET" "$url" || die "download failed: $url (check the version and your connection)"
  tar xzf "$tmp/$ASSET" -C "$tmp"
  [[ -x "$tmp/orchard-download-manager/orchard-download-manager" ]] || die "the download is not an Orchard Download Manager package"

  mkdir -p "$(dirname "$PREFIX")" "$(dirname "$BIN")" "$APPS" "$ICONS"
  rm -rf "$PREFIX.new"
  mv "$tmp/orchard-download-manager" "$PREFIX.new"
  rm -rf "$PREFIX"
  mv "$PREFIX.new" "$PREFIX"
  ln -sf "$PREFIX/orchard-download-manager" "$BIN"

  curl -fsSL -o "$ICONS/orchard-download-manager.png" \
    "https://raw.githubusercontent.com/$REPO/main/packaging/linux/orchard-download-manager.png" || true
  cat >"$APPS/orchard-download-manager.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Orchard Download Manager
GenericName=Download manager
Comment=Fast, resumable downloads with a queue and a schedule
Exec=$PREFIX/orchard-download-manager %U
Icon=orchard-download-manager
Terminal=false
Categories=Network;FileTransfer;
Keywords=download;downloader;queue;resume;http;
EOF
  update-desktop-database "$APPS" 2>/dev/null || true
}

add_binding() {
  [[ -f "$BINDINGS" ]] || die "no $BINDINGS: is this Omarchy 4 or later?"
  if grep -qF -- "$MARK" "$BINDINGS"; then return; fi
  printf '\n%s\no.bind("SUPER + CTRL + SHIFT + D", "Orchard Download Manager", "uwsm-app -- orchard-download-manager")\n' \
    "$MARK" >>"$BINDINGS"
  hyprctl reload >/dev/null 2>&1 || true
}

free_f9() {
  [[ -f "$BINDINGS" ]] || die "no $BINDINGS: is this Omarchy 4 or later?"
  if grep -qF -- "$MARK_F9" "$BINDINGS"; then return; fi
  printf '\n%s\nhl.unbind("F9")\n' "$MARK_F9" >>"$BINDINGS"
  hyprctl reload >/dev/null 2>&1 || true
}

main() {
  local free=0 bind=0 version=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --bind) bind=1 ;;
      --free-f9) free=1 ;;
      --version) version="${2:?--version needs a number, for example 1.0.0}"; shift ;;
      --uninstall) uninstall; exit 0 ;;
      -h|--help) sed -n '2,15p' "${BASH_SOURCE[0]:-$0}" 2>/dev/null || true; exit 0 ;;
      *) die "unknown option: $1" ;;
    esac
    shift
  done

  [[ "$(uname -m)" == x86_64 ]] || die "Orchard Download Manager for Linux needs an x86-64 machine"
  command -v pacman >/dev/null || die "this installer is for Arch-based systems such as Omarchy"
  install_deps
  install_app "$version"
  [[ $bind -eq 1 ]] && add_binding
  [[ $free -eq 1 ]] && free_f9

  say "Installed. Start it from the launcher (Super+Space) or with: orchard-download-manager"
  [[ $bind -eq 1 ]] && echo "Key: Super+Ctrl+Shift+D"
  [[ $bind -eq 0 || $free -eq 0 ]] && echo "Options: --bind adds a key; --free-f9 frees F9 from Omarchy's dictation key."
  case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "Add ~/.local/bin to your PATH to run it from a terminal." ;; esac
}

main "$@"
