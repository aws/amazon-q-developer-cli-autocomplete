# Trusted Commands

The Trusted Commands feature in Amazon Q Developer CLI allows you to define shell commands that can be executed without requiring explicit user confirmation each time. This feature enhances your workflow by reducing interruptions for frequently used, safe commands while maintaining security for potentially dangerous operations.

## Overview

By default, Amazon Q Developer CLI has a built-in list of read-only commands (like `ls`, `cat`, `echo`, `pwd`) that are considered safe to execute without confirmation. The Trusted Commands feature extends this by allowing you to add your own commands to this trusted list.

## How It Works

When Amazon Q Developer CLI needs to execute a shell command, it follows this security check order:

1. **Dangerous Pattern Check**: Commands containing potentially dangerous patterns (like `>`, `&&`, `||`, `$(`, etc.) always require confirmation, regardless of trusted status
2. **Built-in Safe Commands**: Commands in the predefined read-only list (`ls`, `cat`, `echo`, etc.) are executed without confirmation
3. **User-defined Trusted Commands**: Commands matching your trusted patterns are executed without confirmation
4. **Default Behavior**: All other commands require user confirmation

## Managing Trusted Commands

### Adding Trusted Commands

You can add trusted commands using the `/tools allow` command:

```bash
/tools allow execute_bash --command "npm *"
/tools allow execute_bash --command "git status"
/tools allow execute_bash --command "docker ps"
```

### Viewing Trusted Commands

Use the `/tools` command to see all your trusted commands:

```bash
/tools
```

This will display output similar to:
```
execute_bash: Trusted Commands: "npm *" "git status" "docker ps"
```

### Interactive Rule Creation

When Amazon Q Developer CLI prompts you to confirm a command execution, you can press 'c' to create a trusted command rule interactively. This option is only available for `execute_bash` and `execute_cmd` tools.

For example, if you're prompted to confirm:
```
execute_bash (command=git restore --staged Makefile frontend/ opentofu/)
```

Pressing 'c' will show you options like:
```
Create rule for: execute_bash (command=git restore --staged Makefile frontend/ opentofu/)
Trusted commands do not ask for confirmation before running.

1. Trust this exact command only
2. Trust all 'git restore' commands (up to first argument)
3. Trust all 'git' commands (first word only)
4. Run the command without adding a rule
5. Exit rule creation and don't run any commands

Choose an option (1-5):
```

Options 1-3 create trusted command rules with different pattern scopes. Option 4 runs the current command once without creating any rule. Option 5 cancels both rule creation and command execution, returning you to the chat prompt.

## Command Pattern Syntax

Trusted commands support glob-style pattern matching:

- **Exact Match**: `git status` - matches only the exact command
- **Wildcard Match**: `npm *` - matches any command starting with `npm `
- **Complex Patterns**: `git restore *` - matches any `git restore` command with any arguments

### Pattern Examples

| Pattern | Matches | Doesn't Match |
|---------|---------|---------------|
| `npm *` | `npm install`, `npm run build`, `npm test` | `npx create-react-app`, `yarn install` |
| `git status` | `git status` (exact) | `git status --short`, `git log` |
| `docker ps *` | `docker ps`, `docker ps -a`, `docker ps --all` | `docker run`, `docker images` |
| `ls *` | `ls`, `ls -la`, `ls /home` | `ll`, `dir` |

## Configuration Storage

Trusted commands are stored in your profile's `context.json` file located at:
```
~/.aws/amazonq/profiles/<profile_name>/context.json
```

The configuration structure looks like:
```json
{
  "trusted_commands": [
    {
      "command": "npm *",
      "description": "All npm commands"
    },
    {
      "command": "git status",
      "description": "Git status command only"
    }
  ]
}
```

## Security Considerations

### Important Security Notes

1. **Dangerous Patterns Override Trust**: Even if a command is in your trusted list, it will still require confirmation if it contains dangerous patterns like:
   - Redirections: `>`, `>>`, `<`
   - Command chaining: `&&`, `||`, `;`
   - Command substitution: `$(...)`, `` `...` ``
   - Background execution: `&`
   - Process substitution: `<(...)`

2. **Wildcard Caution**: Be careful with wildcard patterns. `git *` would trust ALL git commands, including potentially dangerous ones like `git reset --hard HEAD~10`.

