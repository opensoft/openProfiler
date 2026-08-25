#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(CDPATH="" cd "$SCRIPT_DIR/../.." && pwd)"

trim() {
    local value="$1"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    printf '%s' "$value"
}

normalize_config_value() {
    local value="$1"

    value="${value%%[[:space:]]#*}"
    value="$(trim "$value")"
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
    printf '%s' "$value"
}

resolve_config_value() {
    local repo_root="$1"
    local key="$2"
    local default_value="$3"
    local config_file="$repo_root/.specify/extensions/git/git-config.yml"

    if [ ! -f "$config_file" ]; then
        printf '%s\n' "$default_value"
        return 0
    fi

    local raw_value
    raw_value=$(awk -F':' -v key="$key" '$1 == key {sub(/^[^:]*:[[:space:]]*/, "", $0); print $0; exit}' "$config_file")
    raw_value=$(normalize_config_value "${raw_value:-}")
    if [ -z "$raw_value" ]; then
        printf '%s\n' "$default_value"
    else
        printf '%s\n' "$raw_value"
    fi
}

mtime_for_path() {
    if stat -c %Y "$1" >/dev/null 2>&1; then
        stat -c %Y "$1"
    else
        stat -f %m "$1"
    fi
}

list_worktree_dirs_by_mtime() {
    local root="$1"
    local path mtime

    find "$root" -maxdepth 1 -mindepth 1 -type d -print 2>/dev/null |
        while IFS= read -r path; do
            mtime="$(mtime_for_path "$path" 2>/dev/null || true)"
            [ -n "$mtime" ] || continue
            printf '%s\t%s\n' "$mtime" "$path"
        done |
        sort -rn |
        sed -n $'s/^[^\t]*\t//p'
}

