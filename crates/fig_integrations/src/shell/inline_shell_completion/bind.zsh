
#--------------------------------------------------------------------#
# Widget Helpers                                                     #
#--------------------------------------------------------------------#

_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_incr_bind_count() {
	typeset -gi bind_count=$((_{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_BIND_COUNTS[$1]+1))
	_{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_BIND_COUNTS[$1]=$bind_count
}

# Bind a single widget to an autosuggest widget, saving a reference to the original widget
_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bind_widget() {
	typeset -gA _{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_BIND_COUNTS

	local widget=$1
	local autosuggest_action=$2
	local prefix=${{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_ORIGINAL_WIDGET_PREFIX

	local -i bind_count

	# Save a reference to the original widget
	case $widgets[$widget] in
		# Already bound
		user:_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_(bound|orig)_*)
			bind_count=$((_{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_BIND_COUNTS[$widget]))
			;;

		# User-defined widget
		user:*)
			_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_incr_bind_count $widget
			zle -N $prefix$bind_count-$widget ${widgets[$widget]#*:}
			;;

		# Built-in widget
		builtin)
			_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_incr_bind_count $widget
			eval "_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_orig_${(q)widget}() { zle .${(q)widget} }"
			zle -N $prefix$bind_count-$widget _{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_orig_$widget
			;;

		# Completion widget
		completion:*)
			_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_incr_bind_count $widget
			eval "zle -C $prefix$bind_count-${(q)widget} ${${(s.:.)widgets[$widget]}[2,3]}"
			;;
	esac

	# Pass the original widget's name explicitly into the autosuggest
	# function. Use this passed in widget name to call the original
	# widget instead of relying on the $WIDGET variable being set
	# correctly. $WIDGET cannot be trusted because other plugins call
	# zle without the `-w` flag (e.g. `zle self-insert` instead of
	# `zle self-insert -w`).
	eval "_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bound_${bind_count}_${(q)widget}() {
		_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_widget_$autosuggest_action $prefix$bind_count-${(q)widget} \$@
	}"

	# Create the bound widget
	zle -N -- $widget _{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bound_${bind_count}_$widget
}

# Map all configured widgets to the right autosuggest widgets
_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bind_widgets() {
	emulate -L zsh

 	local widget
	local ignore_widgets

	ignore_widgets=(
		.\*
		_\*
		${_{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_BUILTIN_ACTIONS/#/autosuggest-}
		${{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_ORIGINAL_WIDGET_PREFIX\*
		${{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_IGNORE_WIDGETS
	)

	# Find every widget we might want to bind and bind it appropriately
	for widget in ${${(f)"$(builtin zle -la)"}:#${(j:|:)~ignore_widgets}}; do
		if [[ -n ${{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_CLEAR_WIDGETS[(r)$widget]} ]]; then
			_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bind_widget $widget clear
		elif [[ -n ${{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_ACCEPT_WIDGETS[(r)$widget]} ]]; then
			_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bind_widget $widget accept
		elif [[ -n ${{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_EXECUTE_WIDGETS[(r)$widget]} ]]; then
			_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bind_widget $widget execute
		elif [[ -n ${{{CLI_BINARY_NAME_UPPER}}_AUTOSUGGEST_PARTIAL_ACCEPT_WIDGETS[(r)$widget]} ]]; then
			_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bind_widget $widget partial_accept
		else
			# Assume any unspecified widget might modify the buffer
			_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_bind_widget $widget modify
		fi
	done
}

# Given the name of an original widget and args, invoke it, if it exists
_{{CLI_BINARY_NAME_UNDERSCORE}}_autosuggest_invoke_original_widget() {
	# Do nothing unless called with at least one arg
	(( $# )) || return 0

	local original_widget_name="$1"

	shift

	if (( ${+widgets[$original_widget_name]} )); then
		zle $original_widget_name -- $@
	fi
}
