
#--------------------------------------------------------------------#
# Start                                                              #
#--------------------------------------------------------------------#

# Start the autosuggestion widgets
_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_start() {
	# By default we re-bind widgets on every precmd to ensure we wrap other
	# wrappers. Specifically, highlighting breaks if our widgets are wrapped by
	# zsh-syntax-highlighting widgets. This also allows modifications to the
	# widget list variables to take effect on the next precmd. However this has
	# a decent performance hit, so users can set {{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_MANUAL_REBIND
	# to disable the automatic re-binding.
	if (( ${+{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_MANUAL_REBIND} )); then
		add-zsh-hook -d precmd _{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_start
	fi

	_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bind_widgets
}

# Mark for auto-loading the functions that we use
autoload -Uz add-zsh-hook is-at-least

# Automatically enable asynchronous mode in newer versions of zsh. Disable for
# older versions because there is a bug when using async mode where ^C does not
# work immediately after fetching a suggestion.
# See https://github.com/zsh-users/zsh-autosuggestions/issues/364
if is-at-least 5.0.8; then
	typeset -g {{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_USE_ASYNC=
fi

# Start the autosuggestion widgets on the next precmd
add-zsh-hook precmd _{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_start
