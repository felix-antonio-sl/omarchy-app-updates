use crate::{
    alpm::transaction::PackageTransaction, domain::*, helper::*, planner::plan_digest,
    policy::Policy,
};
use std::io::{BufRead, Read, Write};
pub struct HelperSession {
    transaction: Box<dyn PackageTransaction>,
    policy: Policy,
    now: u64,
    sink: std::sync::Arc<dyn EventSink>,
    deadline: Option<crate::helper::control::DecisionDeadline>,
    started: std::time::Instant,
}
impl HelperSession {
    pub fn new(transaction: impl PackageTransaction + 'static, policy: Policy, now: u64) -> Self {
        Self {
            transaction: Box::new(transaction),
            policy,
            now,
            sink: std::sync::Arc::new(SilentSink),
            deadline: None,
            started: std::time::Instant::now(),
        }
    }
    pub fn with_sink(mut self, sink: std::sync::Arc<dyn EventSink>) -> Self {
        self.sink = sink;
        self
    }
    pub fn with_deadline(mut self, deadline: crate::helper::control::DecisionDeadline) -> Self {
        self.deadline = Some(deadline);
        self
    }
    fn read(&self, input: &mut dyn BufRead) -> AppResult<HelperRequest> {
        if let Some(d) = &self.deadline {
            d.arm();
        }
        read_request(input)
    }
    pub fn run(&mut self, input: &mut dyn BufRead, output: &mut dyn Write) -> AppResult<()> {
        let result = self.run_inner(input, output);
        let released = self.transaction.release();
        result.and(released)
    }
    fn run_inner(&mut self, input: &mut dyn BufRead, output: &mut dyn Write) -> AppResult<()> {
        let first = self.read(input)?;
        let HelperRequest::Prepare {
            operation_id,
            plan: original,
        } = first
        else {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "La primera solicitud debe preparar una transacción",
            ));
        };
        if operation_id.0.is_empty()
            || operation_id.0.len() > 100
            || !operation_id
                .0
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(AppError::new(ErrorCode::InvalidInput, "Operación inválida"));
        }
        if !self.policy.coverage.verified() || original.policy_digest != self.policy.digest() {
            return Err(AppError::new(
                ErrorCode::CoverageNeedsReview,
                "Cambió la política de exclusiones",
            ));
        }
        if self.now < original.created_at
            || self.now >= original.expires_at
            || original.expires_at.saturating_sub(original.created_at) > 900
        {
            return Err(AppError::new(ErrorCode::PlanExpired, "El plan venció"));
        }
        if original.digest != plan_digest(&original) {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "Digest del plan inválido",
            ));
        }
        loop {
            let (prepared, pending) = match self.transaction.prepare(&original) {
                Ok(plan) => {
                    if plan.digest != plan_digest(&plan) {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "El ejecutor preparó un alcance inválido",
                        ));
                    }
                    let event = if plan.digest == original.digest {
                        HelperEvent::Prepared {
                            operation_id: operation_id.clone(),
                            plan: plan.clone(),
                        }
                    } else {
                        HelperEvent::RevisedPlan {
                            operation_id: operation_id.clone(),
                            plan: plan.clone(),
                        }
                    };
                    write_event(&event, output)?;
                    (Some(plan), None)
                }
                Err(e) => {
                    if let Some(question) = self.transaction.pending_question() {
                        write_event(
                            &HelperEvent::Question {
                                operation_id: operation_id.clone(),
                                question: question.clone(),
                                digest: original.digest.clone(),
                            },
                            output,
                        )?;
                        (None, Some(question))
                    } else {
                        return Err(e);
                    }
                }
            };
            let request = self.read(input)?;
            if request.operation_id() != &operation_id {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "Respuesta de otra operación",
                ));
            }
            match request {
                HelperRequest::Prepare { .. } => {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        "Una operación no admite otra preparación",
                    ));
                }
                HelperRequest::Abort { .. } => {
                    return Err(AppError::new(
                        ErrorCode::AuthCancelled,
                        "Operación cancelada",
                    ));
                }
                HelperRequest::Answer {
                    question_id,
                    answer,
                    ..
                } => {
                    let question = pending.as_ref().ok_or_else(|| {
                        AppError::new(ErrorCode::InvalidInput, "No hay pregunta pendiente")
                    })?;
                    if question.id != question_id {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            "Respuesta de una pregunta anterior",
                        ));
                    }
                    if answer == DecisionValue::Reject {
                        return Err(AppError::new(
                            ErrorCode::AuthCancelled,
                            "Decisión rechazada",
                        ));
                    }
                    match &answer {
                        DecisionValue::Approve
                            if question.kind != "provider"
                                && question.choices.iter().any(|c| c == "approve") => {}
                        DecisionValue::SelectProvider(p)
                            if question.kind == "provider" && question.choices.contains(p) => {}
                        _ => {
                            return Err(AppError::new(
                                ErrorCode::InvalidInput,
                                "Opción fuera de la pregunta",
                            ));
                        }
                    }
                    self.transaction.answer(&question_id, &answer)?;
                }
                HelperRequest::Confirm { digest, .. } => {
                    let now = self.now.saturating_add(self.started.elapsed().as_secs());
                    if now >= original.expires_at {
                        return Err(AppError::new(
                            ErrorCode::PlanExpired,
                            "El plan venció mientras se esperaba",
                        ));
                    }
                    if pending.is_some() {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "Falta responder una decisión",
                        ));
                    }
                    let plan = prepared.as_ref().ok_or_else(|| {
                        AppError::new(ErrorCode::PlanChanged, "No hay un alcance preparado")
                    })?;
                    if digest != plan.digest {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "Cambió el alcance; se requiere aprobar el plan revisado",
                        ));
                    }
                    let results = self.transaction.commit(&digest, self.sink.as_ref())?;
                    write_event(
                        &HelperEvent::Result {
                            operation_id,
                            results,
                        },
                        output,
                    )?;
                    return Ok(());
                }
            }
        }
    }
}
pub fn read_request(input: &mut dyn BufRead) -> AppResult<HelperRequest> {
    let mut bytes = vec![];
    let size = input
        .take((crate::protocol::MAX_REQUEST + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::TimedOut {
                AppError::new(
                    ErrorCode::AuthCancelled,
                    "Venció la espera de decisión; no se instala nada",
                )
            } else {
                e.into()
            }
        })?;
    if size == 0 {
        return Err(AppError::new(
            ErrorCode::AuthCancelled,
            "Se cerró el canal antes de confirmar",
        ));
    }
    if bytes.len() > crate::protocol::MAX_REQUEST || !bytes.ends_with(b"\n") {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Solicitud administrativa demasiado grande o incompleta",
        ));
    }
    serde_json::from_slice(&bytes).map_err(Into::into)
}
