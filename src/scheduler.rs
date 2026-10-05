use crate::control::Control;
use anyhow::Result;
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    thread,
    time::Duration,
};
pub struct Slot {
    _file: File,
}
impl Slot {
    pub fn acquire(pool: &str, limit: usize, control: &Control) -> Result<Self> {
        let dir = crate::config::data_dir()?.join("scheduler");
        std::fs::create_dir_all(&dir)?;
        loop {
            control.check()?;
            for index in 0..limit.clamp(1, 16) {
                let file = OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(dir.join(format!("{pool}-{index}.lock")))?;
                match file.try_lock_exclusive() {
                    Ok(()) => return Ok(Self { _file: file }),
                    Err(error)
                        if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
                    Err(error) => return Err(error.into()),
                }
            }
            thread::sleep(Duration::from_millis(150));
        }
    }
}
