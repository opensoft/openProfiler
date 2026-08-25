#!/usr/bin/env bash
# Git extension: auto-commit.sh
# Automatically commit changes after a Spec Kit command completes.
# Checks per-command config keys in git-config.yml before committing.
#
# Usage: auto-commit.sh <event_name>
#   e.g.: auto-commit.sh after_specify

set -e

EVENT_NAME="${1:-}"
if [ -z "$EVENT_NAME" ]; then
    echo "Usage: $0 <event_name>" >&2
    exit 1
fi

SCRIPT_DIR="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

_find_project_root() {
    local dir="$1"
    while [ "$dir" != "/" ]; do
        if [ -d "$dir/.specify" ] || [ -d "$dir/.git" ]; then
            echo "$dir"
            return 0
        fi
        dir="$(dirname "$dir")"
    done
    return 1
}

REPO_ROOT=$(_find_project_root "$SCRIPT_DIR") || REPO_ROOT="$(pwd)"
cd "$REPO_ROOT"

# Check if git is available
if ! command -v git >/dev/null 2>&1; then
    echo "[specify] Warning: Git not found; skipped auto-commit" >&2
    exit 0
fi

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "[specify] Warning: Not a Git repository; skipped auto-commit" >&2
    exit 0
fi

# Read per-command config from git-config.yml
_config_file="$REPO_ROOT/.specify/extensions/git/git-config.yml"
_enabled=false
_commit_msg=""

_normalize_yaml_scalar() {
    local value="$1"

    value="${value%%[[:space:]]#*}"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    case "$value" in
        \"*\")
            value="${value#\"}"
            value="${value%\"}"
            ;;
        \'*\')
            value="${value#\'}"
            value="${value%\'}"
            ;;
    esac
    printf '%s\n' "$value"
}

if [ -f "$_config_file" ]; then
    # Parse the auto_commit section for this event.
    # Look for auto_commit.<event_name>.enabled and .message
    # Also check auto_commit.default as fallback.
    _in_auto_commit=false
    _in_event=false
    _default_enabled=false
    _event_seen=false
    _event_indent=0

    while IFS= read -r _line || [ -n "$_line" ]; do
        _trimmed="${_line#"${_line%%[![:space:]]*}"}"
        [ -n "$_trimmed" ] || continue
        case "$_trimmed" in
            \#*) continue ;;
        esac

        _indent="${_line%%[![:space:]]*}"
        _indent_len=${#_indent}
        _key="${_trimmed%%:*}"
        [ "$_trimmed" != "$_key" ] || continue
        _value="${_trimmed#*:}"
        _value="${_value#"${_value%%[![:space:]]*}"}"
        _value="$(_normalize_yaml_scalar "$_value")"

        # Detect auto_commit: section.
        if [ "$_indent_len" -eq 0 ] && [ "$_key" = "auto_commit" ]; then
            _in_auto_commit=true
            _in_event=false
            continue
        fi

        # Exit auto_commit section on any later top-level YAML key.
        if $_in_auto_commit && [ "$_indent_len" -eq 0 ]; then
            break
        fi

        if $_in_auto_commit; then
            if $_in_event && [ "$_indent_len" -le "$_event_indent" ] && [ "$_key" != "$EVENT_NAME" ]; then
                _in_event=false
            fi

            # Check default key.
            if [ "$_key" = "default" ]; then
                _val=$(printf '%s\n' "$_value" | tr -d '[:space:]' | tr '[:upper:]' '[:lower:]')
                [ "$_val" = "true" ] && _default_enabled=true
            fi

            # Detect our event subsection using a literal key comparison.
            if [ "$_key" = "$EVENT_NAME" ]; then
                _in_event=true
                _event_seen=true
                _event_indent=$_indent_len
                continue
            fi

            # Inside our event subsection.
            if $_in_event; then
                if [ "$_key" = "enabled" ]; then
                    _val=$(printf '%s\n' "$_value" | tr -d '[:space:]' | tr '[:upper:]' '[:lower:]')
                    [ "$_val" = "true" ] && _enabled=true
                    [ "$_val" = "false" ] && _enabled=false
                fi
                if [ "$_key" = "message" ]; then
                    _commit_msg="$_value"
                fi
            fi
        fi
    done < "$_config_file"

    # If event-specific key not found, use default
    if [ "$_enabled" = "false" ] && [ "$_default_enabled" = "true" ]; then
        # Only use default if the event wasn't explicitly set to false.
        if ! $_event_seen; then
            _enabled=true
        fi
    fi
else
    # No config file — auto-commit disabled by default
    exit 0
fi

if [ "$_enabled" != "true" ]; then
    exit 0
fi

# Check if there are changes to commit
if git diff --quiet HEAD 2>/dev/null && git diff --cached --quiet 2>/dev/null && [ -z "$(git ls-files --others --exclude-standard 2>/dev/null)" ]; then
    echo "[specify] No changes to commit after $EVENT_NAME" >&2
    exit 0
fi

# Derive a human-readable command name from the event
# e.g., after_specify -> specify, before_plan -> plan
_command_name=$(echo "$EVENT_NAME" | sed 's/^after_//' | sed 's/^before_//')
_phase=$(echo "$EVENT_NAME" | grep -q '^before_' && echo 'before' || echo 'after')

# Use custom message if configured, otherwise default
if [ -z "$_commit_msg" ]; then
    _commit_msg="[Spec Kit] Auto-commit ${_phase} ${_command_name}"
fi

# Stage and commit
_git_out=$(git add . 2>&1) || { echo "[specify] Error: git add failed: $_git_out" >&2; exit 1; }
_git_out=$(git commit -q -m "$_commit_msg" 2>&1) || { echo "[specify] Error: git commit failed: $_git_out" >&2; exit 1; }

echo "✓ Changes committed ${_phase} ${_command_name}" >&2
