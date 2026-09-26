//! Keep an active Paper Pro request running while its UI waits for the Mac.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct RequestWakeLock {
    name: String,
    unlock: PathBuf,
}

impl RequestWakeLock {
    pub fn acquire(enabled: bool) -> Result<Option<Self>> {
        if !enabled || !matches!(crate::device::DeviceModel::detect(), crate::device::DeviceModel::RemarkablePaperPro) {
            return Ok(None);
        }
        Self::at(Path::new("/sys/power")).map(Some)
    }

    fn at(root: &Path) -> Result<Self> {
        let name = format!("smart_remarkable_request_{}", std::process::id());
        // Kernel timeout is in nanoseconds. A crash must not leave an indefinite
        // wake lock; ordinary completion/error/cancellation releases it sooner.
        std::fs::write(root.join("wake_lock"), format!("{name} 900000000000\n")).context("Could not keep the tablet awake for this request")?;
        log::info!("Request wake lock acquired (15-minute maximum)");
        Ok(Self {
            name,
            unlock: root.join("wake_unlock"),
        })
    }
}

impl Drop for RequestWakeLock {
    fn drop(&mut self) {
        match std::fs::write(&self.unlock, &self.name) {
            Ok(()) => log::info!("Request wake lock released"),
            Err(error) => log::warn!("Could not release request wake lock; kernel timeout remains active: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_wake_lock_releases_when_a_request_returns_an_error() {
        let root = std::env::temp_dir().join(format!("smart-remarkable-awake-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let run = || -> Result<()> {
            let _guard = RequestWakeLock::at(&root)?;
            let value = std::fs::read_to_string(root.join("wake_lock"))?;
            assert!(value.ends_with(" 900000000000\n"));
            anyhow::bail!("simulated request failure")
        };
        assert!(run().is_err());
        let locked = std::fs::read_to_string(root.join("wake_lock")).unwrap();
        let unlocked = std::fs::read_to_string(root.join("wake_unlock")).unwrap();
        assert_eq!(locked.split_whitespace().next().unwrap(), unlocked);
        std::fs::remove_dir_all(root).unwrap();
    }
}
