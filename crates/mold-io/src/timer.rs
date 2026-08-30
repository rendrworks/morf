/// Periodic timer event receiver.
pub struct Timer {
    ticks: mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Timer {
    pub fn every(interval: Duration) -> io::Result<Self> {
        if interval.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "timer interval cannot be zero",
            ));
        }
        let (tx, ticks) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let join = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                thread::park_timeout(interval);
                let _ = tx.try_send(());
            }
        });
        Ok(Self {
            ticks,
            stop,
            join: Some(join),
        })
    }

    pub fn tick(&self, timeout: Duration) -> bool {
        self.ticks.recv_timeout(timeout).is_ok()
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.thread().unpark();
            let _ = join.join();
        }
    }
}

