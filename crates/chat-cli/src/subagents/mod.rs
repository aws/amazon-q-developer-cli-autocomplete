use std::env;
// Import the submodules
use std::process::{
    Command,
    ExitCode,
};

use clap::{
    Args,
    Subcommand,
};
use crossterm::style::{
    Attribute,
    Color,
};
use crossterm::{
    execute,
    style,
};
use eyre::Result;
use libproc::libproc::proc_pid;
use libproc::processes;
use serde::Serialize;
use tokio::io::{
    AsyncReadExt,
    AsyncWriteExt,
};
use tokio::net::UnixStream;
use tokio::signal::ctrl_c;

use crate::cli::OutputFormat;
use crate::cli::chat::util::shared_writer::SharedWriter;
use crate::util::choose;

// Arguments for agent command
#[derive(Debug, Args, PartialEq, Eq)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub subcommand: Option<AgentSubcommand>,
}

// Define all possible enums for agent
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum AgentSubcommand {
    List(ListArgs),
    Compare(CompareArgs),
    Send(SendArgs),
}

// Define all possible arguments for list subcommand
#[derive(Debug, Args, PartialEq, Eq)]
pub struct ListArgs {
    /// Output format just says can be --f, -f, etc
    #[arg(long, short, value_enum, default_value_t)]
    pub format: OutputFormat,
    /// Run once and exit (breaks out of loop)
    #[arg(long)]
    pub single: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum MessagePurpose {
    List,
    Prompt,
    Summary,
    NumAgents,
    Default,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct CompareArgs {
    pub task_description: String,
    #[arg(long, value_delimiter = ',')]
    pub models: Vec<String>,
    #[arg(long)]
    pub path: Option<String>,
    #[arg(long, short, value_enum, default_value_t)]
    pub format: OutputFormat,
}

#[derive(Debug, Args, PartialEq, Eq, Clone)]
pub struct SendArgs {
    pub task_description: String,
    #[arg(long)]
    pub pid: u32,
    #[arg(long, short, value_enum, default_value_t)]
    pub format: OutputFormat,
    #[arg(long, help = "Optional purpose for this message")]
    pub purpose: Option<MessagePurpose>,
}

#[derive(Debug, Serialize)]
pub struct AgentInfo {
    pub pid: u32,
    pub profile: String,
    pub tokens_used: u64,
    pub context_window_percent: f32,
    pub running_time: u64,
    pub status: String,
}

impl std::str::FromStr for MessagePurpose {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "list" => Ok(MessagePurpose::List),
            "prompt" => Ok(MessagePurpose::Prompt),
            "summary" => Ok(MessagePurpose::Summary),
            "num_agents" => Ok(MessagePurpose::NumAgents),
            _ => Ok(MessagePurpose::Default),
        }
    }
}
// TODO: Fix error handling logic instead of just adding eprintln everywhere
// Lists all chat_cli instances metadata running in system
pub async fn list_agents(args: ListArgs) -> Result<ExitCode> {
    // Set up for live updates
    let refresh_interval = tokio::time::Duration::from_secs(1);
    let mut interval = tokio::time::interval(refresh_interval);
    let mut output = SharedWriter::stdout();
    let display_once = args.single;

    println!("Live agent status (press Ctrl+C to exit)...");

    // Main refresh loop
    loop {
        // Clear screen
        execute!(
            output,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
            crossterm::cursor::MoveTo(0, 0)
        )?;

        execute!(
            output,
            style::SetForegroundColor(Color::White),
            style::SetAttribute(Attribute::Bold),
            style::Print(format!(
                "{:<8} {:<25} {:<20} {:<10} {:<10} {:<8}\n",
                "PID", "Status", "Profile", "Tokens", "Context %", "Duration"
            )),
            style::SetAttribute(Attribute::Reset),
            style::Print("▔".repeat(80)),
            style::Print("\n")
        )?;

        // Ask all chat_cli instances for metadata using UDS
        let process_filter = processes::ProcFilter::All;
        let all_procs = processes::pids_by_type(process_filter)?;
        let mut agent_infos: Vec<_> = Vec::new();

        for curr_process in all_procs {
            let curr_pid = curr_process.try_into().unwrap();
            let curr_process_name = proc_pid::name(curr_pid).unwrap_or("Unknown process".to_string());
            let is_qcli_process = curr_process_name.contains("chat_cli")
                || curr_process_name.contains("qchat")
                || curr_process_name.contains("q_cli")
                || curr_process_name.contains("q chat")
                || curr_process_name == "q";
            if is_qcli_process {
                let socket_path = format!("/tmp/qchat/{}", curr_pid);
                if !std::path::Path::new(&socket_path).exists() {
                    continue;
                }
                match UnixStream::connect(&socket_path).await {
                    Ok(mut stream) => {
                        // Send request
                        stream.write_all(b"LIST ").await?;
                        let mut buffer = [0u8; 1024];
                        // Read response metadata
                        let n = stream.read(&mut buffer).await?;
                        if n == 0 {
                            continue;
                        }
                        let response_str = std::str::from_utf8(&buffer[..n]).unwrap_or("<invalid utf8>");

                        // Parse JSON response
                        match serde_json::from_str::<serde_json::Value>(&response_str) {
                            Ok(json) => {
                                // Extract values from JSON
                                let profile = json.get("profile").and_then(|v| v.as_str()).unwrap_or("unknown");
                                let tokens_used = json.get("tokens_used").and_then(|v| v.as_u64()).unwrap_or(0);
                                let context_window = json
                                    .get("context_window")
                                    .and_then(|v| v.as_f64())
                                    .map(|v| v as f32)
                                    .unwrap_or(0.0);
                                let total_time_sec = json
                                    .get("duration_secs")
                                    .and_then(|v| v.as_f64())
                                    .map(|v| v as f32)
                                    .unwrap_or(0.0);
                                let status = json.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");

                                // Create AgentInfo
                                let info = AgentInfo {
                                    pid: curr_process,
                                    profile: profile.to_string(),
                                    tokens_used,
                                    context_window_percent: context_window,
                                    running_time: total_time_sec as u64,
                                    status: status.to_string(),
                                };

                                agent_infos.push(info);
                            },
                            Err(_) => continue,
                        }
                    },
                    Err(_) => continue,
                }
            }
        }

        // Print results
        if !agent_infos.is_empty() {
            for info in &agent_infos {
                execute!(
                    output,
                    style::SetForegroundColor(Color::Green),
                    style::Print(format!("{:<8} ", info.pid)),
                    style::SetForegroundColor(Color::Blue),
                    style::Print(format!("{:<25} ", info.status)),
                    style::SetForegroundColor(Color::Magenta),
                    style::Print(format!("{:<20} ", info.profile)),
                    style::SetForegroundColor(Color::Yellow),
                    style::Print(format!("{:<10} ", info.tokens_used)),
                    style::SetForegroundColor(Color::DarkCyan),
                    style::Print(format!("{:<10.1}% ", info.context_window_percent)),
                    style::SetForegroundColor(Color::White),
                    style::Print(format!("{:<8}s", info.running_time)),
                    style::SetForegroundColor(Color::Reset),
                    style::Print("\n")
                )?;
            }
        } else {
            execute!(
                output,
                style::SetForegroundColor(Color::DarkGrey),
                style::Print("No running instances found.\n"),
                style::SetForegroundColor(Color::Reset)
            )?;
        }

        execute!(
            output,
            style::SetForegroundColor(Color::Cyan),
            style::SetAttribute(Attribute::Bold),
            style::Print(format!("\nFound {} active Q agent(s)\n", agent_infos.len())),
            style::SetAttribute(Attribute::Reset)
        )?;

        execute!(
            output,
            style::SetForegroundColor(Color::Yellow),
            style::Print("\nPress Ctrl+C to exit\n"),
            style::SetForegroundColor(Color::Reset)
        )?;

        // Wait for next interval or Ctrl+C
        let ctrl_c_stream = ctrl_c();
        tokio::select! {
            _ = interval.tick() => {
            },
            Ok(_) = ctrl_c_stream => {
                break;
            }
        }
        if display_once {
            break;
        }
    }
    Ok(ExitCode::SUCCESS)
}

