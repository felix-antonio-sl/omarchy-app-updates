pub mod aur;
pub mod flatpak;
use crate::{domain::*, paths::Paths, providers::ProviderContext};
use std::sync::Arc;
pub trait EventSink: Send + Sync {
    fn send(&self, event: OperationEvent) -> AppResult<()>;
}
pub struct ExecutionContext<'a> {
    pub provider: ProviderContext,
    pub paths: Paths,
    pub operation_id: OperationId,
    pub emit: &'a dyn EventSink,
    pub flatpak: Arc<dyn flatpak::FlatpakBackend>,
}
impl<'a> ExecutionContext<'a> {
    pub fn new(
        provider: ProviderContext,
        operation_id: OperationId,
        emit: &'a dyn EventSink,
    ) -> Self {
        Self {
            paths: provider.paths.clone(),
            provider,
            operation_id,
            emit,
            flatpak: Arc::new(crate::flatpak_native::NativeFlatpak),
        }
    }
    pub fn with_flatpak(mut self, backend: Arc<dyn flatpak::FlatpakBackend>) -> Self {
        self.flatpak = backend;
        self
    }
}
