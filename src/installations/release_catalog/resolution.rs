//! Query intent and observation freshness, independent of transport/cache lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseIntent {
    Browse,
    AutomaticUpdate,
    ManualUpdate,
    InstallValidation,
}
pub const OBSERVATION_SECONDS: i64 = 24 * 3600;
pub fn eligible(checked: i64, now: i64) -> bool {
    checked > 0 && checked <= now + 300 && now.saturating_sub(checked) < OBSERVATION_SECONDS
}
impl ReleaseIntent {
    pub fn uses_catalog(self) -> bool {
        matches!(self, Self::Browse | Self::AutomaticUpdate)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn freshness_boundaries_do_not_extend_when_a_catalog_is_downloaded() {
        let now = 2_000_000;
        assert!(eligible(now - 3600, now));
        assert!(eligible(now - OBSERVATION_SECONDS + 1, now));
        assert!(!eligible(now - OBSERVATION_SECONDS, now));
        assert!(!eligible(now - OBSERVATION_SECONDS - 1, now));
        assert!(!eligible(0, now));
        assert!(!eligible(now + 301, now));
        assert!(!ReleaseIntent::ManualUpdate.uses_catalog());
        assert!(!ReleaseIntent::InstallValidation.uses_catalog());
        for intent in [ReleaseIntent::Browse, ReleaseIntent::AutomaticUpdate] {
            assert!(intent.uses_catalog());
        }
    }
}
