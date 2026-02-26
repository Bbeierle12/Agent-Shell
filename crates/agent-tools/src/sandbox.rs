use agent_core::config::{AppConfig, SandboxMode};
use agent_core::error::AgentError;
use tokio::process::Command;
use tracing::debug;

/// Unified executor that dispatches to Docker or unsafe (direct) execution.
pub struct SandboxExecutor {
    mode: SandboxMode,
    docker_image: String,
    timeout_secs: u64,
    memory_limit: Option<u64>,
    work_dir: String,
}

impl SandboxExecutor {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            mode: config.sandbox.mode,
            docker_image: config.sandbox.docker_image.clone(),
            timeout_secs: config.sandbox.timeout_secs,
            memory_limit: config.sandbox.memory_limit,
            work_dir: config.sandbox.work_dir.clone(),
        }
    }

    /// Execute a command string, returning stdout+stderr.
    pub async fn exec_shell(&self, command: &str) -> Result<ExecResult, AgentError> {
        match self.mode {
            SandboxMode::Unsafe => self.exec_shell_unsafe(command).await,
            SandboxMode::Docker => self.exec_shell_docker(command).await,
        }
    }

    /// Execute Python code, returning stdout+stderr.
    pub async fn exec_python(&self, code: &str) -> Result<ExecResult, AgentError> {
        match self.mode {
            SandboxMode::Unsafe => self.exec_python_unsafe(code).await,
            SandboxMode::Docker => self.exec_python_docker(code).await,
        }
    }

    // ── Unsafe (direct) execution ──────────────────────────────────────

    async fn exec_shell_unsafe(&self, command: &str) -> Result<ExecResult, AgentError> {
        debug!("Executing shell command (unsafe mode): {}", command);
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs),
            Command::new("bash").arg("-c").arg(command).output(),
        )
        .await
        .map_err(|_| AgentError::Sandbox("Command timed out".into()))?
        .map_err(|e| AgentError::Sandbox(format!("Failed to spawn: {}", e)))?;

        Ok(ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    async fn exec_python_unsafe(&self, code: &str) -> Result<ExecResult, AgentError> {
        debug!("Executing Python code (unsafe mode)");
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs),
            Command::new("python3").arg("-c").arg(code).output(),
        )
        .await
        .map_err(|_| AgentError::Sandbox("Python execution timed out".into()))?
        .map_err(|e| AgentError::Sandbox(format!("Failed to spawn python3: {}", e)))?;

        Ok(ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    // ── Docker sandboxed execution ─────────────────────────────────────

    async fn exec_shell_docker(&self, command: &str) -> Result<ExecResult, AgentError> {
        debug!("Executing shell command (Docker mode): {}", command);
        self.docker_run(&["bash", "-c", command]).await
    }

    async fn exec_python_docker(&self, code: &str) -> Result<ExecResult, AgentError> {
        debug!("Executing Python code (Docker mode)");
        // Pipe code via stdin instead of CLI arg to avoid OS ARG_MAX limits
        // on large LLM-generated scripts.
        self.docker_run_stdin(&["python3", "-"], code).await
    }

    /// Build the common Docker argument list (shared between `docker_run` and `docker_run_stdin`).
    fn docker_base_args(&self) -> (Vec<String>, String) {
        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(),
            "--network=none".to_string(),
            "--read-only".to_string(),
        ];

        if let Some(mem) = self.memory_limit {
            args.push("--memory".to_string());
            args.push(format!("{}m", mem / (1024 * 1024)));
        }

        args.push("--stop-timeout".to_string());
        args.push(format!("{}s", self.timeout_secs));

        args.push("-w".to_string());
        args.push(self.work_dir.clone());

        // Add tmpfs for /tmp since we're read-only
        args.push("--tmpfs".to_string());
        args.push("/tmp:rw,noexec,nosuid,size=64m".to_string());
        args.push("--tmpfs".to_string());
        args.push("/workspace:rw,noexec,nosuid,size=64m".to_string());

        let image = self.docker_image.clone();
        (args, image)
    }

    /// Run a command inside a Docker container using `docker run`.
    /// Uses the CLI for simplicity in MVP; can migrate to bollard for streaming later.
    async fn docker_run(&self, cmd: &[&str]) -> Result<ExecResult, AgentError> {
        let (mut docker_args, image) = self.docker_base_args();
        docker_args.push(image);
        for c in cmd {
            docker_args.push(c.to_string());
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs + 5), // grace period
            Command::new("docker").args(&docker_args).output(),
        )
        .await
        .map_err(|_| AgentError::Sandbox("Docker execution timed out".into()))?
        .map_err(|e| AgentError::Sandbox(format!("Failed to run docker: {}", e)))?;

        Ok(ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    /// Run a command inside Docker, piping `input` to the process via stdin.
    ///
    /// This avoids OS `ARG_MAX` limits when the payload (e.g. a large Python
    /// script generated by an LLM) would exceed the maximum CLI argument size.
    async fn docker_run_stdin(
        &self,
        cmd: &[&str],
        input: &str,
    ) -> Result<ExecResult, AgentError> {
        let (mut docker_args, image) = self.docker_base_args();
        // -i keeps stdin open so we can pipe data into the container.
        docker_args.push("-i".to_string());
        docker_args.push(image);
        for c in cmd {
            docker_args.push(c.to_string());
        }

        use tokio::io::AsyncWriteExt;

        let mut child = Command::new("docker")
            .args(&docker_args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| AgentError::Sandbox(format!("Failed to spawn docker: {}", e)))?;

        // Write the code to stdin, then close stdin so the process can finish.
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input.as_bytes())
                .await
                .map_err(|e| AgentError::Sandbox(format!("Failed to write to docker stdin: {}", e)))?;
            // drop closes the pipe
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs + 5),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| AgentError::Sandbox("Docker execution timed out".into()))?
        .map_err(|e| AgentError::Sandbox(format!("Failed to run docker: {}", e)))?;

        Ok(ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}

/// Result of executing code or a shell command.
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl ExecResult {
    /// Format as a human-readable string for the model.
    pub fn to_display_string(&self) -> String {
        let mut parts = Vec::new();
        if !self.stdout.is_empty() {
            parts.push(format!("stdout:\n{}", self.stdout));
        }
        if !self.stderr.is_empty() {
            parts.push(format!("stderr:\n{}", self.stderr));
        }
        parts.push(format!("exit_code: {}", self.exit_code));
        parts.join("\n")
    }
}