// Summon multiple models to do the same task and compare results amongst the different git
// worktrees
pub async fn compare_agents(args: CompareArgs) -> Result<ExitCode> {
    // Check if we're in a git repo
    if !is_in_git_repo() {
        eprintln!("Error: Not in a git repository. Please run this command from a git repository.");
        return Ok(ExitCode::FAILURE);
    }

    // Create a new tmux session
    let mut output = SharedWriter::stdout();
    let main_pid: u32 = std::process::id();
    let session_name = format!("qagent-compare-{}", main_pid);
    let tmux_create = Command::new("tmux")
        .args(["new-session", "-d", "-s", &session_name])
        .output()?;

    if !tmux_create.status.success() {
        eprintln!(
            "Failed to create tmux session: {}",
            String::from_utf8_lossy(&tmux_create.stderr)
        );
        return Ok(ExitCode::FAILURE);
    }

    // Establish where CWD must be (path flag or cwd default)
    let base_dir = if let Some(path) = args.path.clone() {
        std::path::PathBuf::from(path)
    } else {
        // falls back to temp if get_cwd fails
        env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
    };

    // Create a worktree for each model and start a chat session
    for (i, model) in args.models.iter().enumerate() {
        let worktree_dir = base_dir.join(format!("worktree-{}-{}", model, main_pid));
        let worktree_dir_str = worktree_dir.to_str().unwrap();
        // Create git worktree with detach flag (background task)
        let git_worktree = Command::new("git")
            .args(["worktree", "add", "--detach", &worktree_dir_str])
            .output()?;

        if !git_worktree.status.success() {
            eprintln!(
                "Failed to create git worktree: {}",
                String::from_utf8_lossy(&git_worktree.stderr)
            );
            continue;
        }

        // Create a new tmux window for this model with cwd set
        let window_name = format!("{}", model);
        let window_index = i + 1; // Window indices start at 1 in tmux
        let tmux_window = Command::new("tmux")
            .args([
                "new-window",
                "-d",
                "-n",
                &window_name,
                "-t",
                &format!("{}:{}", session_name, window_index),
                "-c",
                &worktree_dir_str,
            ])
            .output()?;

        if !tmux_window.status.success() {
            eprintln!(
                "Failed to create tmux window: {}",
                String::from_utf8_lossy(&tmux_window.stderr)
            );
            continue;
        }

        // Start q chat with the specified model
        //
        let trust_all_tools = "--trust-all-tools";
        let chat_command = format!(
            "q chat --model {} {} \"{}\"",
            model,
            trust_all_tools,
            args.task_description.replace("\"", "\\\"") // Escape quotes
        );

        // sends keys emulates typing in a window
        // We are routing prompt to appropriate model window
        let tmux_send = Command::new("tmux")
            .args([
                "send-keys",
                "-t",
                &format!("{}:{}", session_name, window_index),
                &chat_command,
                "Enter",
            ])
            .output()?;

        if !tmux_send.status.success() {
            eprintln!(
                "Failed to send command to tmux: {}",
                String::from_utf8_lossy(&tmux_send.stderr)
            );
        }
    }

    // UI update
    execute!(
        output,
        style::SetForegroundColor(Color::Cyan),
        style::SetAttribute(Attribute::Bold),
        style::Print(format!(
            "\nCreated tmux session '{}' with windows for models: {:?}\n",
            session_name, args.models
        )),
        style::SetAttribute(Attribute::Reset),
        style::SetForegroundColor(Color::White),
        style::Print("\nRun the following command to attach to the session:\n"),
        style::Print(format!("  tmux attach-session -t {}\n\n", session_name)),
        style::SetForegroundColor(Color::Yellow),
        style::Print("Waiting for all models to complete their tasks...\n"),
        style::Print("Once completed, you can compare the results in each worktree.\n\n"),
        style::SetForegroundColor(Color::Reset)
    )?;

    // Grab user model preference
    match choose("Please select your desired model: ", &args.models)? {
        Some(index) => {
            let model_number = index + 1;
            let _ = handle_model_choice(model_number, args, main_pid);
        },
        None => {
            let model_number = args.models.len() + 1;
            let _ = handle_model_choice(model_number, args, main_pid);
        },
    }

    execute!(
        output,
        style::SetForegroundColor(Color::Green),
        style::Print(format!("Clean up process complete.\n")),
        style::SetForegroundColor(Color::Reset),
    )?;
    Ok(ExitCode::SUCCESS)
}

