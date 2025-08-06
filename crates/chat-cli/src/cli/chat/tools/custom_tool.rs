use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crossterm::{
    queue,
    style,
};
use eyre::Result;
use serde::{
    Deserialize,
    Serialize,
};
use tokio::sync::RwLock;
use tracing::warn;

use super::InvokeOutput;
use crate::cli::chat::CONTINUATION_LINE;
use crate::cli::chat::token_counter::TokenCounter;
use crate::mcp_client::{
    Client as McpClient,
    ClientConfig as McpClientConfig,
    JsonRpcResponse,
    JsonRpcStdioTransport,
    MessageContent,
    Messenger,
    PromptGet,
    ServerCapabilities,
    StdioTransport,
    ToolCallResult,
};
use crate::os::Os;
use crate::util::env_expansion::{
    expand_env_vars_in_args,
    expand_env_vars_in_command,
    expand_env_vars_in_map,
};

// TODO: support http transport type
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CustomToolConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default)]
    pub disabled: bool,
}

pub fn default_timeout() -> u64 {
    120 * 1000
}

#[derive(Debug)]
pub enum CustomToolClient {
    Stdio {
        server_name: String,
        client: McpClient<StdioTransport>,
        server_capabilities: RwLock<Option<ServerCapabilities>>,
    },
}

impl CustomToolClient {
    // TODO: add support for http transport
    pub fn from_config(server_name: String, config: CustomToolConfig) -> Result<Self> {
        let CustomToolConfig {
            command,
            args,
            env,
            timeout,
            disabled: _,
        } = config;

        // Expand environment variables in command, args, and env
        let expanded_command = expand_env_vars_in_command(&command)
            .map_err(|e| eyre::eyre!("Failed to expand environment variables in command '{}': {}", command, e))?;

        let expanded_args = expand_env_vars_in_args(&args)
            .map_err(|e| eyre::eyre!("Failed to expand environment variables in args: {}", e))?;

        let expanded_env = if let Some(env_vars) = env {
            Some(
                expand_env_vars_in_map(&env_vars)
                    .map_err(|e| eyre::eyre!("Failed to expand environment variables in env: {}", e))?,
            )
        } else {
            None
        };

        let mcp_client_config = McpClientConfig {
            server_name: server_name.clone(),
            bin_path: expanded_command,
            args: expanded_args,
            timeout,
            client_info: serde_json::json!({
               "name": "Q CLI Chat",
               "version": "1.0.0"
            }),
            env: expanded_env,
        };
        let client = McpClient::<JsonRpcStdioTransport>::from_config(mcp_client_config)?;
        Ok(CustomToolClient::Stdio {
            server_name,
            client,
            server_capabilities: RwLock::new(None),
        })
    }

    pub async fn init(&self) -> Result<()> {
        match self {
            CustomToolClient::Stdio {
                client,
                server_capabilities,
                ..
            } => {
                if let Some(messenger) = &client.messenger {
                    let _ = messenger.send_init_msg().await;
                }
                // We'll need to first initialize. This is the handshake every client and server
                // needs to do before proceeding to anything else
                let cap = client.init().await?;
                // We'll be scrapping this for background server load: https://github.com/aws/amazon-q-developer-cli/issues/1466
                // So don't worry about the tidiness for now
                server_capabilities.write().await.replace(cap);
                Ok(())
            },
        }
    }

    pub fn assign_messenger(&mut self, messenger: Box<dyn Messenger>) {
        match self {
            CustomToolClient::Stdio { client, .. } => {
                client.messenger = Some(messenger);
            },
        }
    }

    pub fn get_server_name(&self) -> &str {
        match self {
            CustomToolClient::Stdio { server_name, .. } => server_name.as_str(),
        }
    }

    pub async fn request(&self, method: &str, params: Option<serde_json::Value>) -> Result<JsonRpcResponse> {
        match self {
            CustomToolClient::Stdio { client, .. } => Ok(client.request(method, params).await?),
        }
    }

    pub fn list_prompt_gets(&self) -> Arc<std::sync::RwLock<HashMap<String, PromptGet>>> {
        match self {
            CustomToolClient::Stdio { client, .. } => client.prompt_gets.clone(),
        }
    }

    #[allow(dead_code)]
    pub async fn notify(&self, method: &str, params: Option<serde_json::Value>) -> Result<()> {
        match self {
            CustomToolClient::Stdio { client, .. } => Ok(client.notify(method, params).await?),
        }
    }

    pub fn is_prompts_out_of_date(&self) -> bool {
        match self {
            CustomToolClient::Stdio { client, .. } => client.is_prompts_out_of_date.load(Ordering::Relaxed),
        }
    }

    pub fn prompts_updated(&self) {
        match self {
            CustomToolClient::Stdio { client, .. } => client.is_prompts_out_of_date.store(false, Ordering::Relaxed),
        }
    }
}

/// Represents a custom tool that can be invoked through the Model Context Protocol (MCP).
#[derive(Clone, Debug)]
pub struct CustomTool {
    /// Actual tool name as recognized by its MCP server. This differs from the tool names as they
    /// are seen by the model since they are not prefixed by its MCP server name.
    pub name: String,
    /// Reference to the client that manages communication with the tool's server process.
    pub client: Arc<CustomToolClient>,
    /// The method name to call on the tool's server, following the JSON-RPC convention.
    /// This corresponds to a specific functionality provided by the tool.
    pub method: String,
    /// Optional parameters to pass to the tool when invoking the method.
    /// Structured as a JSON value to accommodate various parameter types and structures.
    pub params: Option<serde_json::Value>,
}

