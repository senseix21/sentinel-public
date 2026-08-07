use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc as tokio_mpsc;
use tracing::{error, info};

#[derive(Clone)]
pub struct FileRecorderHandle {
    sender: Option<tokio_mpsc::Sender<RecorderMessage>>,
}

#[allow(dead_code)]
impl FileRecorderHandle {
    pub fn send_buffer(&self, buffer: Vec<u8>, timestamp: Option<i64>) -> Result<()> {
        if let Some(sender) = &self.sender {
            sender.blocking_send(RecorderMessage::Buffer { buffer, timestamp })?;
        }
        Ok(())
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum RecorderMessage {
    Buffer {
        buffer: Vec<u8>,
        timestamp: Option<i64>,
    },
    Shutdown,
}

#[allow(dead_code)]
pub struct FileRecorder {
    recordings_dir: PathBuf,
    current_file: Option<PathBuf>,
    file_start_time: Option<DateTime<Utc>>,
    sender: Option<tokio_mpsc::Sender<RecorderMessage>>,
}

#[allow(dead_code)]
impl FileRecorder {
    pub fn new(recordings_dir: PathBuf) -> Self {
        Self {
            recordings_dir,
            current_file: None,
            file_start_time: None,
            sender: None,
        }
    }

    pub fn start(&mut self) -> FileRecorderHandle {
        // Create recordings directory if it doesn't exist
        if let Err(e) = std::fs::create_dir_all(&self.recordings_dir) {
            error!("Failed to create recordings directory: {}", e);
        }

        let (tx, rx) = tokio_mpsc::channel::<RecorderMessage>(1000);
        self.sender = Some(tx.clone());

        // Start recorder task
        let recordings_dir = self.recordings_dir.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::recorder_task(rx, recordings_dir).await {
                error!("File recorder task error: {}", e);
            }
        });

        FileRecorderHandle { sender: Some(tx) }
    }

    async fn recorder_task(
        mut rx: tokio_mpsc::Receiver<RecorderMessage>,
        recordings_dir: PathBuf,
    ) -> Result<()> {
        let mut current_file: Option<tokio::fs::File> = None;
        let _buffer_queue: Vec<u8> = Vec::new();

        while let Some(msg) = rx.recv().await {
            match msg {
                RecorderMessage::Buffer {
                    buffer,
                    timestamp: _,
                } => {
                    // Write buffer to file
                    if let Some(ref mut file) = current_file {
                        if let Err(e) = file.write_all(&buffer).await {
                            error!("Failed to write buffer to file: {}", e);
                        }
                    } else {
                        // Create new file
                        let timestamp_str = Utc::now().format("%Y%m%d_%H%M%S");
                        let file_path =
                            recordings_dir.join(format!("stream_{}.mkv", timestamp_str));
                        current_file = match tokio::fs::File::create(&file_path).await {
                            Ok(file) => {
                                info!("Started new recording file: {}", file_path.display());
                                Some(file)
                            }
                            Err(e) => {
                                error!("Failed to create recording file: {}", e);
                                None
                            }
                        };
                    }
                }
                RecorderMessage::Shutdown => {
                    info!("Shutting down file recorder");
                    break;
                }
            }
        }

        // Flush any remaining data
        if let Some(mut file) = current_file {
            if let Err(e) = file.flush().await {
                error!("Failed to flush file: {}", e);
            }
        }

        Ok(())
    }

    pub fn create_timestamped_filename(&self, timestamp: DateTime<Utc>) -> PathBuf {
        self.recordings_dir
            .join(format!("stream_{}.mkv", timestamp.format("%Y%m%d_%H%M%S")))
    }
}

impl Drop for FileRecorderHandle {
    fn drop(&mut self) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(RecorderMessage::Shutdown);
        }
    }
}
