use crossterm::execute;
use crossterm::queue;
use futures::future::join_all;
use spinners::{Spinner, Spinners};
use std::io::Write;
use std::process::Stdio;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;

use super::InvokeOutput;
use super::OutputKind;
use crate::platform::Context;
use crate::util::spinner::SpinnerComponent;
use crossterm::cursor;
use crossterm::style::Attribute;
use crossterm::style::{self, Color};
use eyre::Result;
use serde::{Deserialize, Serialize};

/// Tool for launching a new Q agent as a background process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgent {
    // 3-5 word unique name to identify agent
    pub agent_name: String,
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
            style::SetForegroundColor(Color::Cyan),
            style::SetAttribute(Attribute::Bold),
            style::Print(format!(
                "Launch {} Q agent(s) to perform tasks in parallel:\n\n",
                self.subagents.len()
            )),
            style::ResetColor,
            style::Print("─".repeat(50)),
            style::Print("\n\n"),
        )?;

        for agent in self.subagents.iter() {
            queue!(
                updates,
                style::SetForegroundColor(Color::Blue),
                style::Print("  • "),
                style::SetForegroundColor(Color::White),
                style::SetAttribute(Attribute::Bold),
                style::Print(&agent.agent_name),
                style::ResetColor,
                style::SetForegroundColor(Color::DarkGrey),
                style::Print(" ("),
                style::Print(agent.model.clone().unwrap_or_else(|| "Claude-3.7-Sonnet".to_string())),
                style::Print(")\n"),
                style::ResetColor,
            )?;

            // Show truncated prompt preview
            let prompt_preview = if agent.prompt.len() > 60 {
                format!("{}...", &agent.prompt[..57])
            } else {
                agent.prompt.clone()
            };

            queue!(
                updates,
                style::SetForegroundColor(Color::DarkGrey),
                style::Print("    "),
                style::Print(prompt_preview),
                style::Print("\n\n"),
                style::ResetColor,
            )?;
        }

        Ok(())
    }
}

impl SubAgent {
    pub async fn invoke(agents: &[Self], updates: &mut impl Write) -> Result<InvokeOutput> {
        let prompt_template = r#"{}. SUBAGENT - You are a specialized instance delegated a task by your parent agent.

        SUBAGENT CONTEXT:
        - You are NOT the primary agent - you are a focused subprocess
        - Your parent agent is coordinating multiple subagents like you
        - Your role is to execute your specific task and report back with actionable intelligence
        - The parent agent depends on your detailed findings to make informed decisions
        
        CRITICAL REPORTING REQUIREMENTS:
        After completing your task, you MUST provide a DETAILED technical summary including:
        
        - Specific findings with concrete examples (file paths, code patterns, function names)
        - Actual implementation details and technical specifics
        - Quantifiable data (line counts, file sizes, performance metrics, etc.)
        - Key technical insights that directly inform the parent agent's next actions
        
        UNACCEPTABLE: Generic summaries like "analyzed codebase" or "completed task"
        REQUIRED: Specific technical intelligence that enables the parent agent to proceed effectively
        
        Execute your assigned subagent task, then provide your detailed technical report."#;

        let mut task_handles = Vec::new();
        std::fs::write("debug.log", "")?;

        // mpsc to track number of agents completed to update spinner
        let (progress_tx, mut progress_rx) = mpsc::channel::<u32>(agents.len());

        // Spawns a new async task for each subagent with prompt
        for agent in agents {
            let curr_prompt = prompt_template.replace("{}", &agent.prompt);
            let model_clone = agent.model.clone();
            let tx_clone = progress_tx.clone();
            let handle = spawn_agent_task(curr_prompt, model_clone, tx_clone).await?;
            task_handles.push(handle);
        }

        // Track completed progress and update spinner
        queue!(updates, style::Print("\n"),)?;
        let mut spinner = Spinner::new(
            Spinners::Dots,
            format!("Waiting for subagents... (0/{} complete)", agents.len()).into(),
        );

        let mut completed = 0;
        drop(progress_tx);
        while let Some(_) = progress_rx.recv().await {
            completed += 1;
            spinner.stop();
            spinner = Spinner::new(
                Spinners::Dots,
                format!("Waiting for subagents... ({}/{} complete)", completed, agents.len()).into(),
            );
        }
        spinner.stop();

        // wait till all subagents receive output
        let results = join_all(task_handles).await;

        // concatenate output + send to orchestrator
        let all_stdout = process_agent_results(results, updates)?;
        // send_concatenated_output(&all_stdout, updates).await?;

        Ok(InvokeOutput {
            output: OutputKind::Text(all_stdout),
        })
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        queue!(updates, style::Print(&self.prompt))?;
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

/// Runs a q subagent process as an async tokio task with specified prompt and model
async fn spawn_agent_task(
    prompt: String,
    model: Option<String>,
    tx: tokio::sync::mpsc::Sender<u32>,
) -> Result<tokio::task::JoinHandle<Result<(u32, std::process::ExitStatus, String), eyre::Error>>, eyre::Error> {
    let handle = tokio::spawn(async move {
        let mut cmd = tokio::process::Command::new("q");
        cmd.arg("chat");
        if let Some(model_arg) = model {
            cmd.arg(format!("--model={}", model_arg));
        }
        cmd.arg("--trust-all-tools");
        cmd.arg(prompt);
        cmd.env("Q_SUBAGENT", "1");

        let debug_log = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open("debug.log")?;

        // Clone the file handle for stderr
        let debug_log_stderr = debug_log.try_clone()?;

        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(std::process::Stdio::from(debug_log_stderr))
            .stdin(std::process::Stdio::null())
            .spawn()?;

        let child_pid = child
            .id()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "Failed to get child PID"))?;

        let output = capture_stdout_and_log(child.stdout.take().unwrap(), debug_log).await?;
        let exit_status = child.wait().await?;
        let _ = tx.send(1).await;
        Ok((child_pid, exit_status, output))
    });

