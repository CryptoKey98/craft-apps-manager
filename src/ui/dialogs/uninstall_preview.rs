use std::path::PathBuf;

#[derive(Default)]
pub(in crate::ui) struct ProfilePreview {
    key: Option<String>,
    receiver: Option<std::sync::mpsc::Receiver<Result<Vec<PathBuf>, String>>>,
    pub(super) result: Option<Result<Vec<PathBuf>, String>>,
}
impl ProfilePreview {
    pub(super) fn update(
        &mut self,
        key: String,
        ctx: &eframe::egui::Context,
        resolve: impl FnOnce() -> Result<Vec<PathBuf>, String> + Send + 'static,
    ) -> bool {
        let changed = self.key.as_ref() != Some(&key);
        if changed {
            self.key = Some(key);
            self.result = None;
            let (tx, rx) = std::sync::mpsc::channel();
            self.receiver = Some(rx);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(resolve());
                ctx.request_repaint();
            });
        }
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.result = Some(result);
                    self.receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.result = Some(Err(
                        "Profile folder check stopped. Reopen this dialog to try again.".into(),
                    ));
                    self.receiver = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_lookup_runs_off_ui_thread_and_is_reused_for_the_same_selection() {
        let caller = std::thread::current().id();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = eframe::egui::Context::default();
        let mut preview = ProfilePreview::default();
        preview.update("first-copy".into(), &ctx, move || {
            tx.send(std::thread::current().id()).unwrap();
            Ok(vec![PathBuf::from("profile")])
        });
        assert_ne!(
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
            caller
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while preview.result.is_none() {
            assert!(std::time::Instant::now() < deadline);
            preview.update("first-copy".into(), &ctx, || {
                panic!("Repeated profile scan")
            });
            std::thread::yield_now();
        }
        assert_eq!(
            preview.result.as_ref().unwrap().as_ref().unwrap(),
            &[PathBuf::from("profile")]
        );
    }
    #[test]
    fn switching_installations_discards_late_profile_results() {
        let ctx = eframe::egui::Context::default();
        let mut preview = ProfilePreview::default();
        let (release, wait) = std::sync::mpsc::channel();
        preview.update("old".into(), &ctx, move || {
            wait.recv().unwrap();
            Ok(vec![PathBuf::from("old-profile")])
        });
        preview.update("new".into(), &ctx, || {
            Ok(vec![PathBuf::from("new-profile")])
        });
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while preview.result.is_none() {
            assert!(std::time::Instant::now() < deadline);
            preview.update("new".into(), &ctx, || panic!("Repeated scan"));
            std::thread::yield_now();
        }
        assert_eq!(
            preview.result.as_ref().unwrap().as_ref().unwrap(),
            &[PathBuf::from("new-profile")]
        );
    }
}