3. **Regular Review**: Periodically review your trusted commands to ensure they still align with your security requirements.

### Best Practices

1. **Start Specific**: Begin with exact command matches before using wildcards
2. **Incremental Trust**: Add commands to your trusted list gradually as you encounter them
3. **Avoid Overly Broad Patterns**: Prefer `git status *` over `git *` unless you're certain about all git subcommands
4. **Use Descriptions**: Add meaningful descriptions to help you remember why you trusted each command

### Safe Pattern Examples

✅ **Good patterns** (specific and safe):
```bash
/tools allow execute_bash --command "npm test"
/tools allow execute_bash --command "git status"
/tools allow execute_bash --command "docker ps"
/tools allow execute_bash --command "kubectl get pods"
```

⚠️ **Use with caution** (broad but potentially useful):
```bash
/tools allow execute_bash --command "npm *"
/tools allow execute_bash --command "git log *"
/tools allow execute_bash --command "docker ps *"
```

❌ **Avoid** (too broad and potentially dangerous):
```bash
/tools allow execute_bash --command "git *"
/tools allow execute_bash --command "docker *"
/tools allow execute_bash --command "sudo *"
```

## Interactive Rule Creation Details

The interactive rule creation feature (pressing 'c' during command confirmation) is only available for:
- `execute_bash` tool
- `execute_cmd` tool

It is **not** available for other tools like:
- `fs_write` (file operations)
- `use_aws` (AWS API calls)
- Custom tools

When you press 'c', the system analyzes your command and offers three pattern options:

1. **Exact Command**: Trusts only the specific command with all its arguments
2. **Up to First Argument**: Creates a pattern that includes the main command and subcommand, then uses `*` for arguments
3. **First Word Only**: Creates a pattern with just the main command followed by `*`

## Troubleshooting

### Common Issues

**Q: My trusted command still asks for confirmation**
A: Check if your command contains dangerous patterns. Even trusted commands require confirmation if they contain redirections, command chaining, or other potentially dangerous elements.

**Q: The 'c' option doesn't appear when I'm prompted**
A: The 'c' option only appears for `execute_bash` and `execute_cmd` tools, and only when a context manager is available. Other tools will only show 'y' and 'n' options.

**Q: My pattern doesn't match the command I expected**
A: Remember that patterns are case-sensitive and use glob-style matching. `npm *` matches `npm install` but not `NPM install` or `Npm install`.

**Q: How do I remove a trusted command?**
A: You can use the `/tools remove execute_bash`commands. To remove all commands type `/tolls remove execute_bash --all`. You can also manually edit your profile's `context.json` file to remove trusted commands. Look for the file at `~/.aws/amazonq/profiles/<profile_name>/context.json`.

### Error Handling

If there are issues with your trusted commands configuration:
- Invalid JSON in the configuration file will be logged as an error, and the system will fall back to default behavior
- File read/write errors are handled gracefully without crashing the CLI
- Malformed patterns are validated before being added to the trusted list

## Examples

### Development Workflow

For a typical development workflow, you might want to trust these commands:

```bash
# Package management
/tools allow execute_bash --command "npm install"
/tools allow execute_bash --command "npm test"
/tools allow execute_bash --command "yarn install"

# Git operations (read-only)
/tools allow execute_bash --command "git status"
/tools allow execute_bash --command "git log *"
/tools allow execute_bash --command "git diff *"

# Docker inspection
/tools allow execute_bash --command "docker ps *"
/tools allow execute_bash --command "docker images *"
/tools allow execute_bash --command "docker logs *"

# Kubernetes inspection
/tools allow execute_bash --command "kubectl get *"
/tools allow execute_bash --command "kubectl describe *"
```

### DevOps Workflow

For DevOps tasks, you might trust:

```bash
# Infrastructure inspection
/tools allow execute_bash --command "terraform plan"
/tools allow execute_bash --command "terraform show *"
/tools allow execute_bash --command "aws sts get-caller-identity"

# Service status
/tools allow execute_bash --command "systemctl status *"
/tools allow execute_bash --command "service * status"
```

Remember: Always start with specific commands and gradually expand to patterns as you become comfortable with the feature.