impl CustomTool {
    pub async fn invoke(&self, _os: &Os, _updates: impl Write) -> Result<InvokeOutput> {
        // Assuming a response shape as per https://spec.modelcontextprotocol.io/specification/2024-11-05/server/tools/#calling-tools
        let resp = self.client.request(self.method.as_str(), self.params.clone()).await?;
        let result = match resp.result {
            Some(result) => result,
            None => {
                let failure = resp.error.map_or("Unknown error encountered".to_string(), |err| {
                    serde_json::to_string(&err).unwrap_or_default()
                });
                return Err(eyre::eyre!(failure));
            },
        };

        match serde_json::from_value::<ToolCallResult>(result.clone()) {
            Ok(mut de_result) => {
                for content in &mut de_result.content {
                    if let MessageContent::Image { data, .. } = content {
                        *data = format!("Redacted base64 encoded string of an image of size {}", data.len());
                    }
                }
                Ok(InvokeOutput {
                    output: super::OutputKind::Json(serde_json::json!(de_result)),
                })
            },
            Err(e) => {
                warn!("Tool call result deserialization failed: {:?}", e);
                Ok(InvokeOutput {
                    output: super::OutputKind::Json(result.clone()),
                })
            },
        }
    }

    pub fn queue_description(&self, output: &mut impl Write) -> Result<()> {
        queue!(
            output,
            style::Print("Running "),
            style::SetForegroundColor(style::Color::Green),
            style::Print(&self.name),
            style::ResetColor,
        )?;
        if let Some(params) = &self.params {
            let params = match serde_json::to_string_pretty(params) {
                Ok(params) => params
                    .split("\n")
                    .map(|p| format!("{CONTINUATION_LINE} {p}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => format!("{:?}", params),
            };
            queue!(
                output,
                style::Print(" with the param:\n"),
                style::Print(params),
                style::Print("\n"),
                style::ResetColor,
            )?;
        } else {
            queue!(output, style::Print("\n"))?;
        }
        Ok(())
    }

    pub async fn validate(&mut self, _os: &Os) -> Result<()> {
        Ok(())
    }

    pub fn get_input_token_size(&self) -> usize {
        TokenCounter::count_tokens(self.method.as_str())
            + TokenCounter::count_tokens(self.params.as_ref().map_or("", |p| p.as_str().unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::env;

    use super::*;

    #[test]
    fn test_custom_tool_config_env_expansion() {
        // Set up test environment variables
        env::set_var("TEST_USERNAME", "testuser");
        env::set_var("TEST_PASSWORD", "testpass");
        env::set_var("TEST_COMMAND", "python");
        env::set_var("TEST_ARG", "server.py");

        let mut env_vars = HashMap::new();
        env_vars.insert("USERNAME".to_string(), "${TEST_USERNAME}".to_string());
        env_vars.insert("PASSWORD".to_string(), "${TEST_PASSWORD}".to_string());
        env_vars.insert("STATIC_VAR".to_string(), "static_value".to_string());

        let config = CustomToolConfig {
            command: "${TEST_COMMAND}".to_string(),
            args: vec!["${TEST_ARG}".to_string(), "--config".to_string()],
            env: Some(env_vars),
            timeout: 30000,
            disabled: false,
        };

        let result = CustomToolClient::from_config("test_server".to_string(), config);

        // Clean up environment variables
        env::remove_var("TEST_USERNAME");
        env::remove_var("TEST_PASSWORD");
        env::remove_var("TEST_COMMAND");
        env::remove_var("TEST_ARG");

        assert!(
            result.is_ok(),
            "CustomToolClient creation should succeed with valid env vars"
        );
    }

    #[test]
    fn test_custom_tool_config_missing_env_var() {
        let mut env_vars = HashMap::new();
        env_vars.insert("USERNAME".to_string(), "${NONEXISTENT_VAR}".to_string());

        let config = CustomToolConfig {
            command: "python".to_string(),
            args: vec!["server.py".to_string()],
            env: Some(env_vars),
            timeout: 30000,
            disabled: false,
        };

        let result = CustomToolClient::from_config("test_server".to_string(), config);

        assert!(
            result.is_err(),
            "CustomToolClient creation should fail with missing env vars"
        );
        assert!(result.unwrap_err().to_string().contains("NONEXISTENT_VAR"));
    }

    #[test]
    fn test_custom_tool_config_no_env_expansion() {
        let config = CustomToolConfig {
            command: "python".to_string(),
            args: vec!["server.py".to_string(), "--port".to_string(), "8080".to_string()],
            env: None,
            timeout: 30000,
            disabled: false,
        };

        let result = CustomToolClient::from_config("test_server".to_string(), config);

        assert!(
            result.is_ok(),
            "CustomToolClient creation should succeed without env vars"
        );
    }

    #[test]
    fn test_custom_tool_config_mixed_env_expansion() {
        env::set_var("TEST_PORT", "8080");

        let mut env_vars = HashMap::new();
        env_vars.insert("PORT".to_string(), "${TEST_PORT}".to_string());
        env_vars.insert("HOST".to_string(), "localhost".to_string()); // No expansion needed

        let config = CustomToolConfig {
            command: "python".to_string(),
            args: vec![
                "server.py".to_string(),
                "--port".to_string(),
                "${TEST_PORT}".to_string(),
            ],
            env: Some(env_vars),
            timeout: 30000,
            disabled: false,
        };

        let result = CustomToolClient::from_config("test_server".to_string(), config);

        env::remove_var("TEST_PORT");

        assert!(
            result.is_ok(),
            "CustomToolClient creation should succeed with mixed env expansion"
        );
    }
}
