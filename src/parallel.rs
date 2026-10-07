//! Bounded, work-conserving workers; outputs retain their input order.
use crate::control::Control;
use anyhow::{Result, anyhow};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

pub fn ordered<T, F, P>(count: usize, limit: usize, work: F, mut progress: P) -> Result<Vec<T>>
where
    T: Send,
    F: Fn(usize) -> Result<T> + Sync,
    P: FnMut(usize, &T) -> Result<()>,
{
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let control = Control::current();
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let mut handles = Vec::new();
        for _ in 0..limit.max(1).min(count) {
            let sender = sender.clone();
            let work = &work;
            let next = &next;
            let stopped = &stopped;
            let control = control.clone();
            handles.push(scope.spawn(move || {
                control.set_current();
                while !stopped.load(Ordering::Acquire) {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= count {
                        break;
                    }
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        control.check().and_then(|()| work(index))
                    }))
                    .unwrap_or_else(|_| Err(anyhow!("repository map worker thread panicked")));
                    if result.is_err() {
                        stopped.store(true, Ordering::Release);
                    }
                    if sender.send((index, result)).is_err() {
                        break;
                    }
                }
            }));
        }
        drop(sender);
        let mut outputs = (0..count).map(|_| None).collect::<Vec<_>>();
        let mut failure = None;
        for (index, result) in receiver {
            match result {
                Ok(output) => {
                    if failure.is_none()
                        && let Err(error) = progress(index, &output)
                    {
                        failure = Some(error);
                        stopped.store(true, Ordering::Release);
                    }
                    outputs[index] = Some(output);
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        for handle in handles {
            if handle.join().is_err() {
                failure.get_or_insert_with(|| anyhow!("repository map worker thread panicked"));
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        outputs
            .into_iter()
            .map(|output| output.ok_or_else(|| anyhow!("missing repository map output")))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Mutex, time::Duration};

    #[test]
    fn refills_a_free_worker_while_an_earlier_item_waits_and_preserves_order() {
        let (sender, receiver) = mpsc::channel();
        let receiver = Mutex::new(receiver);
        let mut completed = Vec::new();
        let outputs = ordered(
            3,
            2,
            |index| {
                if index == 0 {
                    receiver
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(3))?;
                } else if index == 2 {
                    sender.send(())?;
                }
                Ok(index)
            },
            |index, _| {
                completed.push(index);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(outputs, [0, 1, 2]);
        assert_eq!(completed.len(), 3);
        assert!(completed.iter().position(|&i| i == 1) < completed.iter().position(|&i| i == 0));
    }

    #[test]
    fn failure_stops_new_work_and_propagates_the_error() {
        let called = AtomicUsize::new(0);
        let result = ordered(
            20,
            1,
            |_| {
                called.fetch_add(1, Ordering::Relaxed);
                Err::<(), _>(anyhow!("map failed"))
            },
            |_, _| Ok(()),
        );
        assert!(result.unwrap_err().to_string().contains("map failed"));
        assert_eq!(called.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn panics_stop_new_work_and_are_reported_as_errors() {
        let called = AtomicUsize::new(0);
        let result = ordered(
            20,
            1,
            |_| -> Result<()> {
                called.fetch_add(1, Ordering::Relaxed);
                panic!("broken worker");
            },
            |_, _| Ok(()),
        );
        assert!(result.unwrap_err().to_string().contains("panicked"));
        assert_eq!(called.load(Ordering::Relaxed), 1);
    }
}
