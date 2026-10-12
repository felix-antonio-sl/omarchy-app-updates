use crate::{
    domain::*,
    helper::{DecisionValue, QuestionId},
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserDecision {
    pub question_id: QuestionId,
    pub digest: String,
    pub value: DecisionValue,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionGate {
    pub operation_id: OperationId,
    pub question_id: QuestionId,
    pub digest: String,
    pub expires_at: u64,
    pub choices: Vec<DecisionValue>,
}
impl DecisionGate {
    pub fn validate(
        &self,
        operation: &OperationId,
        decision: &UserDecision,
        now: u64,
    ) -> AppResult<()> {
        if &self.operation_id != operation
            || self.question_id != decision.question_id
            || self.digest != decision.digest
        {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "La decisión pertenece a otro alcance, pregunta u operación",
            ));
        }
        if now >= self.expires_at {
            return Err(AppError::new(
                ErrorCode::AuthCancelled,
                "La decisión venció",
            ));
        }
        if !self.choices.contains(&decision.value) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Respuesta fuera de las opciones admitidas",
            ));
        }
        Ok(())
    }
}

use super::worker::JobHandle;
use crate::paths::Paths;
use std::{
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
fn socket_path(operation: &OperationId, paths: &Paths) -> AppResult<PathBuf> {
    super::valid_id(operation)?;
    Ok(paths
        .runtime_dir
        .join("app-updates")
        .join(format!("{}.sock", operation.0)))
}
pub struct ControlServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    receiver: mpsc::Receiver<UserDecision>,
}
impl ControlServer {
    pub fn start(job: JobHandle) -> AppResult<Self> {
        let path = socket_path(&job.record().operation.id, &job.paths)?;
        crate::store::private_dir(path.parent().unwrap())?;
        if path.exists() {
            return Err(AppError::new(
                ErrorCode::Busy,
                "Ya existe un canal de este trabajo",
            ));
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let (tx, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            while !stop_thread.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let response = (|| -> AppResult<()> {
                            let cred = rustix::net::sockopt::socket_peercred(&stream)
                                .map_err(std::io::Error::from)?;
                            if cred.uid != rustix::process::geteuid() {
                                return Err(AppError::new(
                                    ErrorCode::InvalidInput,
                                    "Operador ajeno al trabajo",
                                ));
                            }
                            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                            let mut bytes = vec![];
                            Read::by_ref(&mut stream)
                                .take(crate::protocol::MAX_REQUEST as u64 + 1)
                                .read_to_end(&mut bytes)?;
                            if bytes.len() > crate::protocol::MAX_REQUEST {
                                return Err(AppError::new(
                                    ErrorCode::InvalidInput,
                                    "Decisión demasiado grande",
                                ));
                            }
                            let decision: UserDecision = serde_json::from_slice(&bytes)?;
                            job.mutate(|record|{
                        let gate=record.pending.as_ref().ok_or_else(||AppError::new(ErrorCode::PlanChanged,"No hay una decisión pendiente; la instalación confirmada solo se observa"))?;
                        gate.validate(&record.operation.id,&decision,crate::now())?;
                        tx.send(decision.clone()).map_err(|_|AppError::new(ErrorCode::IoError,"Trabajo cerrado"))?;record.pending=None;Ok(())
                    })?;
                            if decision.value == DecisionValue::Reject {
                                job.cancelled.store(true, Ordering::Release);
                            }
                            Ok(())
                        })();
                        let _ = stream.write_all(&serde_json::to_vec(&response).unwrap());
                        let _ = stream.flush();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(30))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            path,
            stop,
            thread: Some(thread),
            receiver,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn wait(&self, gate: &DecisionGate) -> AppResult<DecisionValue> {
        loop {
            let now = crate::now();
            if now >= gate.expires_at {
                return Err(AppError::new(
                    ErrorCode::AuthCancelled,
                    "Venció la espera de decisión; no se instala este alcance",
                ));
            }
            match self.receiver.recv_timeout(Duration::from_secs(1)) {
                Ok(d) if d.question_id == gate.question_id => {
                    gate.validate(&gate.operation_id, &d, crate::now())?;
                    return Ok(d.value);
                }
                Ok(d) if d.value == DecisionValue::Reject => {
                    return Err(AppError::new(
                        ErrorCode::AuthCancelled,
                        "Preparación cancelada",
                    ));
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(_) => {
                    return Err(AppError::new(
                        ErrorCode::IoError,
                        "Canal de decisiones cerrado",
                    ));
                }
            }
        }
    }
}
impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}
pub fn respond(operation: &OperationId, decision: &UserDecision, paths: &Paths) -> AppResult<()> {
    let job = super::load_job(operation, paths)?;
    job.pending
        .as_ref()
        .ok_or_else(|| AppError::new(ErrorCode::PlanChanged, "No hay pregunta pendiente"))?
        .validate(operation, decision, crate::now())?;
    let path = socket_path(operation, paths)?;
    let meta = std::fs::symlink_metadata(&path)?;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    if !meta.file_type().is_socket()
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Canal de decisión no privado",
        ));
    }
    let mut stream = UnixStream::connect(path)?;
    if rustix::net::sockopt::socket_peercred(&stream)
        .map_err(std::io::Error::from)?
        .uid
        != rustix::process::geteuid()
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Canal de otro usuario",
        ));
    }
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(&serde_json::to_vec(decision)?)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut bytes = vec![];
    stream
        .take(crate::protocol::MAX_REQUEST as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > crate::protocol::MAX_REQUEST {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Respuesta demasiado grande",
        ));
    }
    serde_json::from_slice::<AppResult<()>>(&bytes)?
}
