
#--------------------------------------------------------------------#
# Fetch Suggestion                                                   #
#--------------------------------------------------------------------#
# Loops through all specified strategies and returns a suggestion
# from the first strategy to provide one.
#

_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_fetch_suggestion() {
	typeset -g suggestion
	local -a strategies
	local strategy

	# Ensure we are working with an array
	strategies=(${={{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_STRATEGY})

	for strategy in $strategies; do
		# Try to get a suggestion from this strategy
		_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_strategy_$strategy "$1"

		# Ensure the suggestion matches the prefix
		[[ "$suggestion" != "$1"* ]] && unset suggestion

		# Break once we've found a valid suggestion
		[[ -n "$suggestion" ]] && break
	done
}
