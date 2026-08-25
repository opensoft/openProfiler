# Source this file from zsh to enable Speckit worktree helpers.
#
# Example:
#   source .specify/shell/ct.zsh

_speckit_ct_dir="${${(%):-%x}:A:h}"
source "$_speckit_ct_dir/worktrees.sh"
unset _speckit_ct_dir
