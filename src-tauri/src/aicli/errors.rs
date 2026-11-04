//! AICLI lines share one backend: only transport/CDN failures may change lines.
use crate::proxy::error::{ErrorCategory, ProxyError};
use serde_json::Value;
use std::time::{Duration, SystemTime};

pub fn categorize(error: &ProxyError) -> ErrorCategory {
    match error {
        ProxyError::ClientAbort => ErrorCategory::ClientAbort,
        ProxyError::ForwardFailed(_)
        | ProxyError::Timeout(_)
        | ProxyError::StreamIdleTimeout(_) => ErrorCategory::Retryable,
        ProxyError::UpstreamError { status, body }
            if matches!(status, 502..=504 | 520..=527 | 530)
                && !body
                    .as_deref()
                    .is_some_and(|s| serde_json::from_str::<Value>(s).is_ok()) =>
        {
            ErrorCategory::Retryable
        }
        _ => ErrorCategory::NonRetryable,
    }
}

pub fn code(error: &ProxyError) -> Option<&'static str> {
    let ProxyError::UpstreamError { status, body } = error else {
        return None;
    };
    let parsed = body
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let code = parsed
        .as_ref()
        .and_then(|v| {
            v.pointer("/error/code")
                .or_else(|| v.pointer("/error/type"))
                .or_else(|| v.get("code"))
        })
        .and_then(Value::as_str);
    match code {
        Some("insufficient_quota") => Some("insufficient_quota"),
        Some("model_not_found") => Some("model_not_found"),
        Some("invalid_token") => Some("invalid_token"),
        _ if *status == 401 => Some("invalid_token"),
        _ => None,
    }
}

/// Fixed messages do not copy upstream text (which can contain credentials).
pub fn mapped(error: ProxyError) -> ProxyError {
    let Some(code) = code(&error) else {
        return error;
    };
    let ProxyError::UpstreamError { status, .. } = error else {
        return error;
    };
    let message = match code {
        "insufficient_quota" => "账户额度不足",
        "model_not_found" => "模型不存在",
        _ => "API 密钥已失效，请重试以使用恢复后的密钥",
    };
    ProxyError::UpstreamError {
        status,
        body: Some(
            serde_json::json!({"error":{"code":code,"type":code,"message":message}}).to_string(),
        ),
    }
}

pub fn retry_after(value: Option<&str>, now: SystemTime) -> Option<Duration> {
    let value = value?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds.min(3600)));
    }
    chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|date| SystemTime::from(date.with_timezone(&chrono::Utc)))?
        .duration_since(now)
        .ok()
        .map(|d| d.min(Duration::from_secs(3600)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classification_table() {
        for status in [
            401, 403, 404, 408, 409, 429, 451, 500, 501, 502, 503, 504, 520, 521, 522, 523, 524,
            525, 526, 527, 528, 529, 530,
        ] {
            for body in [
                None,
                Some("<html>cdn unavailable</html>"),
                Some("{\"error\":\"backend failure\"}"),
                Some("null"),
                Some("42"),
            ] {
                let expected = if matches!(status,502..=504|520..=527|530)
                    && matches!(body, None | Some("<html>cdn unavailable</html>"))
                {
                    ErrorCategory::Retryable
                } else {
                    ErrorCategory::NonRetryable
                };
                assert_eq!(
                    categorize(&ProxyError::UpstreamError {
                        status,
                        body: body.map(str::to_owned)
                    }),
                    expected,
                    "status={status}, body={body:?}"
                );
            }
        }
        for error in [
            ProxyError::ForwardFailed("TLS".into()),
            ProxyError::Timeout("first byte".into()),
            ProxyError::StreamIdleTimeout(1),
        ] {
            assert_eq!(categorize(&error), ErrorCategory::Retryable);
        }
        for error in [
            ProxyError::ConfigError("fixture".into()),
            ProxyError::TransformError("fixture".into()),
            ProxyError::AuthError("fixture".into()),
        ] {
            assert_eq!(categorize(&error), ErrorCategory::NonRetryable);
        }
        assert_eq!(
            categorize(&ProxyError::ClientAbort),
            ErrorCategory::ClientAbort
        );
    }
    #[test]
    fn mapped_errors_are_distinct_and_do_not_echo_upstream_text() {
        for (status, code, message) in [
            (403, "insufficient_quota", "账户额度不足"),
            (404, "model_not_found", "模型不存在"),
            (
                401,
                "invalid_token",
                "API 密钥已失效，请重试以使用恢复后的密钥",
            ),
        ] {
            let error = mapped(ProxyError::UpstreamError {
                status,
                body: Some(
                    serde_json::json!({"error":{"code":code,"message":"private-upstream-text"}})
                        .to_string(),
                ),
            });
            let ProxyError::UpstreamError {
                status: actual,
                body: Some(body),
            } = error
            else {
                panic!("upstream error expected")
            };
            let body: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(actual, status);
            assert_eq!(body["error"]["code"], code);
            assert_eq!(body["error"]["message"], message);
            assert!(!body.to_string().contains("private-upstream-text"));
        }
        assert_eq!(
            code(&ProxyError::UpstreamError {
                status: 401,
                body: None
            }),
            Some("invalid_token")
        );
    }
    #[test]
    fn retry_after_seconds_and_dates() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000);
        assert_eq!(
            retry_after(Some("120"), now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            retry_after(Some("99999"), now),
            Some(Duration::from_secs(3600))
        );
        let date = chrono::DateTime::<chrono::Utc>::from(now + Duration::from_secs(180))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        assert_eq!(
            retry_after(Some(&date), now),
            Some(Duration::from_secs(180))
        );
        assert_eq!(retry_after(Some("bad"), now), None);
        assert_eq!(retry_after(None, now), None);
    }
}
