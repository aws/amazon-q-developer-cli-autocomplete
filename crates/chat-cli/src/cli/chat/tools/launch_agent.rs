use crossterm::queue;
use std::io::Write;
use std::process::Command;

use super::InvokeOutput;
use super::OutputKind;
use crate::platform::Context;
use crossterm::style::{self, Color};
use eyre::Result;
use serde::{Deserialize, Serialize};

/// Tool for launching a new Q agent as a background process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgent {
    /// The prompt to send to the new agent
    pub prompt: String,
    /// Optional model to use for the agent (defaults to the system default)
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentWrapper {
    pub subagents: Vec<SubAgent>,
}

impl SubAgentWrapper {
    pub async fn invoke(&self, updates: &mut impl Write) -> Result<InvokeOutput> {
        SubAgent::invoke(&self.subagents, updates).await
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        queue!(
            updates,
            style::SetForegroundColor(Color::Yellow),
            style::Print(format!(
                "Launch {} Q agent(s) to perform tasks in parallel:\n\n",
                self.subagents.len()
            )),
            style::ResetColor,
        )?;

        for (i, agent) in self.subagents.iter().enumerate() {
            queue!(
                updates,
                style::SetForegroundColor(Color::Yellow),
                style::Print(format!(
                    "  Agent {} ({}): ",
                    i + 1,
                    agent.model.clone().unwrap_or_else(|| "Claude-3.7-Sonnet".to_string())
                )),
                style::ResetColor
            )?;

            agent.queue_description(updates)?;

            queue!(updates, style::Print("\n\n"))?;
        }

        Ok(())
    }
}

impl SubAgent {
    pub async fn invoke(agents: &[Self], updates: &mut impl Write) -> Result<InvokeOutput> {
        let mut child_pids = Vec::new();
        let parent_pid: u32 = std::process::id();

        // write number of subagents being used to parent orchestrator stream
        Command::new("q")
            .args(&[
                "agent",
                "send",
                "--pid",
                &parent_pid.to_string(),
                "--purpose",
                "num_agents",
                &format!("NUM_AGENTS {}", agents.len()),
            ])
            .status()?;

        // debug clear file (async)
        tokio::fs::write("debug.log", "").await?;

        // Pre-format the prompt template to avoid repeated string building
        let parent_pid_str = parent_pid.to_string();

        let prompt_template = format!(
            r#"{{}}
        
        CRITICAL INSTRUCTIONS - YOU MUST FOLLOW THESE EXACTLY:
        
        1. After completing your task, you MUST provide a DETAILED summary formatted EXACTLY as:
           [SUMMARY] your detailed summary text [/SUMMARY]
        
           Your summary MUST include:
           - Specific findings, not just general descriptions
           - Concrete examples, code patterns, or file paths you discovered
           - Actual implementation details, not just high-level concepts
           - Numbers, metrics, or quantifiable observations where relevant
           - Key insights that would be valuable to share with the parent process
        
        2. IMMEDIATELY after providing your summary, you MUST execute this EXACT shell command:
           q agent send --pid {} --purpose summary "replace with detailed summary mentioned above"
        
        The $$ should be a short role name (ex: tester, reviewer, etc).
        
        IMPORTANT: Generic summaries like "Analyzed the codebase" or "Completed the task" are NOT acceptable.
        Your summary MUST contain SPECIFIC technical details that would be valuable to the parent process.
        
        FALLBACK MECHANISM: Even if you cannot complete the task, you MUST STILL send a summary with whatever specific details you were able to discover."#,
            parent_pid_str
        );

        // Launch all agents concurrently
        let mut tasks = tokio::task::JoinSet::new();

        for agent in agents {
            let prompt = agent.prompt.clone();
            let model = agent.model.clone();
            let enhanced_prompt = prompt_template.replace("{}", &prompt);

            tasks.spawn(async move {
                // Run subagent with env var, model, trust all tools set
                let mut cmd = tokio::process::Command::new("q");
                cmd.arg("chat");
                if let Some(model_arg) = &model {
                    cmd.arg(format!("--model={}", model_arg));
                }
                cmd.arg("--trust-all-tools");
                cmd.arg(&enhanced_prompt);
                cmd.env("Q_SUBAGENT", "1");

                // Launch the process in the background with logs to debug.log
                let debug_log = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(true)
                    .open("debug.log")?;

                let child = cmd
                    .stdout(std::process::Stdio::from(debug_log.try_clone()?))
                    .stderr(std::process::Stdio::from(debug_log))
                    .stdin(std::process::Stdio::null())
                    .spawn()?;

                let child_pid = child
                    .id()
                    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "Failed to get child PID"))?;

                Ok::<u32, Box<dyn std::error::Error + Send + Sync>>(child_pid)
            });
        }

        // Collect results from all spawned agents
        while let Some(task_result) = tasks.join_next().await {
            match task_result {
                Ok(Ok(child_pid)) => {
                    child_pids.push(child_pid);
                    queue!(
                        updates,
                        style::SetForegroundColor(Color::Yellow),
                        style::Print(format!("\nLaunched new Q agent (PID: {})\n\n", child_pid)),
                        style::ResetColor,
                    )?;
                },
                Ok(Err(e)) => {
                    // Handle individual agent spawn errors
                    queue!(
                        updates,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("\nFailed to launch agent: {}\n\n", e)),
                        style::ResetColor,
                    )?;
                },
                Err(e) => {
                    // Handle task join errors
                    queue!(
                        updates,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("\nTask join error: {}\n\n", e)),
                        style::ResetColor,
                    )?;
                },
            }
        }

        Ok(InvokeOutput {
            output: OutputKind::Text(format!(
                "Successfully launched {} agents: {:?}",
                child_pids.len(),
                child_pids
            )),
        })
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        queue!(
            updates,
            style::Print(format!("Launch a new Q agent with prompt: {}", self.prompt)),
        )?;
        Ok(())
    }

    /// non-empty prompt validation
    pub async fn validate(&self, _ctx: &Context) -> Result<()> {
        if self.prompt.trim().is_empty() {
            return Err(eyre::eyre!("Prompt cannot be empty"));
        }
        Ok(())
    }
}
