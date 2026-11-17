//! K-01 end-to-end probe against the isolated rc.34 fixture, never a public
//! origin: bind -> catalog -> route profile -> takeover (managed token) ->
//! request through the local proxy -> server bill -> unbind.
//!
//! Needs the `test-hooks` feature and `AICLI_TEST_LINE_ORIGIN` (every line
//! points at the fixture). Run it under a throwaway HOME.
//!
//! Usage:
//!   AICLI_TEST_LINE_ORIGIN=http://127.0.0.1:13000 \
//!   AICLI_TEST_ACCESS_TOKEN=... aicli-e2e-probe <workdir>
//!
//! Writes <workdir>/e2e.json (the steps) and <workdir>/canaries.json (values
//! K-02's secret scan looks for; the workdir must be outside HOME). Exits 1 on
//! the first failed step. Secret values are never printed.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cc_switch_lib::aicli::cache::CacheEvents;
use cc_switch_lib::aicli::newapi::NewApiClient;
use cc_switch_lib::aicli::secrets::KeyringStore;
use cc_switch_lib::{AicliServices, AppState, Database};
use serde_json::{json, Value};

struct NoEvents;

impl CacheEvents for NoEvents {
    fn data_updated(&self, _: &str) {}

    fn takeover_results(&self, _: &str) {}
}

struct Report {
    path: PathBuf,
    steps: Vec<Value>,
}

impl Report {
    fn pass(&mut self, step: &str, detail: Value) {
        println!("e2e: {step}: PASS {detail}");
        self.steps
            .push(json!({ "step": step, "result": "pass", "detail": detail }));
        self.write();
    }

    fn fail(&mut self, step: &str, reason: String) -> ! {
        eprintln!("e2e: {step}: FAIL {reason}");
        self.steps
            .push(json!({ "step": step, "result": "fail", "reason": reason }));
        self.write();
        std::process::exit(1);
    }

    fn write(&self) {
        let body = serde_json::to_vec_pretty(&json!({ "steps": self.steps })).unwrap_or_default();
        if let Err(error) = std::fs::write(&self.path, body) {
            eprintln!("e2e: cannot write {}: {error}", self.path.display());
        }
    }
}

fn opencode_config(home: &Path) -> Option<(PathBuf, Value)> {
    let dir = home.join(".config").join("opencode");
    ["opencode.json", "opencode.jsonc"].iter().find_map(|name| {
        let path = dir.join(name);
        let text = std::fs::read_to_string(&path).ok()?;
        let value = json5::from_str::<Value>(&text).ok()?;
        Some((path, value))
    })
}

