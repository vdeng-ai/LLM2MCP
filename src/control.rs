use anyhow::{Result, bail};
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone, Default)]
pub struct Control {
    cancelled: Arc<AtomicBool>,
    job_id: Option<String>,
}
thread_local! { static CURRENT: RefCell<Control> = RefCell::new(Control::default()); }
impl Control {
    pub fn current() -> Self {
        CURRENT.with(|value| value.borrow().clone())
    }
    pub fn set_current(&self) {
        CURRENT.with(|value| *value.borrow_mut() = self.clone());
    }
    pub fn with_job(&self, id: &str) -> Self {
        Self {
            cancelled: self.cancelled.clone(),
            job_id: Some(id.to_owned()),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            bail!("request cancelled");
        }
        if let Some(id) = &self.job_id {
            crate::jobs::Reporter::new(id).check_cancelled()?;
        }
        Ok(())
    }
}
