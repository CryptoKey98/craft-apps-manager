//! Last API attempt only. This observation never authorizes or blocks a request.
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiStatus {
    Available,
    RateLimited,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiObservation {
    pub status: ApiStatus,
    pub observed_at: i64,
}
impl ApiObservation {
    pub fn status_at(&self, now: i64) -> ApiStatus {
        if now < self.observed_at || now.saturating_sub(self.observed_at) >= 300 {
            ApiStatus::Unknown
        } else {
            self.status
        }
    }
}
fn scope(token: Option<&str>) -> String {
    token
        .map(|value| format!("{:x}", Sha256::digest(value.as_bytes())))
        .unwrap_or_else(|| "public-ip".into())
}
#[derive(Default)]
struct Observations(HashMap<String, (u64, ApiObservation)>);
impl Observations {
    fn observe(&mut self, token: Option<&str>, request: u64, status: ApiStatus, now: i64) {
        let key = scope(token);
        if self
            .0
            .get(&key)
            .is_some_and(|(latest, _)| request < *latest)
        {
            return;
        }
        if self.0.len() >= 8 && !self.0.contains_key(&key) {
            if let Some(oldest) = self
                .0
                .iter()
                .min_by_key(|(_, (_, value))| value.observed_at)
                .map(|(key, _)| key.clone())
            {
                self.0.remove(&oldest);
            }
        }
        self.0.insert(
            key,
            (
                request,
                ApiObservation {
                    status,
                    observed_at: now,
                },
            ),
        );
    }
}
static OBSERVATIONS: OnceLock<Mutex<Observations>> = OnceLock::new();
pub(super) fn request_started() -> u64 {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    SEQUENCE.fetch_add(1, Ordering::Relaxed)
}
pub(super) fn observe(token: Option<&str>, request: u64, status: ApiStatus) {
    if let Ok(mut values) = OBSERVATIONS.get_or_init(Mutex::default).lock() {
        values.observe(token, request, status, chrono::Utc::now().timestamp());
    }
}
pub(super) fn read(token: Option<&str>) -> Option<ApiObservation> {
    OBSERVATIONS
        .get()?
        .try_lock()
        .ok()?
        .0
        .get(&scope(token))
        .map(|(_, value)| value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_response_result_wins_over_quota_counts() {
        use reqwest::{header::HeaderMap, StatusCode};
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
        for code in [StatusCode::OK, StatusCode::NOT_MODIFIED] {
            assert_eq!(
                super::super::response_status(code, &headers, ""),
                ApiStatus::Available
            );
        }
        assert_eq!(
            super::super::response_status(StatusCode::FORBIDDEN, &headers, ""),
            ApiStatus::RateLimited
        );
        headers.insert("x-ratelimit-remaining", "51".parse().unwrap());
        assert_eq!(
            super::super::response_status(
                StatusCode::FORBIDDEN,
                &headers,
                r#"{"message":"You have exceeded a secondary rate limit."}"#
            ),
            ApiStatus::RateLimited
        );
        assert_eq!(
            super::super::response_status(StatusCode::TOO_MANY_REQUESTS, &headers, ""),
            ApiStatus::RateLimited
        );
        for code in [
            StatusCode::FORBIDDEN,
            StatusCode::UNAUTHORIZED,
            StatusCode::NOT_FOUND,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert_eq!(
                super::super::response_status(code, &headers, r#"{"message":"Access denied"}"#),
                ApiStatus::Unknown
            );
        }
    }
    #[test]
    fn latest_attempt_recovers_from_limits_and_late_replies_cannot_replace_it() {
        let mut values = Observations::default();
        values.observe(None, 1, ApiStatus::RateLimited, 1000);
        values.observe(None, 3, ApiStatus::Available, 1001);
        values.observe(None, 2, ApiStatus::RateLimited, 1002);
        assert_eq!(values.0[&scope(None)].1.status, ApiStatus::Available);
        values.observe(None, 4, ApiStatus::Unknown, 1003);
        assert_eq!(values.0[&scope(None)].1.status, ApiStatus::Unknown);
    }
    #[test]
    fn expiry_does_not_invent_availability() {
        for status in [
            ApiStatus::Available,
            ApiStatus::RateLimited,
            ApiStatus::Unknown,
        ] {
            let value = ApiObservation {
                status,
                observed_at: 1000,
            };
            assert_eq!(value.status_at(1299), status);
            assert_eq!(value.status_at(1300), ApiStatus::Unknown);
            assert_eq!(value.status_at(999), ApiStatus::Unknown);
        }
    }
    #[test]
    fn observations_are_credential_scoped_and_bounded() {
        let mut values = Observations::default();
        values.observe(None, 1, ApiStatus::Available, 1000);
        values.observe(Some("fixture"), 2, ApiStatus::RateLimited, 1001);
        assert_eq!(values.0[&scope(None)].1.status, ApiStatus::Available);
        assert_eq!(
            values.0[&scope(Some("fixture"))].1.status,
            ApiStatus::RateLimited
        );
        assert!(!values.0.contains_key(&scope(Some("other"))));
        for index in 0..20 {
            values.observe(
                Some(&format!("fixture-{index}")),
                index as u64 + 3,
                ApiStatus::Unknown,
                1002 + index,
            );
        }
        assert_eq!(values.0.len(), 8);
    }
    #[test]
    fn clearing_cache_does_not_clear_or_refresh_status() {
        let root = crate::network::cache_tests::root();
        let credential = format!("fixture-{}", uuid::Uuid::new_v4());
        observe(Some(&credential), request_started(), ApiStatus::RateLimited);
        let before = read(Some(&credential));
        crate::network::clear_api_cache(&root).unwrap();
        assert_eq!(read(Some(&credential)), before);
        std::fs::remove_dir_all(root).unwrap();
    }
}