    Ok(handle)
}

// Runs Q agent send to main pid
// async fn send_concatenated_output(all_stdout: &str, updates: &mut impl Write) -> Result<(), eyre::Error> {
//     queue!(
//         updates,
//         style::SetForegroundColor(Color::Yellow),
//         style::Print("\nSending concatenated agent outputs...\n"),
//         style::ResetColor,
//     )?;

//     let send_result = tokio::process::Command::new("q")
//         .arg("agent")
//         .arg("send")
//         .arg("--pid")
//         .arg(std::process::id().to_string())
//         .arg("--purpose")
//         .arg("summary")
//         .arg(all_stdout)
//         .status()
//         .await?;

//     if send_result.success() {
//         queue!(
//             updates,
//             style::SetForegroundColor(Color::Yellow),
//             style::Print("Successfully sent agent outputs\n\n"),
//             style::ResetColor,
//         )?;
//     } else {
//         queue!(
//             updates,
//             style::SetForegroundColor(Color::Red),
//             style::Print(format!("Failed to send agent outputs: {:?}\n\n", send_result.code())),
//             style::ResetColor,
//         )?;
//     }

//     Ok(())
// }

/// Formats and joins all subagent summaries with error printing for user
fn process_agent_results(
    results: Vec<Result<Result<(u32, std::process::ExitStatus, String), eyre::Error>, tokio::task::JoinError>>,
    updates: &mut impl Write,
) -> Result<String, eyre::Error> {
    let mut all_stdout = String::new();

    for task_result in results {
        match task_result {
            Ok(Ok((child_pid, exit_status, stdout_output))) => {
                if !stdout_output.trim().is_empty() {
                    all_stdout.push_str(&format!("=== Agent {} Output ===\n", child_pid));
                    all_stdout.push_str(&stdout_output);
                    all_stdout.push_str("\n\n");
                }
            },
            Ok(Err(e)) => {
                queue!(
                    updates,
                    style::SetForegroundColor(Color::Red),
                    style::Print(format!("Failed to launch agent: {}\n", e)),
                    style::ResetColor,
                )?;
            },
            Err(e) => {
                queue!(
                    updates,
                    style::SetForegroundColor(Color::Red),
                    style::Print(format!("Task join error: {}\n", e)),
                    style::ResetColor,
                )?;
            },
        }
    }

    Ok(all_stdout)
}

/// Async function that takes child stdout and stores it
async fn capture_stdout_and_log(
    stdout: tokio::process::ChildStdout,
    mut debug_log: std::fs::File,
) -> Result<String, eyre::Error> {
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut output = String::new();
    let mut line = String::new();

    while reader.read_line(&mut line).await? > 0 {
        writeln!(debug_log, "{}", line.trim_end())?;
        output.push_str(&line);
        line.clear();
    }

    Ok(output)
}
