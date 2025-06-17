use crossterm::queue;
use std::io::Write;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::time::sleep;

use super::InvokeOutput;
use super::OutputKind;
use crate::platform::Context;
use crate::util::spinner::{Spinner, SpinnerComponent};
use crossterm::style::{self, Color};
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Tool for launching a new Q agent as a background process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgent {
    /// The prompt to send to the new agent
    pub prompt: String,
    /// Optional model to use for the agent (defaults to the system default)
    pub model: Option<String>,
}

impl SubAgent {
    pub async fn invoke(agents: &[Self], updates: &mut impl Write) -> Result<InvokeOutput> {
        let mut child_pids = Vec::new();
        let parent_pid: u32 = std::process::id();
        let socket_path = format!("/tmp/qchat/{}", parent_pid);

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

        // Launch each agent
        for agent in agents {
            let prompt = agent.prompt.clone();
            let model = agent.model.clone();
            let enhanced_prompt = format!(
                "{}\n\nAfter completing the above task, provide a summary formatted as: \
                 [SUMMARY] your summary text [/SUMMARY]",
                prompt
            );

            // Run subagent with env var, model, trust all tools set
            let mut cmd = Command::new("q");
            cmd.arg("chat");
            if let Some(model_arg) = &model {
                cmd.args(["--model", model_arg]);
            }
            cmd.arg("--trust-all-tools");
            cmd.arg(&enhanced_prompt);
            cmd.env("Q_SUBAGENT", "1");

            // Launch the process in the background
            let child = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
            let child_pid = child.id();
            child_pids.push(child_pid);

            queue!(
                updates,
                style::SetForegroundColor(Color::Yellow),
                style::Print(format!("\nLaunched new Q agent (PID: {})\n\n", child_pid)),
                style::ResetColor,
            )?;

            let summary_output = Command::new("q")
                .args(&[
                    "agent",
                    "send",
                    "--pid",
                    &parent_pid.to_string(),
                    "--purpose",
                    "summary",
                    &enhanced_prompt,
                ])
                .output()?;
            let summary_string = String::from_utf8_lossy(&summary_output.stdout).to_string();

            // Send the summary back to the parent process
            Command::new("q")
                .args(&[
                    "agent",
                    "send",
                    "--pid",
                    &parent_pid.to_string(),
                    &format!(
                        "[AGENT_SUMMARY:{}] {} [/AGENT_SUMMARY]",
                        parent_pid,
                        summary_string.trim()
                    ),
                ])
                .status()?;
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
            style::SetForegroundColor(Color::Yellow),
            style::Print(format!("Launch a new Q agent with prompt: {}", self.prompt)),
            style::ResetColor,
        )?;
        Ok(())
    }

    // non-empty prompt validation
    pub async fn validate(&self, _ctx: &Context) -> Result<()> {
        if self.prompt.trim().is_empty() {
            return Err(eyre::eyre!("Prompt cannot be empty"));
        }
        Ok(())
    }

    // Start monitoring agent statuses in the background
    // pub fn start_agent_status_monitor() {
    //     tokio::spawn(async {
    //         if let Err(e) = Self::check_all_agents_waiting_for_input().await {
    //             eprintln!("Error monitoring agent statuses: {:?}", e);
    //         }
    //     });
    // }

    // //// Asynchronously checks if all child agents are waiting for user input
    // /// and signals the parent when they are
    // async fn check_all_agents_waiting_for_input() -> Result<(), Box<dyn std::error::Error>> {
    //     let parent_pid = std::process::id();
    //     let mut spinner = Spinner::new(vec![
    //         SpinnerComponent::Spinner,
    //         SpinnerComponent::Text(" Waiting for all subagents to complete...".into()),
    //     ]);

    //     loop {
    //         // Check the status of all agents using q agent list --json
    //         let output = Command::new("q")
    //             .args(&["agent", "list", "--single", "--json"])
    //             .output()?;

    //         if !output.status.success() {
    //             // Command failed, wait and retry
    //             sleep(Duration::from_millis(500)).await;
    //             continue;
    //         }

    //         // Parse the JSON output
    //         let json: Value = match serde_json::from_slice(&output.stdout) {
    //             Ok(json) => json,
    //             Err(_) => {
    //                 // Failed to parse JSON, wait and retry
    //                 sleep(Duration::from_millis(500)).await;
    //                 continue;
    //             },
    //         };

    //         // Check if all child agents are waiting for user input
    //         let mut all_waiting = true;
    //         if let Some(agents) = json.as_array() {
    //             for agent in agents {
    //                 if let (Some(pid), Some(status)) = (
    //                     agent.get("pid").and_then(|p| p.as_u64()),
    //                     agent.get("status").and_then(|s| s.as_str()),
    //                 ) {
    //                     if status != "waiting for user input" {
    //                         all_waiting = false;
    //                         break;
    //                     }
    //                 }
    //             }
    //         }

    //         // If all active child agents are waiting for user input, notify the parent
    //         if all_waiting {
    //             spinner.stop_with_message("All agents have completed their tasks".into());
    //             let all_summaries = Self::get_all_summaries();
    //             let summary_text = if all_summaries.is_empty() {
    //                 "All child agents are waiting for user input (no summaries available)".to_string()
    //             } else {
    //                 let mut text = String::new();
    //                 for (pid, summary) in all_summaries {
    //                     text.push_str(&format!("• Summary from Agent PID {}: {}\n", pid, summary.trim()));
    //                 }
    //                 text
    //             };

    //             Command::new("q")
    //                 .args(&[
    //                     "agent",
    //                     "send",
    //                     "--pid",
    //                     &parent_pid.to_string(),
    //                     &format!("[SUMMARY] {} [/SUMMARY]", summary_text),
    //                 ])
    //                 .status()?;

    //             return Ok(());
    //         }

    //         // Wait before checking again
    //         tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    //     }
    // }
}
