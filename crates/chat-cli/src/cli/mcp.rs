use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{
    Args,
    ValueEnum,
};
use crossterm::{
    execute,
    style,
};
use eyre::{
    Result,
    bail,
};
use tracing::warn;

use crate::cli::chat::tool_manager::{
    McpServerConfig,
    global_mcp_config_path,
    workspace_mcp_config_path,
};
use crate::cli::chat::tools::custom_tool::{
    CustomToolConfig,
    default_timeout,
};
use crate::cli::chat::util::shared_writer::SharedWriter;
use crate::platform::Context;

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum Scope {
    Workspace,
    Global,
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scope::Workspace => write!(f, "workspace"),
            Scope::Global => write!(f, "global"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Subcommand)]
pub enum McpSubcommand {
    /// Add or replace a configured server
    Add(AddArgs),
    /// Remove a server from the MCP configuration
    #[command(alias = "rm")]
    Remove(RemoveArgs),
    /// List configured servers
    List(ListArgs),
    /// Import a server configuration from another file
    Import(ImportArgs),
    /// Get the status of a configured server
    Status(StatusArgs),
}

impl McpSubcommand {
    pub async fn execute(self) -> Result<ExitCode> {
        let ctx = Context::new();
        let mut output = SharedWriter::stdout();

        match self {
            Self::Add(args) => args.execute(&ctx, &mut output).await?,
            Self::Remove(args) => args.execute(&ctx, &mut output).await?,
            Self::List(args) => args.execute(&ctx, &mut output).await?,
            Self::Import(args) => args.execute(&ctx, &mut output).await?,
            Self::Status(args) => args.execute(&ctx, &mut output).await?,
        }

        output.flush()?;
        Ok(ExitCode::SUCCESS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct AddArgs {
    /// Name for the server
    #[arg(long)]
    pub name: String,
    /// The command used to launch the server
    #[arg(long)]
    pub command: String,
    /// Where to add the server to.
    #[arg(long, value_enum)]
    pub scope: Option<Scope>,
    /// Environment variables to use when launching the server
    #[arg(long, value_parser = parse_env_vars)]
    pub env: Vec<HashMap<String, String>>,
    /// Server launch timeout, in milliseconds
    #[arg(long)]
    pub timeout: Option<u64>,
    /// Overwrite an existing server with the same name
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

impl AddArgs {
    pub async fn execute(self, ctx: &Context, output: &mut SharedWriter) -> Result<()> {
        let scope = self.scope.unwrap_or(Scope::Workspace);
        let config_path = resolve_scope_profile(ctx, self.scope)?;

        let mut config: McpServerConfig = ensure_config_file(ctx, &config_path, output).await?;

        if config.mcp_servers.contains_key(&self.name) && !self.force {
            bail!(
                "\nMCP server '{}' already exists in {} (scope {}). Use --force to overwrite.",
                self.name,
                config_path.display(),
                scope
            );
        }

        let merged_env = self.env.into_iter().flatten().collect::<HashMap<_, _>>();
        let tool: CustomToolConfig = serde_json::from_value(serde_json::json!({
            "command": self.command,
            "env": merged_env,
            "timeout": self.timeout.unwrap_or(default_timeout()),
        }))?;

        writeln!(
            output,
            "\nTo learn more about MCP safety, see https://docs.aws.amazon.com/amazonq/latest/qdeveloper-ug/command-line-mcp-security.html\n\n"
        )?;

        config.mcp_servers.insert(self.name.clone(), tool);
        config.save_to_file(ctx, &config_path).await?;
        writeln!(
            output,
            "✓ Added MCP server '{}' to {}\n",
            self.name,
            scope_display(&scope)
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct RemoveArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long, value_enum)]
    pub scope: Option<Scope>,
}

impl RemoveArgs {
    pub async fn execute(self, ctx: &Context, output: &mut SharedWriter) -> Result<()> {
        let scope = self.scope.unwrap_or(Scope::Workspace);
        let config_path = resolve_scope_profile(ctx, self.scope)?;

        if !ctx.fs().exists(&config_path) {
            writeln!(output, "\nNo MCP server configurations found.\n")?;
            return Ok(());
        }

        let mut config = McpServerConfig::load_from_file(ctx, &config_path).await?;
        match config.mcp_servers.remove(&self.name) {
            Some(_) => {
                config.save_to_file(ctx, &config_path).await?;
                writeln!(
                    output,
                    "\n✓ Removed MCP server '{}' from {}\n",
                    self.name,
                    scope_display(&scope)
                )?;
            },
            None => {
                writeln!(
                    output,
                    "\nNo MCP server named '{}' found in {}\n",
                    self.name,
                    scope_display(&scope)
                )?;
            },
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ListArgs {
    #[arg(value_enum)]
    pub scope: Option<Scope>,
    #[arg(long, hide = true)]
    pub profile: Option<String>,
}

impl ListArgs {
    pub async fn execute(self, ctx: &Context, output: &mut SharedWriter) -> Result<()> {
        let configs = get_mcp_server_configs(ctx, self.scope).await?;
        if configs.is_empty() {
            writeln!(output, "No MCP server configurations found.\n")?;
            return Ok(());
        }

        for (scope, path, cfg_opt) in configs {
            writeln!(output)?;
            writeln!(output, "{}:\n  {}", scope_display(&scope), path.display())?;
            match cfg_opt {
                Some(cfg) if !cfg.mcp_servers.is_empty() => {
                    for (name, tool_cfg) in &cfg.mcp_servers {
                        writeln!(output, "    • {name:<12} {}", tool_cfg.command)?;
                    }
                },
                _ => {
                    writeln!(output, "    (empty)")?;
                },
            }
        }
        writeln!(output, "\n")?;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ImportArgs {
    #[arg(long)]
    pub file: String,
    #[arg(value_enum)]
    pub scope: Option<Scope>,
    /// Overwrite an existing server with the same name
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

impl ImportArgs {
    pub async fn execute(self, ctx: &Context, output: &mut SharedWriter) -> Result<()> {
        let scope: Scope = self.scope.unwrap_or(Scope::Workspace);
        let config_path = resolve_scope_profile(ctx, self.scope)?;
        let mut dst_cfg = ensure_config_file(ctx, &config_path, output).await?;

        let src_path = expand_path(ctx, &self.file)?;
        let src_cfg: McpServerConfig = McpServerConfig::load_from_file(ctx, &src_path).await?;

        let mut added = 0;
        for (name, cfg) in src_cfg.mcp_servers {
            if dst_cfg.mcp_servers.contains_key(&name) && !self.force {
                bail!(
                    "\nMCP server '{}' already exists in {} (scope {}). Use --force to overwrite.\n",
                    name,
                    config_path.display(),
                    scope
                );
            }
            dst_cfg.mcp_servers.insert(name.clone(), cfg);
            added += 1;
        }

        writeln!(
            output,
            "\nTo learn more about MCP safety, see https://docs.aws.amazon.com/amazonq/latest/qdeveloper-ug/command-line-mcp-security.html\n\n"
        )?;

        dst_cfg.save_to_file(ctx, &config_path).await?;
        writeln!(
            output,
            "✓ Imported {added} MCP server(s) into {}\n",
            scope_display(&scope)
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct StatusArgs {
    #[arg(long)]
    pub name: String,
}

impl StatusArgs {
    pub async fn execute(self, ctx: &Context, output: &mut SharedWriter) -> Result<()> {
        let configs = get_mcp_server_configs(ctx, None).await?;
        let mut found = false;

        for (sc, path, cfg_opt) in configs {
            if let Some(cfg) = cfg_opt.and_then(|c| c.mcp_servers.get(&self.name).cloned()) {
                found = true;
                execute!(
                    output,
                    style::Print("\n─────────────\n"),
                    style::Print(format!("Scope   : {}\n", scope_display(&sc))),
                    style::Print(format!("File    : {}\n", path.display())),
                    style::Print(format!("Command : {}\n", cfg.command)),
                    style::Print(format!("Timeout : {} ms\n", cfg.timeout)),
                    style::Print(format!(
                        "Env Vars: {}\n",
                        cfg.env
                            .as_ref()
                            .map_or_else(|| "(none)".into(), |e| e.keys().cloned().collect::<Vec<_>>().join(", "))
                    )),
                )?;
            }
        }
        writeln!(output, "\n")?;

        if !found {
            bail!("No MCP server named '{}' found in any scope/profile\n", self.name);
        }

        Ok(())
    }
}

async fn get_mcp_server_configs(
    ctx: &Context,
    scope: Option<Scope>,
) -> Result<Vec<(Scope, PathBuf, Option<McpServerConfig>)>> {
    let mut targets = Vec::new();
    match scope {
        Some(scope) => targets.push(scope),
        None => targets.extend([Scope::Workspace, Scope::Global]),
    }

    let mut results = Vec::new();
    for sc in targets {
        let path = resolve_scope_profile(ctx, Some(sc))?;
        let cfg_opt = if ctx.fs().exists(&path) {
            match McpServerConfig::load_from_file(ctx, &path).await {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    warn!(?path, error = %e, "Invalid MCP config file—ignored, treated as null");
                    None
                },
            }
        } else {
            None
        };
        results.push((sc, path, cfg_opt));
    }
    Ok(results)
}

fn scope_display(scope: &Scope) -> String {
    match scope {
        Scope::Workspace => "📄 workspace".into(),
        Scope::Global => "🌍 global".into(),
    }
}

fn resolve_scope_profile(ctx: &Context, scope: Option<Scope>) -> Result<PathBuf> {
    Ok(match scope {
        Some(Scope::Global) => global_mcp_config_path(ctx)?,
        _ => workspace_mcp_config_path(ctx)?,
    })
}

fn expand_path(ctx: &Context, p: &str) -> Result<PathBuf> {
    let p = shellexpand::tilde(p);
    let mut path = PathBuf::from(p.as_ref() as &str);
    if path.is_relative() {
        path = ctx.env().current_dir()?.join(path);
    }
    Ok(path)
}

async fn ensure_config_file(ctx: &Context, path: &PathBuf, out: &mut SharedWriter) -> Result<McpServerConfig> {
    if !ctx.fs().exists(path) {
        if let Some(parent) = path.parent() {
            ctx.fs().create_dir_all(parent).await?;
        }
        McpServerConfig::default().save_to_file(ctx, path).await?;
        writeln!(out, "\n📁 Created MCP config in '{}'", path.display())?;
    }

    load_cfg(ctx, path).await
}

fn parse_env_vars(arg: &str) -> Result<HashMap<String, String>> {
    let mut vars = HashMap::new();

    for pair in arg.split(",") {
        match pair.split_once('=') {
            Some((key, value)) => {
                vars.insert(key.trim().to_string(), value.trim().to_string());
            },
            None => {
                bail!(
                    "Failed to parse environment variables, invalid environment variable '{}'. Expected 'name=value'",
                    pair
                )
            },
        }
    }

    Ok(vars)
}

async fn load_cfg(ctx: &Context, p: &PathBuf) -> Result<McpServerConfig> {
    Ok(if ctx.fs().exists(p) {
        McpServerConfig::load_from_file(ctx, p).await?
    } else {
        McpServerConfig::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::RootSubcommand;
    use crate::util::test::assert_parse;

    #[test]
    fn test_scope_and_profile_defaults_to_workspace() {
        let ctx = Context::new();
        let path = resolve_scope_profile(&ctx, None).unwrap();
        assert_eq!(
            path.to_str(),
            workspace_mcp_config_path(&ctx).unwrap().to_str(),
            "No scope or profile should default to the workspace path"
        );
    }

    #[test]
    fn test_resolve_paths() {
        let ctx = Context::new();
        // workspace
        let p = resolve_scope_profile(&ctx, Some(Scope::Workspace)).unwrap();
        assert_eq!(p, workspace_mcp_config_path(&ctx).unwrap());

        // global
        let p = resolve_scope_profile(&ctx, Some(Scope::Global)).unwrap();
        assert_eq!(p, global_mcp_config_path(&ctx).unwrap());
    }

    #[ignore = "TODO: fix in CI"]
    #[tokio::test]
    async fn ensure_file_created_and_loaded() {
        let ctx = Context::new();
        let mut out = SharedWriter::null();
        let path = workspace_mcp_config_path(&ctx).unwrap();

        let cfg = super::ensure_config_file(&ctx, &path, &mut out).await.unwrap();
        assert!(path.exists(), "config file should be created");
        assert!(cfg.mcp_servers.is_empty());
    }

    #[tokio::test]
    async fn add_then_remove_cycle() {
        let ctx = Context::new();
        let mut out = SharedWriter::null();

        // 1. add
        AddArgs {
            name: "local".into(),
            command: "echo hi".into(),
            env: vec![],
            timeout: None,
            scope: None,
            force: false,
        }
        .execute(&ctx, &mut out)
        .await
        .unwrap();

        let cfg_path = workspace_mcp_config_path(&ctx).unwrap();
        let cfg: McpServerConfig =
            serde_json::from_str(&ctx.fs().read_to_string(cfg_path.clone()).await.unwrap()).unwrap();
        assert!(cfg.mcp_servers.len() == 1);

        // 2. remove
        RemoveArgs {
            name: "local".into(),
            scope: None,
        }
        .execute(&ctx, &mut out)
        .await
        .unwrap();

        let cfg: McpServerConfig = serde_json::from_str(&ctx.fs().read_to_string(cfg_path).await.unwrap()).unwrap();
        assert!(cfg.mcp_servers.is_empty());
    }

    #[test]
    fn test_mcp_subcomman_add() {
        assert_parse!(
            [
                "mcp",
                "add",
                "--name",
                "test_server",
                "--command",
                "test_command",
                "--env",
                "key1=value1,key2=value2"
            ],
            RootSubcommand::Mcp(McpSubcommand::Add(AddArgs {
                name: "test_server".to_string(),
                command: "test_command".to_string(),
                scope: None,
                env: vec![
                    [
                        ("key1".to_string(), "value1".to_string()),
                        ("key2".to_string(), "value2".to_string())
                    ]
                    .into_iter()
                    .collect()
                ],
                timeout: None,
                force: false,
            }))
        );
    }

    #[test]
    fn test_mcp_subcomman_remove_workspace() {
        assert_parse!(
            ["mcp", "remove", "--name", "old"],
            RootSubcommand::Mcp(McpSubcommand::Remove(RemoveArgs {
                name: "old".into(),
                scope: None,
            }))
        );
    }

    #[test]
    fn test_mcp_subcomman_import_profile_force() {
        assert_parse!(
            ["mcp", "import", "--file", "servers.json", "--force"],
            RootSubcommand::Mcp(McpSubcommand::Import(ImportArgs {
                file: "servers.json".into(),
                scope: None,
                force: true,
            }))
        );
    }

    #[test]
    fn test_mcp_subcommand_status_simple() {
        assert_parse!(
            ["mcp", "status", "--name", "aws"],
            RootSubcommand::Mcp(McpSubcommand::Status(StatusArgs { name: "aws".into() }))
        );
    }

    #[test]
    fn test_mcp_subcommand_list() {
        assert_parse!(
            ["mcp", "list", "global"],
            RootSubcommand::Mcp(McpSubcommand::List(ListArgs {
                scope: Some(Scope::Global),
                profile: None
            }))
        );
    }
}
