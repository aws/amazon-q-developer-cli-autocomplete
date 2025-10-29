
#--------------------------------------------------------------------#
# Highlighting                                                       #
#--------------------------------------------------------------------#

# If there was a highlight, remove it
_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_highlight_reset() {
	typeset -g _{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_LAST_HIGHLIGHT

	if [[ -n "$_{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_LAST_HIGHLIGHT" ]]; then
		region_highlight=("${(@)region_highlight:#$_{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_LAST_HIGHLIGHT}")
		unset _{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_LAST_HIGHLIGHT
	fi
}

# If there's a suggestion, highlight it
_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_highlight_apply() {
	typeset -g _{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_LAST_HIGHLIGHT

	if (( $#POSTDISPLAY )); then
		typeset -g _{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_LAST_HIGHLIGHT="$#BUFFER $(($#BUFFER + $#POSTDISPLAY)) ${{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_HIGHLIGHT_STYLE"
		region_highlight+=("$_{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_LAST_HIGHLIGHT")
	else
		unset _{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_LAST_HIGHLIGHT
	fi
}
