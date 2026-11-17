use super::parser::parse_deeplink_url;
use super::{PendingDeepLink, PendingDeepLinkEntry};
use crate::error::AppError;

fn parse_err(url: &str) -> AppError {
    parse_deeplink_url(url).expect_err("expected the URL to be rejected")
}

#[test]
fn accepts_bare_bind_link() {
    let request = parse_deeplink_url("aicliapi://v1/bind").expect("bare bind link is valid");
    assert_eq!(request.version, "v1");
    assert_eq!(request.line, None);
}

#[test]
fn accepts_registry_line_ids() {
    for id in ["main", "global", "apac"] {
        let url = format!("aicliapi://v1/bind?line={id}");
        let request = parse_deeplink_url(&url).expect("line id from the registry is valid");
        assert_eq!(request.version, "v1");
        assert_eq!(request.line.as_deref(), Some(id));
    }
}

#[test]
fn rejects_key_like_parameters_without_echoing_values() {
    let secret = "supercalifragilistic-value";
    for key in ["accessKey", "apiKey", "ticket"] {
        let url = format!("aicliapi://v1/bind?{key}={secret}");
        let error = parse_err(&url).to_string();
        assert!(
            !error.contains(secret),
            "error message leaks the parameter value: {error}"
        );
        // 参数名可以出现，参数值不行
        assert!(matches!(
            parse_deeplink_url(&url),
            Err(AppError::InvalidInput(_))
        ));
    }
}

#[test]
fn rejects_any_unknown_parameter() {
    assert!(matches!(
        parse_err("aicliapi://v1/bind?foo=bar"),
        AppError::InvalidInput(_)
    ));
    // 合法参数后再带一个未知参数同样拒绝
    assert!(matches!(
        parse_err("aicliapi://v1/bind?line=main&foo=bar"),
        AppError::InvalidInput(_)
    ));
    assert!(matches!(
        parse_err("aicliapi://v1/bind?foo=bar&line=main"),
        AppError::InvalidInput(_)
    ));
}

#[test]
fn rejects_unknown_line_id_without_echoing_value() {
    let crafted = "not-a-registry-line";
    let error = parse_err(&format!("aicliapi://v1/bind?line={crafted}")).to_string();
    assert!(
        !error.contains(crafted),
        "error message leaks the line value: {error}"
    );
}

#[test]
fn rejects_empty_line_value() {
    assert!(matches!(
        parse_err("aicliapi://v1/bind?line="),
        AppError::InvalidInput(_)
    ));
}

#[test]
fn rejects_duplicate_line_parameter() {
    assert!(matches!(
        parse_err("aicliapi://v1/bind?line=main&line=global"),
        AppError::InvalidInput(_)
    ));
}

#[test]
fn rejects_legacy_import_path() {
    let url = "aicliapi://v1/import?resource=provider&app=claude&name=x&endpoint=https%3A%2F%2Fapi.example.com&apiKey=secret-value";
    let error = parse_err(url).to_string();
    assert!(
        !error.contains("secret-value"),
        "error leaks apiKey value: {error}"
    );
    assert!(
        error.contains("/bind"),
        "error should point at /bind: {error}"
    );
}

#[test]
fn rejects_wrong_scheme_version_or_path() {
    for url in [
        "aicliclient://v1/bind",
        "https://aicliapi.com/v1/bind",
        "aicliapi://v2/bind",
        "aicliapi://v1/takeover",
        "aicliapi://v1/bind/extra",
        "aicliapi://",
    ] {
        assert!(
            matches!(parse_err(url), AppError::InvalidInput(_)),
            "expected rejection: {url}"
        );
    }
}

#[test]
fn pending_payload_carries_no_url_parameter_values() {
    // 事件负载为空（lib.rs 发 `aicli-open-bind` 时附带 ()），这里锁的是待处理条目：
    // 序列化结果只含 kind/version/line，不含 URL 本身或其他参数值
    let request = parse_deeplink_url("aicliapi://v1/bind?line=main").unwrap();
    let entry = PendingDeepLinkEntry {
        id: 7,
        link: PendingDeepLink::Bind {
            request: Box::new(request),
        },
    };
    let json = serde_json::to_string(&entry).expect("serialize pending entry");
    assert!(
        json.contains("\"kind\":\"bind\""),
        "unexpected payload: {json}"
    );
    assert!(
        json.contains("\"version\":\"v1\""),
        "unexpected payload: {json}"
    );
    assert!(
        json.contains("\"line\":\"main\""),
        "unexpected payload: {json}"
    );
    assert!(
        !json.contains("aicliapi://"),
        "payload leaks the URL: {json}"
    );
    assert!(!json.contains('?'), "payload leaks query content: {json}");
}
