//! The one export or save of a device log that may run at a time.
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

/// How long the app's end waits in all for a write to stop and delete its
/// temporary file. A disk that takes longer keeps the file.
const END: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct Exports {
    running: Mutex<Option<Arc<Job>>>,
    /// When the app's end stops waiting.
    deadline: OnceLock<Instant>,
}
#[derive(Default)]
pub struct Job {
    cancel: AtomicBool,
    /// Whether it writes: not while the save panel is open.
    writing: Mutex<bool>,
    ended: Condvar,
}
impl Job {
    /// The flag that stops it.
    pub fn cancelled(&self) -> &AtomicBool {
        &self.cancel
    }
    /// Says that it writes until the returned value is dropped; None once it
    /// was cancelled.
    pub fn write(&self) -> Option<Writing<'_>> {
        let mut writing = self.writing.lock().unwrap();
        if self.cancel.load(Ordering::Acquire) {
            return None;
        }
        *writing = true;
        Some(Writing(self))
    }
}
pub struct Writing<'a>(&'a Job);
impl Drop for Writing<'_> {
    fn drop(&mut self) {
        *self.0.writing.lock().unwrap() = false;
        self.0.ended.notify_all();
    }
}
/// A running export or save; dropping it frees its place, even when the
/// request itself is dropped.
pub struct Running<'a> {
    exports: &'a Exports,
    pub job: Arc<Job>,
}
impl Drop for Running<'_> {
    fn drop(&mut self) {
        *self.exports.running.lock().unwrap() = None;
    }
}
impl Exports {
    /// Takes the one place until the returned export ends.
    pub fn start(&self) -> Result<Running<'_>, String> {
        let mut slot = self.running.lock().unwrap();
        if slot.is_some() {
            return Err("An export is already running".into());
        }
        let job = Arc::new(Job::default());
        *slot = Some(job.clone());
        Ok(Running { exports: self, job })
    }
    /// Stops the one that runs, if any; it deletes its temporary file.
    pub fn cancel(&self) -> Option<Arc<Job>> {
        let job = self.running.lock().unwrap().clone()?;
        {
            // Not between a write's check and its start.
            let _writing = job.writing.lock().unwrap();
            job.cancel.store(true, Ordering::Release);
        }
        Some(job)
    }
    /// Stops the one that runs as the app ends, and waits a little for it to
    /// delete its temporary file: the process may end before it would.
    pub fn end(&self) {
        let deadline = *self.deadline.get_or_init(|| Instant::now() + END);
        let Some(job) = self.cancel() else { return };
        let writing = job.writing.lock().unwrap();
        let left = deadline.saturating_duration_since(Instant::now());
        let _ = job.ended.wait_timeout_while(writing, left, |w| *w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn one_runs_at_a_time_and_can_be_cancelled() {
        let exports = Exports::default();
        assert!(exports.cancel().is_none());
        let first = exports.start().unwrap();
        assert!(exports.start().is_err());
        assert!(!first.job.cancelled().load(Ordering::Acquire));
        exports.cancel();
        assert!(first.job.cancelled().load(Ordering::Acquire));
        // Cancelled while its save panel was open, it writes nothing.
        assert!(first.job.write().is_none());
        drop(first);
        assert!(exports.start().unwrap().job.write().is_some());
    }

    #[test]
    fn the_end_waits_for_a_write_to_stop() {
        let exports = Exports::default();
        let running = exports.start().unwrap();
        let cleaned = AtomicBool::new(false);
        let (started, writes) = std::sync::mpsc::channel();
        thread::scope(|s| {
            let (job, cleaned) = (running.job.clone(), &cleaned);
            s.spawn(move || {
                let _writing = job.write().unwrap();
                started.send(()).unwrap();
                while !job.cancelled().load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                thread::sleep(Duration::from_millis(50));
                cleaned.store(true, Ordering::Release);
            });
            writes.recv().unwrap();
            exports.end();
            assert!(cleaned.load(Ordering::Acquire));
        });
    }

    #[test]
    fn the_end_waits_for_no_panel_and_not_for_ever() {
        let exports = Exports::default();
        let at = Instant::now();
        exports.end();
        let running = exports.start().unwrap();
        exports.end();
        assert!(at.elapsed() < END);
        drop(running);
        // Closing the window and then the app's end wait once, not twice.
        let exports = Exports::default();
        let running = exports.start().unwrap();
        let _writing = running.job.write().unwrap();
        let at = Instant::now();
        exports.end();
        exports.end();
        assert!(at.elapsed() >= END && at.elapsed() < END * 2);
    }
}
