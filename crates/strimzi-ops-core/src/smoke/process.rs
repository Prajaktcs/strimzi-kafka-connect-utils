use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot run pipeline smoke: {0}")]
    Assertion(String),
    #[error("cannot access smoke evidence {path}: {source}")]
    Evidence {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot execute {program}; install it and ensure it is on PATH: {source}")]
    Execute {
        program: String,
        source: std::io::Error,
    },
    #[error("pipeline smoke command {label} failed; inspect {evidence}")]
    Command { label: String, evidence: PathBuf },
    #[error("pipeline smoke command {label} exceeded its deadline; inspect {evidence}")]
    Timeout { label: String, evidence: PathBuf },
}

pub(super) type Result<T, E = Error> = std::result::Result<T, E>;

pub(super) fn assertion(message: impl Into<String>) -> Error {
    Error::Assertion(message.into())
}

pub(super) fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    private_file(path)?
        .write_all(bytes)
        .map_err(|source| Error::Evidence {
            path: path.to_path_buf(),
            source,
        })
}

fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|source| Error::Evidence {
        path: path.to_path_buf(),
        source,
    })
}

pub(super) struct Runner {
    pub directory: PathBuf,
    sequence: usize,
}

impl Runner {
    pub fn new(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(directory.parent().unwrap_or_else(|| Path::new("."))).map_err(
            |source| Error::Evidence {
                path: directory.clone(),
                source,
            },
        )?;
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&directory)
            .map_err(|source| Error::Evidence {
                path: directory.clone(),
                source,
            })?;
        Ok(Self {
            directory,
            sequence: 0,
        })
    }

    pub fn command(
        &mut self,
        label: &str,
        program: &str,
        args: &[&str],
        input: Option<&str>,
        timeout: Duration,
    ) -> Result<String> {
        if input.is_some_and(|input| input.len() > 8192) {
            return Err(assertion(
                "smoke SQL/manifest input exceeds the bounded 8 KiB limit",
            ));
        }
        self.sequence += 1;
        let prefix = format!("{:04}-{label}", self.sequence);
        let stdout = self.directory.join(format!("{prefix}.stdout"));
        let stderr = self.directory.join(format!("{prefix}.stderr"));
        let deadline = Instant::now() + timeout;
        let mut child = Command::new(program)
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(private_file(&stdout)?)
            .stderr(private_file(&stderr)?)
            .spawn()
            .map_err(|source| Error::Execute {
                program: program.to_owned(),
                source,
            })?;
        if let (Some(input), Some(mut pipe)) = (input, child.stdin.take()) {
            // Smoke inputs are bounded SQL statements or one connector manifest.
            if pipe.write_all(input.as_bytes()).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Command {
                    label: label.to_owned(),
                    evidence: self.directory.clone(),
                });
            }
        }
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break,
                Ok(Some(_)) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Command {
                        label: label.to_owned(),
                        evidence: self.directory.clone(),
                    });
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Timeout {
                        label: label.to_owned(),
                        evidence: self.directory.clone(),
                    });
                }
                Ok(None) => thread::sleep(Duration::from_millis(100)),
            }
        }
        fs::read_to_string(&stdout).map_err(|source| Error::Evidence {
            path: stdout,
            source,
        })
    }
}