resolve_path_from_root() {
    local repo_root="$1"
    local raw_path="$2"
    local combined

    if [[ "$raw_path" = /* ]]; then
        combined="$raw_path"
    else
        combined="$repo_root/$raw_path"
    fi

    local parent_dir
    parent_dir=$(dirname "$combined")
    local leaf_name
    leaf_name=$(basename "$combined")
    if [ -d "$parent_dir" ]; then
        printf '%s/%s\n' "$(cd "$parent_dir" && pwd -P)" "$leaf_name"
    else
        printf '%s\n' "$combined"
    fi
}

resolve_main_repo_root() {
    local repo_root="$1"

    local inferred_root
    case "$repo_root" in
        */.worktrees/*)
            inferred_root="${repo_root%%/.worktrees/*}"
            if [ -d "$inferred_root/.specify" ]; then
                (cd "$inferred_root" && pwd -P)
                return 0
            fi
            ;;
        */worktrees/*)
            inferred_root="${repo_root%%/worktrees/*}"
            if [ -d "$inferred_root/.specify" ]; then
                (cd "$inferred_root" && pwd -P)
                return 0
            fi
            ;;
    esac

    local worktree_parent
    worktree_parent=$(dirname "$repo_root")
    local worktree_parent_name
    worktree_parent_name=$(basename "$worktree_parent")
    if [[ "$worktree_parent_name" == *-worktrees ]]; then
        inferred_root="$(dirname "$worktree_parent")/${worktree_parent_name%-worktrees}"
        if [ -d "$inferred_root/.specify" ]; then
            (cd "$inferred_root" && pwd -P)
            return 0
        fi
    fi

    local common_dir
    common_dir=$(git -C "$repo_root" rev-parse --git-common-dir 2>/dev/null || true)

    if [ -z "$common_dir" ]; then
        printf '%s\n' "$repo_root"
        return 0
    fi

    if [[ "$common_dir" != /* ]]; then
        common_dir="$repo_root/$common_dir"
    fi

    if [ "$(basename "$common_dir")" = ".git" ]; then
        local main_root
        main_root=$(dirname "$common_dir")
        if [ -d "$main_root" ]; then
            (cd "$main_root" && pwd -P)
            return 0
        fi
    fi

    printf '%s\n' "$repo_root"
}

worktree_root_has_entries() {
    local candidate="$1"
    local first

    [ -d "$candidate" ] || return 1
    first=$(find "$candidate" -maxdepth 1 -mindepth 1 -type d -print -quit 2>/dev/null || true)
    [ -n "$first" ]
}

resolve_worktree_root() {
    local repo_root="$1"
    local main_root
    main_root=$(resolve_main_repo_root "$repo_root")

    local default_root="../$(basename "$main_root")-worktrees"
    local raw_root
    raw_root=$(resolve_config_value "$main_root" "worktree_root" "$default_root")

    local candidate
    candidate=$(resolve_path_from_root "$main_root" "$raw_root")
    if [ -d "$candidate" ]; then
        (cd "$candidate" && pwd -P)
        return 0
    fi

    if [ "$raw_root" != "$default_root" ]; then
        printf '%s\n' "$candidate"
        return 0
    fi

    for candidate in "$(resolve_path_from_root "$main_root" "worktrees")" "$(resolve_path_from_root "$main_root" ".worktrees")"; do
        if worktree_root_has_entries "$candidate"; then
            (cd "$candidate" && pwd -P)
            return 0
        fi
    done

    printf '%s\n' "$(resolve_path_from_root "$main_root" "$default_root")"
}

MODE="prompt"
if [ "${1:-}" = "--path" ]; then
    MODE="path"
elif [ "${1:-}" = "--list" ]; then
    MODE="list"
elif [ $# -gt 0 ]; then
    echo "Usage: $0 [--path|--list]" >&2
    exit 1
fi

WORKTREE_ROOT=$(resolve_worktree_root "$REPO_ROOT")
if [ ! -d "$WORKTREE_ROOT" ]; then
    echo "No Speckit worktree root found at: $WORKTREE_ROOT" >&2
    exit 1
fi

PATHS=()
BRANCHES=()
while IFS= read -r worktree_path; do
    [ -n "$worktree_path" ] || continue
    if [ -d "$worktree_path" ]; then
        PATHS+=("$(cd "$worktree_path" && pwd -P)")
        BRANCHES+=("$(basename "$worktree_path")")
    fi
done < <(list_worktree_dirs_by_mtime "$WORKTREE_ROOT")

if [ "${#PATHS[@]}" -eq 0 ]; then
    echo "No Speckit worktrees found under: $WORKTREE_ROOT" >&2
    exit 1
fi

if [ "${#PATHS[@]}" -eq 0 ]; then
    echo "No usable Speckit worktrees found under: $WORKTREE_ROOT" >&2
    exit 1
fi

if [ "$MODE" = "list" ]; then
    for i in "${!PATHS[@]}"; do
        index=$((i + 1))
        marker=""
        if [ "$i" -eq 0 ]; then
            marker=" [default]"
        fi
        printf '%d. %s%s\n' "$index" "${BRANCHES[$i]}" "$marker"
        printf '   %s\n' "${PATHS[$i]}"
    done
    exit 0
fi

if [ "$MODE" = "path" ] && ! [ -t 0 ]; then
    printf '%s\n' "${PATHS[0]}"
    exit 0
fi

for i in "${!PATHS[@]}"; do
    index=$((i + 1))
    marker=""
    if [ "$i" -eq 0 ]; then
        marker=" [default]"
    fi
    printf '%d. %s%s\n' "$index" "${BRANCHES[$i]}" "$marker" >&2
    printf '   %s\n' "${PATHS[$i]}" >&2
done

while true; do
    printf 'Select worktree [1] (q to cancel): ' >&2
    if ! IFS= read -r selection; then
        echo >&2
        exit 1
    fi

    selection=$(trim "$selection")
    if [ -z "$selection" ]; then
        selection=1
    fi

    case "$selection" in
        q|Q)
            exit 1
            ;;
    esac

    if [[ "$selection" =~ ^[0-9]+$ ]] && [ "$selection" -ge 1 ] && [ "$selection" -le "${#PATHS[@]}" ]; then
        printf '%s\n' "${PATHS[$((selection - 1))]}"
        exit 0
    fi

    echo "Invalid selection. Enter 1-${#PATHS[@]} or q." >&2
done
