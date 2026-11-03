use reqwest::{header::HeaderMap, StatusCode};

pub(super) fn is_limited(status: StatusCode, headers: &HeaderMap, body: &str) -> bool {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return true;
    }
    if status != StatusCode::FORBIDDEN {
        return false;
    }
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    headers
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        == Some(0)
        || headers.contains_key("retry-after")
        || message.contains("rate limit")
        || message.contains("abuse detection")
}

pub(super) fn error(status: StatusCode, headers: &HeaderMap, body: &str) -> String {
    let remaining = headers
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let lower = message.to_ascii_lowercase();
    if remaining == Some(0) || (remaining.is_none() && lower.contains("api rate limit exceeded")) {
        let reset = headers
            .get("x-ratelimit-reset")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .and_then(|value| chrono::DateTime::from_timestamp(value, 0))
            .map(|value| {
                format!(
                    "; resets at {}",
                    value.with_timezone(&chrono::Local).format("%I:%M %p")
                )
            })
            .unwrap_or_default();
        return format!("GitHub API limit reached{reset}. Wait before retrying.");
    }
    if status == StatusCode::TOO_MANY_REQUESTS
        || lower.contains("rate limit")
        || lower.contains("abuse detection")
        || headers.contains_key("retry-after")
    {
        let retry = headers
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|seconds| format!(" Retry after {seconds} seconds."))
            .unwrap_or_else(|| " Wait before retrying.".into());
        return format!(
            "GitHub API limit temporarily applied; hourly allowance may still remain.{retry}"
        );
    }
    let detail: String = message
        .chars()
        .filter(|c| !c.is_control())
        .take(256)
        .collect();
    format!(
        "GitHub denied the request ({}). {}",
        status.as_u16(),
        detail
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headers(remaining: u64) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ratelimit-remaining",
            remaining.to_string().parse().unwrap(),
        );
        headers.insert("x-ratelimit-reset", "2000000000".parse().unwrap());
        headers
    }
    #[test]
    fn ordinary_forbidden_with_remaining_budget_is_not_a_quota_error() {
        let message = error(
            StatusCode::FORBIDDEN,
            &headers(51),
            r#"{"message":"Resource not accessible by personal access token"}"#,
        );
        assert!(message.contains("denied"));
        assert!(!message.contains("limit"));
    }
    #[test]
    fn temporary_throttle_does_not_claim_the_hourly_allowance_is_exhausted() {
        let message = error(
            StatusCode::FORBIDDEN,
            &headers(51),
            r#"{"message":"You have exceeded a secondary rate limit."}"#,
        );
        assert!(message.contains("temporarily"));
        assert!(!message.contains("resets at"));
        assert!(error(StatusCode::TOO_MANY_REQUESTS, &headers(51), "").contains("temporarily"));
    }
    #[test]
    fn zero_remaining_is_an_hourly_limit_and_uses_reported_reset() {
        let message = error(
            StatusCode::FORBIDDEN,
            &headers(0),
            r#"{"message":"API rate limit exceeded"}"#,
        );
        assert!(message.contains("API limit reached"));
        assert!(message.contains("resets at"));
    }
}
