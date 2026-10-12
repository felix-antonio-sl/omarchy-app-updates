use omarchy_app_updates::{
    alpm::{self, transaction::AlpmTransaction},
    domain::*,
    execution::EventSink,
    helper::{
        self,
        control::{DeadlineRead, DecisionDeadline},
        session::{HelperSession, read_request},
        validate::{caller, validate_helper_path},
    },
};
use std::{
    io::{self, BufReader, Cursor, Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone)]
struct Output(Arc<Mutex<io::Stdout>>);
impl Write for Output {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("Salida cerrada"))?
            .write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("Salida cerrada"))?
            .flush()
    }
}
impl EventSink for Output {
    fn send(&self, event: OperationEvent) -> AppResult<()> {
        helper::write_event(
            &helper::HelperEvent::Progress { data: event },
            &mut self.clone(),
        )
    }
}
fn run(output: &mut Output, operation_id: &mut Option<OperationId>) -> AppResult<()> {
    alpm::verify_abi()?;
    validate_helper_path(&std::env::current_exe()?)?;
    let caller = caller()?;
    let deadline = DecisionDeadline::new(Duration::from_secs(300));
    let input = std::fs::File::from(rustix::io::dup(io::stdin()).map_err(io::Error::from)?);
    let mut input = BufReader::new(DeadlineRead::new(input, deadline.clone()));
    let first = read_request(&mut input)?;
    if !matches!(first, helper::HelperRequest::Prepare { .. }) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "La primera solicitud debe ser Prepare",
        ));
    }
    *operation_id = Some(first.operation_id().clone());
    let transaction = AlpmTransaction::system(
        caller,
        first.operation_id().clone(),
        Arc::new(output.clone()),
    )?;
    let policy = transaction.policy();
    let mut prefix = serde_json::to_vec(&first)?;
    prefix.push(b'\n');
    let mut input = BufReader::new(Cursor::new(prefix).chain(input));
    HelperSession::new(transaction, policy, omarchy_app_updates::now())
        .with_sink(Arc::new(output.clone()))
        .with_deadline(deadline)
        .run(&mut input, output)
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args == ["--version"] {
        println!(
            "{}",
            serde_json::json!({"protocol":1,"version":env!("CARGO_PKG_VERSION"),"source_digest":env!("APP_UPDATES_SOURCE_DIGEST"),"alpm_compiled":env!("APP_UPDATES_ALPM_VERSION"),"alpm_runtime":alpm::runtime_version()})
        );
        return;
    }
    let mut output = Output(Arc::new(Mutex::new(io::stdout())));
    let mut operation_id = None;
    let result = if args.is_empty() {
        run(&mut output, &mut operation_id)
    } else {
        Err(AppError::new(
            ErrorCode::InvalidInput,
            "El ejecutor no admite argumentos de comandos ni rutas",
        ))
    };
    if let Err(error) = result {
        let _ = helper::write_event(
            &helper::HelperEvent::Error {
                operation_id,
                error,
            },
            &mut output,
        );
        std::process::exit(1);
    }
}
