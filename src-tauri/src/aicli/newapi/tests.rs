use super::*;
use axum::body::Body;
use axum::http::{Request, Response};
use axum::routing::any;
use axum::Router;
use std::sync::{Arc, Mutex};

type Recorded = Arc<Mutex<Vec<(Method, Url, reqwest::header::HeaderMap, Value)>>>;

struct Mock {
    origin: Url,
    requests: Recorded,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn mock(replies: Vec<(u16, &'static str, String)>) -> Mock {
    let requests: Recorded = Arc::default();
    let recorded = requests.clone();
    let replies = Arc::new(Mutex::new(std::collections::VecDeque::from(replies)));
    let app = Router::new().fallback(any(move |request: Request<Body>| {
        let requests = recorded.clone();
        let replies = replies.clone();
        async move {
            let (parts, body) = request.into_parts();
            let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
            let url = Url::parse(&format!("http://localhost{}", parts.uri)).unwrap();
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            requests
                .lock()
                .unwrap()
                .push((parts.method, url, parts.headers, body));
            let (status, content_type, body) = replies.lock().unwrap().pop_front().unwrap_or((
                500,
                "text/plain",
                "unexpected request".into(),
            ));
            Response::builder()
                .status(status)
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Mock {
        origin,
        requests,
        task,
    }
}

fn ok(data: Value) -> (u16, &'static str, String) {
    (
        200,
        "application/json",
        json!({"success":true,"data":data}).to_string(),
    )
}

fn client(origins: &[Url]) -> NewApiClient {
    NewApiClient::build("synthetic-access-value", origins.to_vec(), true).unwrap()
}

#[test]
fn fixed_line_order_and_timeout_constants() {
    let api = NewApiClient::new("synthetic-access-value", "global").unwrap();
    assert_eq!(
        api.origins.iter().map(Url::as_str).collect::<Vec<_>>(),
        vec![
            "https://aicli.qzz.io/",
            "https://aicliapi.com/",
            "https://api.aicli.qzz.io/"
        ]
    );
    assert_eq!(CONNECT_TIMEOUT.as_secs(), 10);
    assert_eq!(REQUEST_TIMEOUT.as_secs(), 20);
    assert!(api.authorization.is_sensitive());
    assert_eq!(
        NewApiClient::new("x", "unknown").unwrap_err(),
        NewApiError::UnknownLine
    );
    assert_eq!(
        NewApiClient::new("", "main").unwrap_err(),
        NewApiError::InvalidInput
    );
    assert_eq!(
        NewApiClient::new("\r\nInjected: x", "main").unwrap_err(),
        NewApiError::InvalidInput
    );
}

#[tokio::test]
async fn falls_back_in_order_on_network_and_html_5xx() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let unreachable = Url::parse(&format!("http://{}/", closed.local_addr().unwrap())).unwrap();
    drop(closed);
    let edge = mock(vec![(502, "text/html", "<!doctype html>edge error".into())]).await;
    let healthy = mock(vec![ok(json!({"id":7}))]).await;
    let result = client(&[unreachable, edge.origin.clone(), healthy.origin.clone()])
        .user_self()
        .await
        .unwrap();
    assert_eq!(result["data"]["id"], 7);
    assert_eq!(edge.requests.lock().unwrap().len(), 1);
    assert_eq!(healthy.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn html_body_without_content_type_can_fall_back() {
    let edge = mock(vec![(503, "text/plain", " \n<HTML>edge error".into())]).await;
    let healthy = mock(vec![ok(json!({"id":7}))]).await;
    assert!(client(&[edge.origin.clone(), healthy.origin.clone()])
        .user_self()
        .await
        .is_ok());
}

#[tokio::test]
async fn status_and_api_errors_do_not_change_lines_or_expose_bodies() {
    for (status, body, expected) in [
        (401, "key=synthetic-response-key Authorization synthetic-access-value", NewApiError::TokenInvalid),
        (403, "<html>Authorization synthetic-response-key", NewApiError::Http(403)),
        (429, "<html>Authorization synthetic-response-key", NewApiError::Http(429)),
        (500, "{\"success\":false,\"key\":\"synthetic-response-key\"}", NewApiError::Http(500)),
        (200, "{\"success\":false,\"message\":\"Authorization synthetic-access-value synthetic-response-key\"}", NewApiError::Rejected),
        (200, "{\"key\":\"synthetic-response-key\"}", NewApiError::InvalidResponse),
        (200, "Authorization synthetic-response-key", NewApiError::InvalidResponse),
        (302, "Authorization synthetic-response-key", NewApiError::Http(302)),
    ] {
        let first = mock(vec![(status, "application/json", body.into())]).await;
        let second = mock(vec![ok(json!({}))]).await;
        let api = client(&[first.origin.clone(), second.origin.clone()]);
        let error = api.user_self().await.unwrap_err();
        assert_eq!(error, expected);
        let diagnostic = format!("{api:?} {error:?} {error}");
        for secret in ["Authorization", "synthetic-response-key", "synthetic-access-value"] {
            assert!(!diagnostic.contains(secret));
        }
        assert_eq!(first.requests.lock().unwrap().len(), 1);
        assert!(second.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn lists_all_pages_even_when_page_is_short_and_rejects_stalled_pages() {
    let server = mock(vec![
        ok(json!({"total":2,"items":[{"id":1}]})),
        ok(json!({"total":2,"items":[{"id":2}]})),
    ])
    .await;
    let items = client(&[server.origin.clone()])
        .list_tokens()
        .await
        .unwrap();
    assert_eq!(items, vec![json!({"id":1}), json!({"id":2})]);
    let requests = server.requests.lock().unwrap();
    for (index, (_, url, _, _)) in requests.iter().enumerate() {
        let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["p"], (index + 1).to_string());
        assert_eq!(query["page_size"], "100");
    }
    drop(requests);
    for second in [
        json!({"total":2,"items":[]}),
        json!({"total":2,"items":[{"id":1}]}),
    ] {
        let server = mock(vec![ok(json!({"total":2,"items":[{"id":1}]})), ok(second)]).await;
        assert_eq!(
            client(&[server.origin.clone()]).list_tokens().await,
            Err(NewApiError::InvalidResponse)
        );
    }
}

#[tokio::test]
async fn wrappers_use_exact_paths_queries_payloads_and_only_bearer_identity() {
    let request_id = "rid&unexpected=1/中文";
    let server = mock(vec![
        ok(json!({})),
        ok(json!({})),
        (
            200,
            "application/json",
            json!({"success":true,"data":[],"group_ratio":{"g-mid":1}}).to_string(),
        ),
        ok(json!({"max_count":10})),
        ok(Value::Null),
        ok(Value::Null),
        ok(json!({"keys":{"7":"synthetic-returned-model-key"}})),
        ok(json!({"items":[{"request_id":request_id}]})),
        ok(json!({"quota":12})),
        ok(json!({"total":3})),
    ])
    .await;
    let api = client(&[server.origin.clone()]);
    api.user_self().await.unwrap();
    api.user_groups().await.unwrap();
    assert_eq!(api.pricing().await.unwrap()["group_ratio"]["g-mid"], 1);
    api.auto_groups_setting().await.unwrap();
    let specification = json!({"name":"aicli:test","auto_groups":["g-mid","g-cheap"],"unlimited_quota":true,"expired_time":-1});
    api.create_token(&specification).await.unwrap();
    api.delete_token(7).await.unwrap();
    assert_eq!(
        api.batch_token_keys(&[7]).await.unwrap()[&7],
        "synthetic-returned-model-key"
    );
    assert_eq!(
        api.log_by_request_id(request_id).await.unwrap().unwrap()["request_id"],
        request_id
    );
    api.log_stat(100, 200).await.unwrap();
    assert_eq!(api.log_count(100, 200).await.unwrap(), 3);
    let requests = server.requests.lock().unwrap();
    let expected = [
        (Method::GET, "/api/user/self"),
        (Method::GET, "/api/user/self/groups"),
        (Method::GET, "/api/pricing"),
        (Method::GET, "/api/token/auto-groups"),
        (Method::POST, "/api/token/"),
        (Method::DELETE, "/api/token/7"),
        (Method::POST, "/api/token/batch/keys"),
        (Method::GET, "/api/log/self"),
        (Method::GET, "/api/log/self/stat"),
        (Method::GET, "/api/log/self"),
    ];
    assert_eq!(requests.len(), expected.len());
    for ((method, url, headers, _), (expected_method, path)) in requests.iter().zip(expected) {
        assert_eq!(method, &expected_method);
        assert_eq!(url.path(), path);
        assert_eq!(
            headers[AUTHORIZATION],
            format!("Bearer {}", "synthetic-access-value")
        );
        for absent in ["cookie", "new-api-user", "x-security-proof"] {
            assert!(!headers.contains_key(absent));
        }
    }
    assert_eq!(requests[4].3, specification);
    assert_eq!(requests[6].3, json!({"ids":[7]}));
    let bill_query: BTreeMap<_, _> = requests[7].1.query_pairs().into_owned().collect();
    assert_eq!(bill_query["request_id"], request_id);
    assert_eq!(bill_query["p"], "1");
    assert_eq!(bill_query["page_size"], "1");
    for i in [8, 9] {
        let query: BTreeMap<_, _> = requests[i].1.query_pairs().into_owned().collect();
        assert_eq!(query["type"], "2");
        assert_eq!(query["start_timestamp"], "100");
        assert_eq!(query["end_timestamp"], "200");
    }
}

#[tokio::test]
async fn rejects_missing_batch_keys_mismatched_logs_and_invalid_time_ranges() {
    let server = mock(vec![
        ok(json!({"keys":{}})),
        ok(json!({"items":[{"request_id":"different"}]})),
        ok(json!({"items":[]})),
    ])
    .await;
    let api = client(&[server.origin.clone()]);
    assert_eq!(
        api.batch_token_keys(&[7]).await,
        Err(NewApiError::InvalidResponse)
    );
    assert_eq!(
        api.log_by_request_id("wanted").await,
        Err(NewApiError::InvalidResponse)
    );
    assert_eq!(api.log_by_request_id("absent").await.unwrap(), None);
    assert_eq!(api.log_stat(200, 100).await, Err(NewApiError::InvalidInput));
    assert_eq!(api.log_count(-1, 100).await, Err(NewApiError::InvalidInput));
    assert_eq!(api.delete_token(0).await, Err(NewApiError::InvalidInput));
    assert_eq!(
        api.batch_token_keys(&[-1]).await,
        Err(NewApiError::InvalidInput)
    );
    assert_eq!(
        api.log_by_request_id("").await,
        Err(NewApiError::InvalidInput)
    );
    assert_eq!(server.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn exhausted_network_error_and_client_debug_are_redacted_in_logs() {
    struct Capture(Mutex<Vec<String>>);
    impl log::Log for Capture {
        fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                self.0.lock().unwrap().push(record.args().to_string());
            }
        }
        fn flush(&self) {}
    }
    static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        log::set_logger(&CAPTURE).unwrap();
        log::set_max_level(log::LevelFilter::Debug);
    });
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    drop(listener);
    let api = client(&[url]);
    let error = api.user_self().await.unwrap_err();
    assert_eq!(error, NewApiError::Network);
    log::error!(target:"aicli::newapi::redaction_test","{api:?} {error:?}: {error}");
    let server=mock(vec![(200,"application/json",json!({"success":false,"message":"Authorization synthetic-response-key synthetic-access-value"}).to_string())]).await;
    let error = client(&[server.origin.clone()])
        .user_self()
        .await
        .unwrap_err();
    log::error!(target:"aicli::newapi::redaction_test","{error:?}: {error}");
    let logs = CAPTURE.0.lock().unwrap().join("\n");
    assert!(!logs.is_empty());
    for forbidden in [
        "Authorization",
        "synthetic-access-value",
        "synthetic-response-key",
    ] {
        assert!(!logs.contains(forbidden));
    }
}

#[cfg(feature = "test-hooks")]
#[test]
fn local_test_constructor_rejects_remote_or_credential_bearing_origins() {
    for url in [
        "https://127.0.0.1",
        "http://example.com",
        "http://127.0.0.1.example.com",
        "http://user@localhost",
        "http://localhost/path",
        "http://localhost/?q=x",
        "http://localhost/#x",
    ] {
        assert_eq!(
            NewApiClient::for_local_test("synthetic-access-value", url).unwrap_err(),
            NewApiError::InvalidInput
        );
    }
}