fn is_in_git_repo() -> bool {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
        .expect("Failed to execute git command");

    String::from_utf8_lossy(&output.stdout).trim() == "true"
}

// Helper: deletes git work trees + close tmux sessions
fn handle_model_choice(model_number: usize, args: CompareArgs, main_pid: u32) -> Result<String> {
    let base_dir = if let Some(path) = args.path.clone() {
        std::path::PathBuf::from(path)
    } else {
        env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
    };

    // Delete each work tree except selected
    for (i, model) in args.models.iter().enumerate() {
        if i + 1 != model_number {
            let worktree_dir = base_dir.join(format!("worktree-{}-{}", model, main_pid));
            let worktree_dir_str = worktree_dir.to_str().unwrap();

            let remove_result = Command::new("git")
                .args(["worktree", "remove", "--force", worktree_dir_str])
                .output();
            if let Ok(output) = remove_result {
                if !output.status.success() {
                    eprintln!("Failed to delete git worktree: {}", worktree_dir_str);
                }
            }

            // close associated tmux sessions
            let _ = Command::new("tmux")
                .args(["kill-window", "-t", &format!("qagent-compare-{}:{}", main_pid, i + 1)])
                .output();
        }
    }
    Ok("Success".to_string())
}

// Send messages to subagents with pid specified
pub async fn send_agent_message(args: SendArgs) -> Result<ExitCode> {
    // ensure socket path exists
    let agent_args = args.clone();
    let curr_pid = agent_args.pid;
    let agent_prompt = agent_args.task_description;
    let socket_path = format!("/tmp/qchat/{}", curr_pid);
    if !std::path::Path::new(&socket_path).exists() {
        return Ok(ExitCode::FAILURE);
    }

    // route message to socket: message prefix based on purpose
    match UnixStream::connect(&socket_path).await {
        Ok(mut stream) => {
            let prefix = if let Some(purpose) = agent_args.purpose {
                match purpose {
                    MessagePurpose::List => "LIST ",
                    MessagePurpose::Prompt => "PROMPT ",
                    MessagePurpose::Summary => "SUMMARY ",
                    MessagePurpose::NumAgents => "NUM_AGENTS ",
                    MessagePurpose::Default => "MESSAGE_SEND_BEGIN ",
                }
            } else {
                "MESSAGE_SEND_BEGIN "
            };

            // Write the prefix followed by the message
            stream.write_all(prefix.as_bytes()).await?;
            if let Some((_, rest)) = agent_prompt.split_once(' ') {
                stream.write_all(rest.as_bytes()).await?;
            } else {
                stream.write_all(agent_prompt.as_bytes()).await?;
            }
        },
        Err(_) => (),
    }
    Ok(ExitCode::SUCCESS)
}

impl AgentArgs {
    pub async fn execute(self) -> Result<ExitCode> {
        match self.subcommand {
            Some(AgentSubcommand::List(args)) => list_agents(args).await,
            Some(AgentSubcommand::Compare(args)) => compare_agents(args).await,
            Some(AgentSubcommand::Send(args)) => send_agent_message(args).await,
            None => {
                list_agents(ListArgs {
                    format: OutputFormat::default(),
                    single: false,
                })
                .await
            },
        }
    }
}
