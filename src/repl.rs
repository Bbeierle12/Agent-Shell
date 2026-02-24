use agent_core::agent_loop::AgentLoop;
use agent_core::config::{AppConfig, SandboxMode};
use agent_core::session::SessionManager;
use agent_core::tool_registry::ToolRegistry;
use agent_core::types::{AgentEvent, Message};
use agent_skills::SkillIndexer;
use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::{Config as RlConfig, DefaultEditor};
use std::sync::Arc;
use tokio::sync::mpsc;

const BANNER: &str = r#"
  ╔═══════════════════════════════════════════╗
  ║          agent-shell v0.1.0               ║
  ║   Model-agnostic AI agent shell           ║
  ╚═══════════════════════════════════════════╝

  Type your message and press Enter to chat.
  Commands:
    /new [name]    — Create a new session
    /sessions      — List all sessions
    /switch <id>   — Switch to a session
    /tools         — List available tools
    /skills        — List loaded skills
    /context [dir] — Detect project, git, and runtime environments
    /analytics     — Show session analytics summary
    /shells        — List detected shells
    /config        — Show current config
    /clear         — Clear current session history
    /help          — Show this help
    /exit          — Quit
"#;

/// Run the interactive REPL.
pub async fn run(
    config: AppConfig,
    tool_registry: Arc<ToolRegistry>,
    skill_indexer: Arc<SkillIndexer>,
    session_name: Option<String>,
) -> Result<()> {
    println!("{}", BANNER);
    println!(
        "  Model: {}  |  Endpoint: {}",
        config.provider.model, config.provider.api_base
    );

    // Warn if running in unsafe (unsandboxed) mode.
    if config.sandbox.mode == SandboxMode::Unsafe {
        println!("\x1b[1;33m  ⚠  WARNING: Sandbox mode is 'unsafe' — tools execute directly on your system!\x1b[0m");
        println!("\x1b[1;33m     Set [sandbox] mode = \"docker\" in config for isolated execution.\x1b[0m");
    }
    println!();

    let mut session_manager = SessionManager::new(&config)?;
    if let Some(name) = session_name {
        session_manager.create_session(name)?;
    }

    let agent_loop = AgentLoop::new(config.clone(), tool_registry.clone())?;

    // Set up rustyline.
    let rl_config = RlConfig::builder().auto_add_history(true).build();
    let history_path = AppConfig::data_dir().join("repl_history.txt");
    let mut rl = DefaultEditor::with_config(rl_config)?;
    let _ = rl.load_history(&history_path);

    loop {
        let session_name = session_manager
            .active_session()
            .map(|s| s.name.as_str())
            .unwrap_or("default");
        let prompt = format!("\x1b[1;36m{}\x1b[0m \x1b[1;32m❯\x1b[0m ", session_name);

        match rl.readline(&prompt) {
            Ok(line) => {
                let input = line.trim();
                if input.is_empty() {
                    continue;
                }

                // Handle slash commands.
                if input.starts_with('/') {
                    let handled = handle_command(
                        input,
                        &mut session_manager,
                        &tool_registry,
                        &skill_indexer,
                        &config,
                    )?;
                    if !handled {
                        break; // /exit
                    }
                    continue;
                }

                // Send user message to agent.
                let user_msg = Message::user(input);
                session_manager.push_message(user_msg)?;

                let messages: Vec<Message> = session_manager
                    .recent_messages()
                    .into_iter()
                    .cloned()
                    .collect();

                // Get session tool filtering.
                let (allowlist, denylist) = {
                    let session = session_manager.active_session().unwrap();
                    (
                        session.tool_allowlist.clone(),
                        session.tool_denylist.clone(),
                    )
                };

                // Create event channel.
                let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();

                // Run agent in background.
                let _agent_loop_ref = &agent_loop;

                // We need to run the agent and consume events concurrently.
                let agent_handle = {
                    let messages = messages.clone();
                    let allowlist = allowlist.clone();
                    let denylist = denylist.clone();
                    let tx = tx.clone();
                    tokio::spawn({
                        let tool_registry = tool_registry.clone();
                        let config = config.clone();
                        async move {
                            let agent = AgentLoop::new(config, tool_registry)?;
                            agent
                                .run(&messages, allowlist.as_deref(), &denylist, tx)
                                .await
                        }
                    })
                };
                drop(tx); // Drop our copy so the channel closes when agent is done.

                // Print events as they arrive.
                print!("\x1b[1;33massistant\x1b[0m: ");
                let mut full_response = String::new();
                while let Some(event) = rx.recv().await {
                    match event {
                        AgentEvent::ContentChunk(token) => {
                            print!("{}", token);
                            full_response.push_str(&token);
                        }
                        AgentEvent::ToolCallStart { name, .. } => {
                            println!("\n  \x1b[0;35m⚡ Calling tool: {}\x1b[0m", name);
                        }
                        AgentEvent::ToolResult(output) => {
                            let status = if output.is_error {
                                "\x1b[0;31m✗\x1b[0m"
                            } else {
                                "\x1b[0;32m✓\x1b[0m"
                            };
                            let preview = if output.content.len() > 200 {
                                format!("{}...", &output.content[..200])
                            } else {
                                output.content.clone()
                            };
                            println!("  {} {}", status, preview.replace('\n', "\n    "));
                            print!("\x1b[1;33massistant\x1b[0m: ");
                        }
                        AgentEvent::Done(_msg) => {
                            // Final message already streamed via Token events.
                        }
                        AgentEvent::Error(e) => {
                            println!("\n\x1b[0;31mError: {}\x1b[0m", e);
                        }
                        _ => {}
                    }
                }
                println!(); // Newline after response.

                // Wait for agent to finish and save the response.
                match agent_handle.await {
                    Ok(Ok(msg)) => {
                        session_manager.push_message(msg)?;
                    }
                    Ok(Err(e)) => {
                        eprintln!("\x1b[0;31mAgent error: {}\x1b[0m", e);
                    }
                    Err(e) => {
                        eprintln!("\x1b[0;31mTask error: {}\x1b[0m", e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye!");
                break;
            }
            Err(e) => {
                eprintln!("Input error: {}", e);
                break;
            }
        }
    }

    // Save history.
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = rl.save_history(&history_path);

    Ok(())
}

/// Handle a slash command. Returns `true` to continue the loop, `false` to exit.
fn handle_command(
    input: &str,
    session_manager: &mut SessionManager,
    tool_registry: &ToolRegistry,
    skill_indexer: &SkillIndexer,
    config: &AppConfig,
) -> Result<bool> {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

    match cmd {
        "/exit" | "/quit" | "/q" => {
            println!("Goodbye!");
            return Ok(false);
        }
        "/new" => {
            let name = if arg.is_empty() {
                format!("session-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"))
            } else {
                arg.to_string()
            };
            let session = session_manager.create_session(&name)?;
            println!("Created session: {} ({})", session.name, &session.id[..8]);
        }
        "/sessions" | "/ls" => {
            let sessions = session_manager.list_sessions();
            if sessions.is_empty() {
                println!("  No sessions.");
            } else {
                let active_id = session_manager.active_session_id().unwrap_or("");
                for (id, name, updated, count) in sessions {
                    let marker = if id == active_id { " ◀" } else { "" };
                    println!(
                        "  {} {} ({} msgs, updated {}){marker}",
                        &id[..8],
                        name,
                        count,
                        updated.format("%Y-%m-%d %H:%M")
                    );
                }
            }
        }
        "/switch" => {
            if arg.is_empty() {
                println!("Usage: /switch <session-id-prefix>");
            } else {
                // Find a session ID that starts with the given prefix.
                let sessions = session_manager.list_sessions();
                let matches: Vec<_> = sessions
                    .iter()
                    .filter(|(id, _, _, _)| id.starts_with(arg))
                    .collect();
                match matches.len() {
                    0 => println!("No session matching '{}'", arg),
                    1 => {
                        let id = matches[0].0.to_string();
                        let name = matches[0].1.to_string();
                        drop(sessions);
                        session_manager.switch_session(&id)?;
                        println!("Switched to session: {} ({})", name, &id[..8]);
                    }
                    _ => println!("Ambiguous prefix '{}', {} matches", arg, matches.len()),
                }
            }
        }
        "/tools" => {
            let names = tool_registry.list_names();
            if names.is_empty() {
                println!("  No tools registered.");
            } else {
                println!("  Available tools ({}):", names.len());
                for name in names {
                    if let Some(tool) = tool_registry.get(name) {
                        println!("    • {} — {}", name, tool.description());
                    }
                }
            }
        }
        "/skills" => {
            let index = skill_indexer.get_skill_index();
            if index.is_empty() {
                println!("  No skills loaded.");
                let skills_dir = AppConfig::data_dir().join("skills");
                println!("  Skills directory: {}", skills_dir.display());
            } else {
                println!("  Loaded skills ({}):", index.len());
                for skill in &index.skills {
                    let extras = if skill.has_sub_skills() {
                        format!(" (sub-skills: {})", skill.sub_skill_names().join(", "))
                    } else {
                        String::new()
                    };
                    println!("    {} — {}{}", skill.name, skill.description, extras);
                }
                if index.has_errors() {
                    println!("  Warnings:");
                    for err in &index.validation_errors {
                        println!("    ! {}", err);
                    }
                }
            }
        }
        "/context" => {
            let dir = if arg.is_empty() {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            } else {
                std::path::PathBuf::from(arg)
            };

            if !dir.is_dir() {
                println!("  Not a directory: {}", dir.display());
            } else {
                // Project detection.
                let mut linker = agent_core::context::ContextLinker::new();
                if let Some(project) = linker.detect_project(&dir) {
                    println!(
                        "  Project: {} ({})",
                        project.name,
                        project.primary_type().display_name()
                    );
                    println!("  Path: {}", project.path.display());
                    if let Some(remote) = &project.git_remote {
                        println!("  Git remote: {}", remote);
                    }
                    if let Some(branch) = &project.git_branch {
                        println!("  Git branch: {}", branch);
                    }
                } else {
                    println!("  Project: (none detected)");
                }

                // Git context.
                if let Some(git) = agent_core::context::ContextLinker::get_git_context(&dir) {
                    if let Some(branch) = &git.branch {
                        println!("  Branch: {}", branch);
                    }
                    if let Some(head) = &git.head_short {
                        println!("  HEAD: {}", head);
                    }
                    println!("  Dirty: {}", if git.is_dirty { "yes" } else { "no" });
                }

                // Runtime environments.
                let envs = agent_tools::env_detect::detect_environments(&dir);
                if envs.is_empty() {
                    println!("  Environments: (none detected)");
                } else {
                    println!("  Environments ({}):", envs.len());
                    for env in &envs {
                        let ver = env
                            .version
                            .as_deref()
                            .map(|v| format!(" v{}", v))
                            .unwrap_or_default();
                        println!("    {} — {}{}", env.name, env.env_type, ver);
                    }
                }
            }
        }
        "/analytics" => {
            // Load all sessions and compute analytics.
            let sessions_dir = config
                .session
                .history_dir
                .clone()
                .unwrap_or_else(|| agent_core::config::AppConfig::data_dir().join("sessions"));

            let mut all_sessions = Vec::new();
            let session_list = session_manager.list_sessions();
            for (id, _, _, _) in &session_list {
                let path = sessions_dir.join(format!("{}.json", id));
                if let Ok(session) = agent_core::session::Session::load_from(&path) {
                    all_sessions.push(session);
                }
            }

            if all_sessions.is_empty() {
                println!("  No sessions to analyze.");
            } else {
                let mut analytics = agent_analytics::Analytics::default();
                analytics.process_sessions(&all_sessions);
                let summary = agent_analytics::ReportGenerator::text_summary(&analytics);
                print!("{}", summary);
            }
        }
        "/shells" => {
            let shells = agent_pty::detect_available_shells();
            if shells.is_empty() {
                println!("  No shells detected.");
            } else {
                println!("  Detected shells:");
                for shell in &shells {
                    println!(
                        "    {} — {} ({})",
                        shell.id,
                        shell.name,
                        shell.path.display()
                    );
                }
                if let Some(default) = agent_pty::default_shell() {
                    println!("  Default: {}", default.id);
                }
            }
        }
        "/config" => {
            let toml_str = toml::to_string_pretty(config)?;
            println!("{}", toml_str);
        }
        "/clear" => {
            // Create a fresh session with the same name.
            if let Some(session) = session_manager.active_session() {
                let name = session.name.clone();
                let old_id = session.id.clone();
                session_manager.delete_session(&old_id)?;
                session_manager.create_session(&name)?;
                println!("Cleared session history.");
            }
        }
        "/help" | "/?" => {
            println!("  /new [name]    — Create a new session");
            println!("  /sessions      — List all sessions");
            println!("  /switch <id>   — Switch to a session");
            println!("  /tools         — List available tools");
            println!("  /skills        — List loaded skills");
            println!("  /context [dir] — Detect project, git, and runtime environments");
            println!("  /analytics     — Show session analytics summary");
            println!("  /shells        — List detected shells");
            println!("  /config        — Show current config");
            println!("  /clear         — Clear current session history");
            println!("  /help          — Show this help");
            println!("  /exit          — Quit");
        }
        _ => {
            println!(
                "Unknown command: {}. Type /help for available commands.",
                cmd
            );
        }
    }

    Ok(true)
}
