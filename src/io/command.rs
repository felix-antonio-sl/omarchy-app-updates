use crate::domain::*;
use std::{
    collections::BTreeMap,
    io::Read,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub timeout: Duration,
    pub cwd: Option<PathBuf>,
    pub cancellation: Option<Arc<std::sync::atomic::AtomicBool>>,
}
impl CommandSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: vec![],
            env: BTreeMap::new(),
            timeout: Duration::from_secs(60),
            cwd: None,
            cancellation: None,
        }
    }
    pub fn args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.args = args.into_iter().collect();
        self
    }
}
#[derive(Debug, Clone)]
pub struct Captured {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}
pub trait CommandBackend: Send + Sync {
    fn run(&self, spec: &CommandSpec) -> AppResult<Captured>;
}
#[derive(Clone)]
pub struct CommandRunner {
    backend: Arc<dyn CommandBackend>,
}
impl Default for CommandRunner {
    fn default() -> Self {
        Self {
            backend: Arc::new(SystemBackend),
        }
    }
}
impl CommandRunner {
    pub fn with_backend(backend: Arc<dyn CommandBackend>) -> Self {
        Self { backend }
    }
    pub fn run(&self, spec: &CommandSpec) -> AppResult<Captured> {
        self.backend.run(spec)
    }
}
struct SystemBackend;
impl CommandBackend for SystemBackend {
    fn run(&self, spec: &CommandSpec) -> AppResult<Captured> {
        if !spec.program.is_absolute() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Programa sin ruta absoluta",
            ));
        }
        let mut cmd = Command::new(&spec.program);
        const ALLOWED_ENV: &[&str] = &[
            "HOME",
            "PATH",
            "LANG",
            "LC_ALL",
            "XDG_RUNTIME_DIR",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_DATA_DIRS",
            "XDG_STATE_HOME",
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_TYPE",
            "DBUS_SESSION_BUS_ADDRESS",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "HYPRLAND_INSTANCE_SIGNATURE",
            "OMARCHY_PATH",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
            "CARGO_TARGET_DIR",
            "CARGO_BUILD_JOBS",
            "CHECKUPDATES_DB",
            "MAKEFLAGS",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_NOSYSTEM",
            "GIT_TERMINAL_PROMPT",
            "PKGDEST",
            "SRCDEST",
            "BUILDDIR",
            "LOGDEST",
            "SRCPKGDEST",
        ];
        if spec.env.keys().any(|k| !ALLOWED_ENV.contains(&k.as_str())) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Variable de proceso no permitida",
            ));
        }
        cmd.env_clear()
            .env("PATH", "/usr/share/omarchy/bin:/usr/bin")
            .process_group(0);
        for key in ALLOWED_ENV.iter().filter(|k| **k != "PATH") {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        cmd.args(&spec.args)
            .envs(&spec.env)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = cmd.spawn()?;
        let out = child.stdout.take().unwrap();
        let err = child.stderr.take().unwrap();
        let read = |mut stream: Box<dyn Read + Send>| {
            let mut bytes = Vec::new();
            let mut buf = [0; 8192];
            let mut overflow = false;
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if bytes.len() + n <= 16_777_216 {
                            bytes.extend_from_slice(&buf[..n]);
                        } else {
                            overflow = true;
                        }
                    }
                    Err(_) => break,
                }
            }
            (bytes, overflow)
        };
        let out = std::thread::spawn(move || read(Box::new(out)));
        let err = std::thread::spawn(move || read(Box::new(err)));
        let start = Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break s;
            }
            let cancelled = spec
                .cancellation
                .as_ref()
                .is_some_and(|v| v.load(std::sync::atomic::Ordering::Acquire));
            if cancelled || start.elapsed() > spec.timeout {
                // This child has its own process group. It cannot include a pre-existing app.
                let _ = Command::new("/usr/bin/kill")
                    .args(["-KILL", "--", &format!("-{}", child.id())])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                let _ = child.kill();
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(if cancelled {
                    AppError::new(
                        ErrorCode::AuthCancelled,
                        "Preparación cancelada; solo se detuvo el grupo de procesos creado por este trabajo",
                    )
                } else {
                    AppError::new(ErrorCode::IoError, "Tiempo de espera del comando agotado")
                });
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        let (stdout, over1) = out
            .join()
            .map_err(|_| AppError::new(ErrorCode::IoError, "Lectura de proceso falló"))?;
        let (stderr, over2) = err
            .join()
            .map_err(|_| AppError::new(ErrorCode::IoError, "Lectura de proceso falló"))?;
        if over1 || over2 {
            return Err(AppError::new(
                ErrorCode::IoError,
                "Salida del proceso demasiado grande",
            ));
        }
        Ok(Captured {
            code: status.code().unwrap_or(128),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}
