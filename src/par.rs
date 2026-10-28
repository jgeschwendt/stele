//! Scoped-thread helpers: the per-claim and per-file passes of `build`, freshness, the
//! anchor scan and extraction fan out here, degrading to serial when the OS refuses a thread.

use std::sync::{Arc, Mutex};
use std::thread::{Builder, Scope};

/// `f` over `items` on up to one thread per core, in order — each claim's digest re-reads
/// and re-parses its file, and its churn count spawns git, so the per-claim passes of
/// `build` and freshness are the engine's widest serial loops.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    std::thread::scope(|scope| {
        let chunks: Vec<_> = items
            .chunks(items.len().div_ceil(cores).max(1))
            .map(|chunk| spawn(scope, || chunk.iter().map(&f).collect::<Vec<_>>()))
            .collect();
        chunks.into_iter().flat_map(|join| join()).collect()
    })
}

/// `f` on a thread of `scope`, joined by calling the result — or, when the OS refuses
/// the thread, run here and now: a parallel pass degrades to the serial one, never a panic.
pub fn spawn<'s, R: Send + 's>(
    scope: &'s Scope<'s, '_>,
    f: impl FnOnce() -> R + Send + 's,
) -> Box<dyn FnOnce() -> R + 's> {
    spawn_on(Builder::new(), scope, f)
}

/// [`spawn`] on `builder` (a test seam: an impossible stack makes the OS refuse).
fn spawn_on<'s, R: Send + 's>(
    builder: Builder,
    scope: &'s Scope<'s, '_>,
    f: impl FnOnce() -> R + Send + 's,
) -> Box<dyn FnOnce() -> R + 's> {
    // A refused spawn drops its closure, so the job is shared to survive it.
    let job = Arc::new(Mutex::new(Some(f)));
    let theirs = Arc::clone(&job);
    match builder.spawn_scoped(scope, move || theirs.lock().unwrap().take().unwrap()()) {
        Ok(thread) => Box::new(move || thread.join().unwrap()),
        Err(_) => {
            let done = job.lock().unwrap().take().unwrap()();
            Box::new(move || done)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thread the OS refuses (here an impossible stack) runs its work inline on the
    /// calling thread, never panicking the command.
    #[test]
    fn refused_thread_runs_inline() {
        let refused = std::thread::Builder::new().stack_size(usize::MAX);
        let ran =
            std::thread::scope(|scope| spawn_on(refused, scope, || std::thread::current().id())());
        assert_eq!(ran, std::thread::current().id());
    }
}
