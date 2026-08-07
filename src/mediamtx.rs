use anyhow::{Context, Result};
use chrono::Utc;
use std::path::PathBuf;
use tokio::process::Command as TokioCommand;
use tracing::{info, warn};

pub struct MediaMTX {
    executable_path: PathBuf,
    config_path: PathBuf,
    process: Option<tokio::process::Child>,
}

#[allow(dead_code)]
impl MediaMTX {
    pub fn new(executable_path: Option<PathBuf>, config_path: PathBuf) -> Self {
        let executable_path = executable_path.unwrap_or_else(|| {
            // Try to find mediamtx in the system PATH
            match which::which("mediamtx") {
                Ok(path) => path,
                Err(_) => PathBuf::from("mediamtx"),
            }
        });

        Self {
            executable_path,
            config_path,
            process: None,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        info!(
            "Starting MediaMTX server at: {}",
            self.executable_path.display()
        );
        info!("Using config: {}", self.config_path.display());

        // Ensure config file exists
        if !self.config_path.exists() {
            self.create_default_config().await?;
        }

        // Start mediamtx process
        let mut cmd = TokioCommand::new(&self.executable_path);
        cmd.arg("-c").arg(&self.config_path);

        self.process = Some(cmd.spawn().context("Failed to start MediaMTX process")?);

        info!("MediaMTX server started successfully");

        // Wait a bit to ensure the server is ready
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut process) = self.process.take() {
            info!("Stopping MediaMTX server");

            // Try to shut down gracefully
            if let Err(e) = process.start_kill() {
                warn!("Failed to kill MediaMTX process: {}", e);
            }

            // Wait for the process to exit
            let status = process.wait().await?;
            info!("MediaMTX server stopped with status: {}", status);
        }

        Ok(())
    }

    pub async fn restart(&mut self) -> Result<()> {
        info!("Restarting MediaMTX server");
        self.stop().await?;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        self.start().await
    }

    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut process) = self.process {
            process
                .try_wait()
                .map(|status| status.is_none())
                .unwrap_or(false)
        } else {
            false
        }
    }

    async fn create_default_config(&self) -> Result<()> {
        let config_content = r#"
# MediaMTX configuration
# This is a default configuration for RTSP streaming

paths:
  record:
    enable: true
    recordPath: ./recordings
  live:
    enable: true
    # Record the stream to disk
    record: true
    # Output format (mkv is better for recording)
    outputFormat: mkv
    # When to rotate the file (never for continuous recording)
    fileRotate: never

# RTSP server settings
rtsp:
  # Listen address
  listenAddr: :8554
  # TLS (optional)
  # enable: true
  # listenAddr: :8555
  # certFile: cert.pem
  # keyFile: key.pem

# HTTP server settings
http:
  # Listen address
  listenAddr: :8181
  # Enable web UI
  enableUI: true

# Logging settings
logLevel: info
"#;

        info!(
            "Creating default MediaMTX config at: {}",
            self.config_path.display()
        );

        // Create recordings directory if it doesn't exist
        if let Some(parent) = self.config_path.parent() {
            if !parent.exists() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .context("Failed to create config directory")?;
            }
        }

        tokio::fs::write(&self.config_path, config_content)
            .await
            .context("Failed to write default MediaMTX config")?;

        info!("Default MediaMTX config created successfully");
        Ok(())
    }

    pub async fn get_status(&mut self) -> Result<MediaMTXStatus> {
        let is_running = self.is_running();
        let uptime = if is_running {
            let start_time = self.get_start_time().unwrap_or(Utc::now());
            Some(Utc::now() - start_time)
        } else {
            None
        };

        Ok(MediaMTXStatus { is_running, uptime })
    }

    fn get_start_time(&mut self) -> Option<chrono::DateTime<chrono::Utc>> {
        // In a real implementation, you might track the start time
        // For now, just return None
        None
    }
}

impl Drop for MediaMTX {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            // Try to shut down cleanly
            let _ = process.start_kill();
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MediaMTXStatus {
    pub is_running: bool,
    pub uptime: Option<chrono::Duration>,
}

#[allow(dead_code)]
impl MediaMTXStatus {
    pub fn is_running(&self) -> bool {
        self.is_running
    }

    pub fn uptime_seconds(&self) -> Option<i64> {
        self.uptime.map(|d| d.num_seconds())
    }
}
