
#--------------------------------------------------------------------#
# InlineShell Suggestion Strategy                                          #
#--------------------------------------------------------------------#
# Suggests the inline_shell_completion command.
#

_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_strategy_inline_shell_completion() {
	typeset -g suggestion="$(command -v {{CLI_BINARY_NAME}} >/dev/null 2>&1 && {{CLI_BINARY_NAME}} _ inline-shell-completion --buffer "${BUFFER}")"
}