async fn aicli_tokens(api: &NewApiClient) -> Result<Vec<i64>, String> {
    let tokens = api
        .list_tokens()
        .await
        .map_err(|error| format!("list_tokens: {error:?}"))?;
    Ok(tokens
        .iter()
        .filter(|token| {
            token["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("aicli:"))
        })
        .filter_map(|token| token["id"].as_i64())
        .collect())
}

#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let workdir: PathBuf = std::env::args()
        .nth(1)
        .expect("usage: aicli-e2e-probe <workdir>")
        .into();
    std::fs::create_dir_all(&workdir).expect("create workdir");
    let mut report = Report {
        path: workdir.join("e2e.json"),
        steps: Vec::new(),
    };
    let origin = std::env::var("AICLI_TEST_LINE_ORIGIN").unwrap_or_default();
    if !origin.starts_with("http://127.0.0.1:") {
        report.fail(
            "environment",
            "AICLI_TEST_LINE_ORIGIN must be the local fixture".into(),
        );
    }
    let access_token = std::env::var("AICLI_TEST_ACCESS_TOKEN")
        .unwrap_or_else(|_| report.fail("environment", "AICLI_TEST_ACCESS_TOKEN unset".into()));
    let home = cc_switch_lib::get_home_dir();
    if home.starts_with(&workdir) || workdir.starts_with(&home) {
        report.fail("environment", "workdir and HOME must not nest".into());
    }

    let db = Arc::new(Database::init().expect("open the AICLI database"));
    cc_switch_lib::aicli::seed::init(&db).expect("seed aicli providers");
    let state = AppState::new(db.clone());
    let services = AicliServices::new(state, Arc::new(KeyringStore::default()), Arc::new(NoEvents));
    let api = NewApiClient::for_local_test(&access_token, &origin)
        .unwrap_or_else(|error| report.fail("environment", format!("fixture client: {error:?}")));

    // 1. Bind.
    match services.bind(access_token.clone(), "main".into()).await {
        Ok(result) if result.proxy_error.is_none() => report.pass(
            "bind",
            json!({ "proxyStarted": result.proxy_started, "account": result.account.is_some() }),
        ),
        Ok(result) => report.fail("bind", format!("proxy: {:?}", result.proxy_error)),
        Err(error) => report.fail("bind", format!("{}", error.code)),
    }

    // 2. Catalog (synced from /api/pricing and the user's groups after bind).
    let mut catalog = None;
    for _ in 0..60 {
        if let Ok(Some(found)) = services.get_catalog() {
            if !found.models.is_empty() {
                catalog = Some(found);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let catalog = catalog.unwrap_or_else(|| report.fail("catalog", "no models after 30s".into()));
    report.pass(
        "catalog",
        json!({ "models": catalog.models.keys().collect::<Vec<_>>(), "groups": catalog.groups.len() }),
    );

    // 3. Route profile: a user copy of the low-cost preset, bound to OpenCode.
    let preset = services
        .get_profiles()
        .ok()
        .and_then(|view| view.presets.into_iter().find(|p| p.id == "low_cost"))
        .unwrap_or_else(|| report.fail("profile", "low_cost preset missing".into()));
    let mut copy = preset.clone();
    copy.id = "e2e-profile".into();
    copy.name = "E2E".into();
    copy.source = "system_copy".into();
    copy.preset_id = Some("low_cost".into());
    copy.revision = 0;
    // As the editor's "confirm all by group order": a model in more than one
    // selected group is a conflict the user confirms before saving.
    let pending: Vec<String> = catalog
        .models
        .iter()
        .filter(|(name, model)| {
            !copy.body.resolved_conflicts.contains(name)
                && model
                    .groups
                    .iter()
                    .filter(|group| copy.body.selected_groups.contains(group))
                    .count()
                    > 1
        })
        .map(|(name, _)| name.clone())
        .collect();
    copy.body.resolved_conflicts.extend(pending);
    let saved = services
        .save_profile(copy)
        .unwrap_or_else(|error| report.fail("profile", format!("save: {}", error.code)));
    if let Err(error) =
        services.set_active_profile("app".into(), "opencode".into(), Some(saved.id.clone()))
    {
        report.fail("profile", format!("bind to opencode: {}", error.code));
    }
    report.pass(
        "profile",
        json!({ "id": saved.id, "revision": saved.revision }),
    );

    // Privacy on: the request below carries a fake key that must never be stored.
    if let Err(error) = services.set_privacy(true, "block".into(), false, None) {
        report.fail("privacy", format!("set_privacy: {}", error.code));
    }

    // 4. Take over OpenCode: the managed sk- token is created for its models.
    let mut taken = Err(String::new());
    for _ in 0..20 {
        match services.take_over("opencode".into(), false).await {
            Ok(()) => {
                taken = Ok(());
                break;
            }
            Err(error) => taken = Err(error.code),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if let Err(code) = taken {
        report.fail("takeover", format!("opencode: {code}"));
    }
    let (config_path, config) = opencode_config(&home)
        .unwrap_or_else(|| report.fail("takeover", "no opencode config".into()));
    let provider = &config["provider"]["aicli"];
    let base = provider["options"]["baseURL"]
        .as_str()
        .unwrap_or_else(|| report.fail("takeover", "no aicli baseURL".into()))
        .to_owned();
    let local_key = provider["options"]["apiKey"]
        .as_str()
        .unwrap_or_else(|| report.fail("takeover", "no aicli apiKey".into()))
        .to_owned();
    let model = provider["models"]
        .as_object()
        .and_then(|models| models.keys().next().cloned())
        .unwrap_or_else(|| report.fail("takeover", "no models projected".into()));
    let text = std::fs::read_to_string(&config_path).unwrap_or_default();
    if text.contains("sk-") || text.contains(&access_token) {
        report.fail("takeover", "client config holds a server credential".into());
    }
    report.pass("takeover", json!({ "base": base, "model": model }));

    let token_ids = aicli_tokens(&api)
        .await
        .unwrap_or_else(|error| report.fail("token", error));
    if token_ids.is_empty() {
        report.fail("token", "no aicli: token on the server".into());
    }
    let keys = api
        .batch_token_keys(&token_ids)
        .await
        .unwrap_or_else(|error| report.fail("token", format!("batch_token_keys: {error:?}")));
    report.pass("token", json!({ "aicliTokens": token_ids.len() }));

    // 5. A request through the local proxy, as OpenCode sends it.
    let canary_secret = format!("sk-canary{}", uuid::Uuid::new_v4().simple().to_string());
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("http client")
        .post(format!("{}/chat/completions", base.trim_end_matches('/')))
        .bearer_auth(&local_key)
        .json(&json!({
            "model": model,
            "messages": [{ "role": "user", "content": format!("K-01 deploy with {canary_secret}") }],
            "max_tokens": 8
        }))
        .send()
        .await
        .unwrap_or_else(|error| report.fail("request", format!("send: {error}")));
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() || !body.contains("mock") {
        report.fail("request", format!("status {status}"));
    }
    // The proxy writes its log row after the reply; wait for it.
    let mut row: Option<(String, String)> = None;
    for _ in 0..40 {
        row = cc_switch_lib::aicli::routing_test_support::connection(&db)
            .query_row(
                "SELECT request_id, app_type FROM proxy_request_logs ORDER BY created_at DESC, rowid DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let (request_id, app_type) =
        row.unwrap_or_else(|| report.fail("request", "no proxy_request_logs row".into()));
    if app_type != "opencode" {
        report.fail("request", format!("app_type {app_type}"));
    }
    report.pass(
        "request",
        json!({ "status": status.as_u16(), "appType": app_type }),
    );

    // 6. The server's bill, backfilled by X-Oneapi-Request-Id.
    let mut billed = None;
    for _ in 0..90 {
        if let Ok(Some(bill)) = services.get_request_bill(request_id.clone()) {
            if bill.billed_at.is_some() {
                billed = Some(bill);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
    let bill = billed.unwrap_or_else(|| report.fail("bill", "not backfilled within 90s".into()));
    report.pass(
        "bill",
        json!({
            "upstreamRequestId": bill.upstream_request_id.is_some(),
            "serverGroup": bill.server_group,
            "billedQuota": bill.billed_quota,
            "lineId": bill.line_id,
            "profileId": bill.profile_id,
        }),
    );

    let canaries = json!({
        "access_token": access_token,
        "managed_keys": keys.values().collect::<Vec<_>>(),
        "local_key": local_key,
        "privacy_secret": canary_secret,
    });
    let canary_path = workdir.join("canaries.json");
    if let Err(error) = std::fs::write(
        &canary_path,
        serde_json::to_vec(&canaries).unwrap_or_default(),
    ) {
        report.fail("canaries", format!("write: {error}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&canary_path, std::fs::Permissions::from_mode(0o600));
    }

    // 7. Unbind: takeovers restored byte for byte, created tokens deleted.
    if let Err(error) = services.unbind(true).await {
        report.fail("unbind", format!("{}", error.code));
    }
    if opencode_config(&home).is_some() {
        report.fail("unbind", "opencode config left behind".into());
    }
    let left = aicli_tokens(&api)
        .await
        .unwrap_or_else(|error| report.fail("unbind", error));
    if !left.is_empty() {
        report.fail("unbind", format!("{} aicli: tokens left", left.len()));
    }
    report.pass("unbind", json!({ "tokensLeft": 0 }));
    println!("e2e: all steps passed");
}
