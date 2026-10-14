#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(test)]
mod waiting_lock_tests {
    #[test]
    fn migration_lock_waits_for_active_operation_then_acquires() {
        let name = format!("Local\\CraftMigrationTest{}", uuid::Uuid::new_v4());
        let held = super::Lock::take(&name).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let _lock = super::Lock::wait(&name).unwrap();
            acquired_tx.send(()).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(matches!(
            acquired_rx.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        drop(held);
        acquired_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        worker.join().unwrap();
    }
}
