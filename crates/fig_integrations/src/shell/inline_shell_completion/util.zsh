
#--------------------------------------------------------------------#
# Utility Functions                                                  #
#--------------------------------------------------------------------#

_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_escape_command() {
	setopt localoptions EXTENDED_GLOB

	# Escape special chars in the string (requires EXTENDED_GLOB)
	echo -E "${1//(#m)[\"\'\\()\[\]|*?~]/\\$MATCH}"
}
