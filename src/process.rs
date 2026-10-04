use anyhow::{Context, Result, bail};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
};

#[derive(Clone)]
pub struct Runner {
    pub home: PathBuf,
}

impl Runner {
    pub fn new() -> Self {
        Self {
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default(),
        }
    }

    pub async fn signature(&self) -> Option<String> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
        let root = runtime.join("hypr");
        let mut candidates = Vec::new();
        if let Ok(sig) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
            && root.join(&sig).join(".socket2.sock").exists()
        {
            return Some(sig);
        }
        let mut dirs = tokio::fs::read_dir(&root).await.ok()?;
        while let Ok(Some(dir)) = dirs.next_entry().await {
            if dir.path().join(".socket2.sock").exists() {
                let modified = dir.metadata().await.ok().and_then(|m| m.modified().ok());
                candidates.push((modified, dir.file_name().to_string_lossy().into_owned()));
            }
        }
        candidates.sort();
        candidates.pop().map(|(_, name)| name)
    }

    pub async fn command(&self, argv: &[String], detached: bool) -> Result<Command> {
        let mut command = Command::new(argv.first().context("empty command")?);
        command.args(&argv[1..]);
        // A dedicated override makes the real command boundary available to the host harness.
        let path = std::env::var("VINCENT_DECK_PATH").unwrap_or_else(|_| {
            format!(
                "/usr/share/omarchy/bin:{}:{}",
                self.home.join(".local/bin").display(),
                std::env::var("PATH").unwrap_or_default()
            )
        });
        command.env("PATH", path).stdin(Stdio::null());
        if let Some(sig) = self.signature().await {
            command.env("HYPRLAND_INSTANCE_SIGNATURE", sig);
        }
        if detached {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
        // Only async-signal-safe libc work between fork and exec. State queries
        // and short audio writes also get their own session, but retain captured
        // output and their separate timeout policy.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(command)
    }

    pub async fn launch(&self, argv: &[String]) -> Result<Child> {
        self.command(argv, true)
            .await?
            .spawn()
            .with_context(|| format!("launch {argv:?}"))
    }

    /// Timed, bounded state queries; cancellation kills and reaps the child.
    pub async fn query(&self, args: &[&str]) -> Result<String> {
        let argv = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut command = self.command(&argv, false).await?;
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().with_context(|| format!("query {args:?}"))?;
        let mut stdout = child.stdout.take().context("query stdout")?.take(262_145);
        let work = async {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).await?;
            if bytes.len() > 262_144 {
                bail!("query output exceeds 256 KiB")
            }
            let status = child.wait().await?;
            if !status.success() {
                bail!("{args:?} exited {status}")
            }
            Ok(String::from_utf8(bytes)?)
        };
        match tokio::time::timeout(Duration::from_secs(2), work).await {
            Ok(Ok(output)) => Ok(output),
            result => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                match result {
                    Ok(Err(error)) => Err(error),
                    _ => bail!("{args:?} timed out after 2 seconds"),
                }
            }
        }
    }
}

impl Default for Runner {
    fn default() -> Self {
        Self::new()
    }
}
