//! Exercise the real client against the isolated rc.34 fixture, never a public origin.
use cc_switch_lib::aicli::newapi::{NewApiClient, NewApiError};
use serde_json::json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn require(condition: bool, message: &'static str) -> Result<(), &'static str> {
    if condition {
        Ok(())
    } else {
        Err(message)
    }
}

async fn exercise_token(api: &NewApiClient, origin: &str, id: i64) -> Result<(), &'static str> {
    let keys = api
        .batch_token_keys(&[id])
        .await
        .map_err(|_| "batch_token_keys failed")?;
    let raw = keys.get(&id).ok_or("batch_token_keys missing key")?;
    let key = if raw.starts_with("sk-") {
        raw.clone()
    } else {
        format!("sk-{raw}")
    };
    let response = reqwest::Client::builder().no_proxy()
        .redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(20))
        .build().map_err(|_| "local model client failed")?
        .post(format!("{}/v1/chat/completions", origin.trim_end_matches('/')))
        .bearer_auth(key)
        .json(&json!({"model":"mock-model-a","messages":[{"role":"user","content":"C1-04 fixture"}],"max_tokens":8}))
        .send().await.map_err(|_| "local mock model request failed")?;
    require(
        response.status().is_success(),
        "local mock model status failed",
    )?;
    let request_id = response
        .headers()
        .get("X-Oneapi-Request-Id")
        .and_then(|value| value.to_str().ok())
        .ok_or("missing local request id")?
        .to_owned();
    response
        .bytes()
        .await
        .map_err(|_| "local model body failed")?;
    let mut bill = None;
    for _ in 0..10 {
        bill = api
            .log_by_request_id(&request_id)
            .await
            .map_err(|_| "log_by_request_id failed")?;
        if bill.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let bill = bill.ok_or("local bill missing")?;
    require(
        bill["type"] == 2 && bill["token_id"] == id,
        "local bill identity failed",
    )?;
    require(
        bill["quota"].as_i64().is_some_and(|quota| quota > 0),
        "local bill quota failed",
    )?;
    let end = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "local clock failed")?
        .as_secs() as i64
        + 1;
    require(
        api.log_count(0, end)
            .await
            .map_err(|_| "log_count failed")?
            == 1,
        "local log count failed",
    )?;
    let stat = api.log_stat(0, end).await.map_err(|_| "log_stat failed")?;
    require(
        stat["data"]["quota"] == bill["quota"],
        "local stat quota differs from bill",
    )?;
    require(
        api.log_by_request_id("c104-missing-request")
            .await
            .map_err(|_| "missing log lookup failed")?
            .is_none(),
        "missing log matched",
    )
}

async fn run() -> Result<(), &'static str> {
    require(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
        "CI fixture only",
    )?;
    let home = std::env::var("HOME").map_err(|_| "missing isolated home")?;
    require(
        std::env::var("CC_SWITCH_TEST_HOME").as_deref() == Ok(home.as_str()),
        "home is not isolated",
    )?;
    let origin = std::env::var("AICLI_TESTENV_BASE_URL").map_err(|_| "missing local origin")?;
    let access =
        std::env::var("AICLI_LOCAL_TEST_ACCESS_TOKEN").map_err(|_| "missing local credential")?;
    let username =
        std::env::var("AICLI_LOCAL_TEST_USERNAME").map_err(|_| "missing fixture username")?;
    let api = NewApiClient::for_local_test(&access, &origin).map_err(|_| "invalid local client")?;
    let account = api.user_self().await.map_err(|_| "user_self failed")?;
    require(
        account["data"]["username"] == username,
        "wrong disposable account",
    )?;
    let groups = api.user_groups().await.map_err(|_| "user_groups failed")?;
    require(
        groups["data"].get("g-cheap").is_some() && groups["data"].get("g-mid").is_some(),
        "fixture groups missing",
    )?;
    let pricing = api.pricing().await.map_err(|_| "pricing failed")?;
    require(
        pricing["data"].as_array().is_some_and(|models| {
            models
                .iter()
                .any(|model| model["model_name"] == "mock-model-a")
        }),
        "fixture pricing missing",
    )?;
    require(
        pricing.get("group_ratio").is_some(),
        "pricing metadata missing",
    )?;
    let settings = api
        .auto_groups_setting()
        .await
        .map_err(|_| "auto_groups_setting failed")?;
    require(
        settings["data"]["max_count"] == 10,
        "fixture max_count differs",
    )?;
    require(
        api.list_tokens()
            .await
            .map_err(|_| "initial list_tokens failed")?
            .is_empty(),
        "disposable account already has tokens",
    )?;
    let name = "aicli:c104-local-probe";
    api.create_token(&json!({"name":name,"group":"auto","auto_groups":["g-cheap","g-mid"],"cross_group_retry":true,"unlimited_quota":true,"expired_time":-1}))
        .await.map_err(|_| "create_token failed")?;
    let tokens = api
        .list_tokens()
        .await
        .map_err(|_| "created list_tokens failed")?;
    require(
        tokens.len() == 1 && tokens[0]["name"] == name,
        "created token differs",
    )?;
    let id = tokens[0]["id"].as_i64().ok_or("created token missing id")?;
    let exercise = exercise_token(&api, &origin, id).await;
    // Always attempt cleanup, including if any billing or key assertion failed.
    let cleanup = api
        .delete_token(id)
        .await
        .map_err(|_| "delete_token failed");
    exercise?;
    cleanup?;
    require(
        api.list_tokens()
            .await
            .map_err(|_| "final list_tokens failed")?
            .is_empty(),
        "deleted token remains",
    )?;
    let invalid = NewApiClient::for_local_test("synthetic-invalid-access", &origin)
        .map_err(|_| "invalid-token client failed")?;
    require(
        matches!(invalid.user_self().await, Err(NewApiError::TokenInvalid)),
        "401 mapping failed",
    )?;
    println!("C1-04: PASS; all 11 wrappers; disposable rc.34 user; mock bill/stat/count match; token deleted; invalid access mapped");
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(step) = run().await {
        // Fixed labels only: never print HTTP bodies, request errors or credentials.
        eprintln!("C1-04: FAIL; {step}");
        std::process::exit(1);
    }
}
