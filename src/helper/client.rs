use super::*;
use crate::paths::Paths;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
pub fn authorization_error(code: i32) -> AppError {
    if code == 126 {
        AppError::new(ErrorCode::AuthCancelled, "Autorización cancelada")
    } else {
        AppError::new(
            ErrorCode::AuthUnavailable,
            "No se completó la autorización gráfica; comprueba el agente polkit",
        )
    }
}
pub struct HelperClient {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    operation_id: OperationId,
}
impl HelperClient {
    pub fn start(operation_id: &OperationId, paths: &Paths) -> AppResult<Self> {
        super::validate::validate_helper_path(&paths.helper)?;
        super::validate::trusted_root_path(std::path::Path::new("/usr/bin/pkexec"))?;
        let mut command = Command::new("/usr/bin/pkexec");
        command
            .args(["--disable-internal-agent", super::validate::HELPER])
            .env_clear()
            .env("PATH", "/usr/bin")
            .env("LANG", "C.UTF-8");
        for key in [
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().ok_or_else(|| {
            AppError::new(ErrorCode::IoError, "Canal administrativo no disponible")
        })?;
        let output = BufReader::new(child.stdout.take().ok_or_else(|| {
            AppError::new(ErrorCode::IoError, "Canal administrativo no disponible")
        })?);
        Ok(Self {
            child,
            input,
            output,
            operation_id: operation_id.clone(),
        })
    }
    fn send(&mut self, request: HelperRequest) -> AppResult<()> {
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');
        if bytes.len() > crate::protocol::MAX_REQUEST {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Plan administrativo demasiado grande",
            ));
        }
        if let Err(e) = self
            .input
            .write_all(&bytes)
            .and_then(|_| self.input.flush())
        {
            if e.kind() == std::io::ErrorKind::BrokenPipe
                && let Ok(Some(status)) = self.child.try_wait()
            {
                return Err(authorization_error(status.code().unwrap_or(127)));
            }
            return Err(e.into());
        }
        Ok(())
    }
    pub fn prepare(&mut self, plan: &UpdatePlan) -> AppResult<HelperEvent> {
        self.send(HelperRequest::Prepare {
            operation_id: self.operation_id.clone(),
            plan: plan.clone(),
        })?;
        self.next_event()
    }
    pub fn confirm(&mut self, digest: &str) -> AppResult<()> {
        self.send(HelperRequest::Confirm {
            operation_id: self.operation_id.clone(),
            digest: digest.into(),
        })
    }
    pub fn answer(&mut self, question: &QuestionId, value: &DecisionValue) -> AppResult<()> {
        self.send(HelperRequest::Answer {
            operation_id: self.operation_id.clone(),
            question_id: question.clone(),
            answer: value.clone(),
        })
    }
    pub fn abort(&mut self) -> AppResult<()> {
        self.send(HelperRequest::Abort {
            operation_id: self.operation_id.clone(),
        })
    }
    pub fn next_event(&mut self) -> AppResult<HelperEvent> {
        let mut bytes = vec![];
        let n = self
            .output
            .by_ref()
            .take((crate::protocol::MAX_REQUEST + 1) as u64)
            .read_until(b'\n', &mut bytes)?;
        if n == 0 {
            let status = self.child.wait()?;
            return Err(if status.success() {
                AppError::new(
                    ErrorCode::TransactionFailed,
                    "El ejecutor terminó sin resultado verificable",
                )
            } else {
                authorization_error(status.code().unwrap_or(127))
            });
        }
        if bytes.len() > crate::protocol::MAX_REQUEST || !bytes.ends_with(b"\n") {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Respuesta administrativa inválida",
            ));
        }
        let event: HelperEvent = serde_json::from_slice(&bytes)?;
        let operation = match &event {
            HelperEvent::Prepared { operation_id, .. }
            | HelperEvent::RevisedPlan { operation_id, .. }
            | HelperEvent::Question { operation_id, .. }
            | HelperEvent::Result { operation_id, .. } => Some(operation_id),
            HelperEvent::Progress { data } => Some(&data.operation_id),
            HelperEvent::Error { operation_id, .. } => operation_id.as_ref(),
        };
        if operation.is_some_and(|id| id != &self.operation_id) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Evento de otra operación",
            ));
        }
        Ok(event)
    }
}
