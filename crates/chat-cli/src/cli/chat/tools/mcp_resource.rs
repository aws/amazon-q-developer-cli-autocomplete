use std::io::Write;

use crossterm::queue;
use crossterm::style::{
    self,
    Color,
};
use eyre::{
    Result,
    bail,
};
use serde::Deserialize;

use super::InvokeOutput;
use crate::platform::Context;

/// Read content from MCP resources using their URI
#[derive(Debug, Clone, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub server_name: String,
}

impl McpResource {
    pub async fn validate(&mut self, _ctx: &Context) -> Result<()> {
        // Basic URI validation
        if self.uri.is_empty() {
            bail!("Resource URI cannot be empty");
        }

        if self.server_name.is_empty() {
            bail!("Server name cannot be empty");
        }

        // Note: Server validation is handled at invocation time since Context doesn't have access to
        // tool_manager

        Ok(())
    }

    pub async fn invoke(&self, _ctx: &Context, _updates: &mut impl Write) -> Result<InvokeOutput> {
        // This method should not be called directly for MCP resource tools
        // The actual implementation is in ChatContext::invoke_mcp_resource_tool
        // This is here as a fallback in case the special handling is bypassed
        bail!("MCP resource tool should be handled by special invocation logic");
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        queue!(
            updates,
            style::Print("Reading MCP resource: "),
            style::SetForegroundColor(Color::Green),
            style::Print(&self.uri),
            style::ResetColor,
            style::Print(" from server "),
            style::SetForegroundColor(Color::Cyan),
            style::Print(&self.server_name),
            style::ResetColor,
        )?;
        Ok(())
    }
}
