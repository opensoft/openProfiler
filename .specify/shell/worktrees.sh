# Source this file from bash or zsh to enable Speckit worktree helpers.
#
# NOTE: In devBench containers, prefer the globally sourced ct helpers from
# /usr/local/share/ct/ct-functions.zsh.
# This file is kept as a per-repo fallback for environments without the
# container-level helpers.

# Auto-detect repo root from this script's own location so the file is
# relocatable and does not embed a host-specific absolute path.
if [ -n "${ZSH_VERSION:-}" ]; then
  SPECKIT_WORKTREE_REPO_ROOT="$(CDPATH="" cd "${${(%):-%x}:A:h}/../.." 2>/dev/null && pwd)"
else
  SPECKIT_WORKTREE_REPO_ROOT="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." 2>/dev/null && pwd)"
fi
SPECKIT_WORKTREE_LAST_WORKTREE_SCRIPT="$SPECKIT_WORKTREE_REPO_ROOT/.specify/extensions/git/scripts/bash/get-last-worktree.sh"
SPECKIT_WORKTREE_SELECT_WORKTREE_SCRIPT="$SPECKIT_WORKTREE_REPO_ROOT/.specify/shell/select-worktree.sh"

_speckit_worktree_prompt_cli() {
  local selection cli_name cli_command status

  while true; do
    printf '1. Anthropic' >&2
    if command -v claude >/dev/null 2>&1; then
      printf ' [available]\n' >&2
    else
      printf ' [not installed]\n' >&2
    fi

    printf '2. Codex' >&2
    if command -v codex >/dev/null 2>&1; then
      printf ' [available]\n' >&2
    else
      printf ' [not installed]\n' >&2
    fi

    printf '3. Gemini' >&2
    if command -v gemini >/dev/null 2>&1; then
      printf ' [available]\n' >&2
    else
      printf ' [not installed]\n' >&2
    fi

    printf 'Select AI CLI [1] (q to cancel): ' >&2
    if ! IFS= read -r selection; then
      echo >&2
      return 1
    fi

    selection="${selection#"${selection%%[![:space:]]*}"}"
    selection="${selection%"${selection##*[![:space:]]}"}"
    if [ -z "$selection" ]; then
      selection=1
    fi

    case "$selection" in
      q|Q)
        return 1
        ;;
      1)
        cli_name="Anthropic"
        cli_command="claude"
        ;;
      2)
        cli_name="Codex"
        cli_command="codex"
        ;;
      3)
        cli_name="Gemini"
        cli_command="gemini"
        ;;
      *)
        echo "Invalid selection. Enter 1-3 or q." >&2
        continue
        ;;
    esac

    if ! command -v "$cli_command" >/dev/null 2>&1; then
      echo "$cli_name CLI is not installed on PATH." >&2
      continue
    fi

    printf '%s\n' "$cli_command"
    return 0
  done
}

_speckit_worktree_select_worktree() {
  local target

  if [ ! -f "$SPECKIT_WORKTREE_SELECT_WORKTREE_SCRIPT" ]; then
    echo "worktree selector not found: $SPECKIT_WORKTREE_SELECT_WORKTREE_SCRIPT" >&2
    return 1
  fi

  target=$(bash "$SPECKIT_WORKTREE_SELECT_WORKTREE_SCRIPT" --path) || return 1
  if [ -z "$target" ]; then
    echo "no Speckit worktree selected" >&2
    return 1
  fi

  printf '%s\n' "$target"
}

_speckit_worktree_start_cli() {
  local cli_command="$1"
  local target="$2"
  shift 2 || true

  cd "$target" || return 1
  "$cli_command" "$@"
}

_speckit_worktree_start_claude() {
  local target="$1"
  shift || true

  _speckit_worktree_start_cli claude "$target" \
    --model opus \
    --dangerously-skip-permissions \
    --permission-mode bypassPermissions \
    --teammate-mode tmux \
    "$@"
}

_speckit_worktree_start_codex() {
  local target="$1"
  shift || true

  _speckit_worktree_start_cli codex "$target" \
    --dangerously-bypass-approvals-and-sandbox \
    -m gpt-5.4 \
    -c 'model_reasoning_effort="high"' \
    "$@"
}

_speckit_worktree_start_gemini() {
  local target="$1"
  shift || true

  _speckit_worktree_start_cli gemini "$target" \
    --yolo \
    --approval-mode yolo \
    --model gemini-2.5-pro \
    "$@"
}

ct() {
  local target

  if [ ! -f "$SPECKIT_WORKTREE_LAST_WORKTREE_SCRIPT" ]; then
    echo "ct: helper script not found: $SPECKIT_WORKTREE_LAST_WORKTREE_SCRIPT" >&2
    return 1
  fi

  target=$(bash "$SPECKIT_WORKTREE_LAST_WORKTREE_SCRIPT") || return 1
  if [ -z "$target" ]; then
    echo "ct: no Speckit worktree path returned" >&2
    return 1
  fi

  cd "$target" || return 1
}

ctp() {
  if [ ! -f "$SPECKIT_WORKTREE_LAST_WORKTREE_SCRIPT" ]; then
    echo "ctp: helper script not found: $SPECKIT_WORKTREE_LAST_WORKTREE_SCRIPT" >&2
    return 1
  fi

  bash "$SPECKIT_WORKTREE_LAST_WORKTREE_SCRIPT" --json
}

cta() {
  local target

  if ! command -v claude >/dev/null 2>&1; then
    echo "cta: Claude CLI not found on PATH" >&2
    return 1
  fi

  target=$(_speckit_worktree_select_worktree) || return 1
  _speckit_worktree_start_claude "$target" "$@"
}

ctc() {
  local target

  if ! command -v codex >/dev/null 2>&1; then
    echo "ctc: Codex CLI not found on PATH" >&2
    return 1
  fi

  target=$(_speckit_worktree_select_worktree) || return 1
  _speckit_worktree_start_codex "$target" "$@"
}

ctg() {
  local target

  if ! command -v gemini >/dev/null 2>&1; then
    echo "ctg: Gemini CLI not found on PATH" >&2
    return 1
  fi

  target=$(_speckit_worktree_select_worktree) || return 1
  _speckit_worktree_start_gemini "$target" "$@"
}

cts() {
  local target
  local cli_command

  target=$(_speckit_worktree_select_worktree) || return 1
  cli_command=$(_speckit_worktree_prompt_cli) || return 1
  case "$cli_command" in
    claude)
      _speckit_worktree_start_claude "$target" "$@"
      ;;
    codex)
      _speckit_worktree_start_codex "$target" "$@"
      ;;
    gemini)
      _speckit_worktree_start_gemini "$target" "$@"
      ;;
    *)
      echo "cts: unsupported CLI selection: $cli_command" >&2
      return 1
      ;;
  esac
}

ctlist() {
  if [ ! -f "$SPECKIT_WORKTREE_SELECT_WORKTREE_SCRIPT" ]; then
    echo "ctlist: helper script not found: $SPECKIT_WORKTREE_SELECT_WORKTREE_SCRIPT" >&2
    return 1
  fi

  bash "$SPECKIT_WORKTREE_SELECT_WORKTREE_SCRIPT" --list
}
