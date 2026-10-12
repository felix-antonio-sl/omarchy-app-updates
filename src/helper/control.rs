use std::{
    io::{Error, ErrorKind, Read},
    os::fd::AsFd,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct DecisionDeadline {
    duration: Duration,
    until: Arc<Mutex<Instant>>,
}
impl DecisionDeadline {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            until: Arc::new(Mutex::new(Instant::now() + duration)),
        }
    }
    pub fn arm(&self) {
        if let Ok(mut deadline) = self.until.lock() {
            *deadline = Instant::now() + self.duration;
        }
    }
    fn remaining(&self) -> std::io::Result<Duration> {
        self.until
            .lock()
            .map_err(|_| Error::other("Control no disponible"))?
            .checked_duration_since(Instant::now())
            .ok_or_else(|| Error::new(ErrorKind::TimedOut, "Venció la espera de decisión"))
    }
}
pub struct DeadlineRead<R> {
    reader: R,
    deadline: DecisionDeadline,
}
impl<R> DeadlineRead<R> {
    pub fn new(reader: R, deadline: DecisionDeadline) -> Self {
        Self { reader, deadline }
    }
}
impl<R: Read + AsFd> Read for DeadlineRead<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let remaining = self.deadline.remaining()?;
            let timeout = rustix::time::Timespec {
                tv_sec: remaining.as_secs() as i64,
                tv_nsec: remaining.subsec_nanos() as i64,
            };
            let mut fds = [rustix::event::PollFd::new(
                &self.reader,
                rustix::event::PollFlags::IN,
            )];
            match rustix::event::poll(&mut fds, Some(&timeout)) {
                Ok(0) => {
                    return Err(Error::new(
                        ErrorKind::TimedOut,
                        "Venció la espera de decisión",
                    ));
                }
                Ok(_) => return self.reader.read(buffer),
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
}
