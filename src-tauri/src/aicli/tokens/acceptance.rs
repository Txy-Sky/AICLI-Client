//! CI-only acceptance on disposable rc.34 users; no public origins or host keychain.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::super::account::{AccountError, AccountLifecycle, AccountService};
use super::super::catalog::Catalog;
use super::super::profiles::{compile, preset};
use super::super::secrets::FileFallbackStore;

fn require(condition: bool, label: &'static str) -> Result<(), &'static str> {
    if condition {
        Ok(())
    } else {
        Err(label)
    }
}

struct Counting {
    api: NewApiClient,
    creates: AtomicUsize,
    batches: AtomicUsize,
}

impl Counting {
    fn new(api: NewApiClient) -> Self {
        Self {
            api,
            creates: AtomicUsize::new(0),
            batches: AtomicUsize::new(0),
        }
    }
}

impl TokenRemote for Counting {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Value>, NewApiError>> {
        Box::pin(self.api.list_tokens())
    }
    fn create<'a>(&'a self, specification: &'a Value) -> BoxFuture<'a, Result<(), NewApiError>> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Box::pin(self.api.create_token(specification))
    }
    fn keys<'a>(
        &'a self,
        ids: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, NewApiError>> {
        self.batches.fetch_add(1, Ordering::SeqCst);
        Box::pin(self.api.batch_token_keys(ids))
    }
}

struct NoTakeovers;
impl AccountLifecycle for NoTakeovers {
    fn refresh(&self) {}
    fn release_all(&self) -> BoxFuture<'_, Result<(), AccountError>> {
        Box::pin(async { Ok(()) })
    }
}

async fn request(origin: &str, secrets: &dyn SecretStore, id: i64) -> Result<(), &'static str> {
    let key = secrets
        .get(&format!("token:{id}"))
        .map_err(|_| "credential lookup failed")?
        .ok_or("credential absent")?;
    let response=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(20)).build().map_err(|_|"model client failed")?
        .post(format!("{}/v1/chat/completions",origin.trim_end_matches('/')))
        .bearer_auth(key).json(&json!({"model":"mock-model-a","messages":[{"role":"user","content":"C1-09 fixture"}],"max_tokens":8}))
        .send().await.map_err(|_|"model request failed")?;
    require(
        response.status().is_success(),
        "managed key request rejected",
    )?;
    let body: Value = response
        .json()
        .await
        .map_err(|_| "model response invalid")?;
    require(
        body["choices"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "model response empty",
    )
}

/// Only called by the existing aicli-newapi-probe after its C1-04 checks.
pub async fn run(origin: &str, access: &str) -> Result<(), &'static str> {
    let api = NewApiClient::for_local_test(access, origin).map_err(|_| "local API rejected")?;
    require(
        api.list_tokens()
            .await
            .map_err(|_| "initial listing failed")?
            .is_empty(),
        "fixture not disposable",
    )?;
    let result = exercise(&api, origin, access).await;
    // Cleanup is HTTP-only and only touches this disposable user's test tokens.
    let cleanup = async {
        for candidate in api
            .list_tokens()
            .await
            .map_err(|_| "cleanup listing failed")?
        {
            let id = candidate["id"].as_i64().ok_or("cleanup identity invalid")?;
            api.delete_token(id)
                .await
                .map_err(|_| "cleanup deletion failed")?;
        }
        Ok::<_, &'static str>(())
    }
    .await;
    result?;
    cleanup?;
    println!("C1-09: PASS; real rc.34 reuse/restrictions; 69 models one key batch; two isolated device HOMEs; both model requests; unbind(false) leaves second device usable; deleted key recovered");
    Ok(())
}

async fn exercise(api: &NewApiClient, origin: &str, access: &str) -> Result<(), &'static str> {
    let root = tempfile::tempdir().map_err(|_| "device root failed")?;
    let home_a = root.path().join("device-a");
    let home_b = root.path().join("device-b");
    std::fs::create_dir_all(&home_a).map_err(|_| "first home failed")?;
    std::fs::create_dir_all(&home_b).map_err(|_| "second home failed")?;
    // Configure each store from its own temporary HOME, then restore the
    // process HOME before any async work. These are fallback files, not the
    // host's keychain or real client configuration directories.
    let original = std::env::var_os("HOME").ok_or("isolated home absent")?;
    let original_test = std::env::var_os("CC_SWITCH_TEST_HOME").ok_or("test home absent")?;
    std::env::set_var("HOME", &home_a);
    std::env::set_var("CC_SWITCH_TEST_HOME", &home_a);
    let secrets_a = Arc::new(FileFallbackStore::default());
    std::env::set_var("HOME", &home_b);
    std::env::set_var("CC_SWITCH_TEST_HOME", &home_b);
    let secrets_b = Arc::new(FileFallbackStore::default());
    std::env::set_var("HOME", original);
    std::env::set_var("CC_SWITCH_TEST_HOME", original_test);
    secrets_a
        .set("access-token", access)
        .map_err(|_| "first access storage failed")?;
    secrets_b
        .set("access-token", access)
        .map_err(|_| "second access storage failed")?;
    let account = api.user_self().await.map_err(|_| "account read failed")?;
    let db_a = Arc::new(Database::memory().map_err(|_| "first database failed")?);
    let db_b = Arc::new(Database::memory().map_err(|_| "second database failed")?);
    for db in [&db_a, &db_b] {
        db.conn
            .lock()
            .map_err(|_| "database lock failed")?
            .execute(
                "INSERT INTO aicli_account(id,user_id,username,user_group) VALUES(1,?1,?2,?3)",
                params![
                    account["data"]["id"].as_i64().ok_or("account id missing")?,
                    account["data"]["username"]
                        .as_str()
                        .ok_or("account name missing")?,
                    account["data"]["group"]
                        .as_str()
                        .ok_or("account group missing")?
                ],
            )
            .map_err(|_| "local binding fixture failed")?;
    }
    let remote_a = Arc::new(Counting::new(api.clone()));
    let remote_b = Arc::new(Counting::new(api.clone()));
    let service_a = TokenService::new(db_a.clone(), secrets_a.clone(), remote_a.clone());
    let service_b = TokenService::new(db_b.clone(), secrets_b.clone(), remote_b.clone());
    // Real server rows cover all reuse restrictions. This account belongs to
    // this integration test alone; no alice/bob or online account is involved.
    for (name, extra) in [
        (
            "user-limited",
            json!({"unlimited_quota":false,"remain_quota":1000}),
        ),
        (
            "user-model-limited",
            json!({"model_limits_enabled":true,"model_limits":"mock-model-a"}),
        ),
        ("user-no-retry", json!({"cross_group_retry":false})),
        ("user-compatible", json!({})),
    ] {
        let mut spec = json!({"name":name,"group":"auto","auto_groups":["g-cheap","g-mid"],"cross_group_retry":true,"unlimited_quota":true,"expired_time":-1});
        spec.as_object_mut()
            .ok_or("spec invalid")?
            .extend(extra.as_object().ok_or("extra invalid")?.clone());
        api.create_token(&spec)
            .await
            .map_err(|_| "fixture token creation failed")?;
    }
    let initial = api
        .list_tokens()
        .await
        .map_err(|_| "reuse listing failed")?;
    let expected = initial
        .iter()
        .find(|v| v["name"] == "user-compatible")
        .and_then(|v| v["id"].as_i64())
        .ok_or("reusable id missing")?;
    let order = vec!["g-cheap".to_owned(), "g-mid".to_owned()];
    require(
        service_a
            .ensure_token(&order, true, &[])
            .await
            .map_err(|_| "user reuse failed")?
            == expected,
        "restricted user token reused",
    )?;
    require(
        remote_a.creates.load(Ordering::SeqCst) == 0,
        "reuse created a token",
    )?;
    let after = api
        .list_tokens()
        .await
        .map_err(|_| "post-reuse listing failed")?;
    require(initial.len() == after.len(), "reuse changed remote rows")?;
    // Start clean for the 69-model batch and multi-device creation checks.
    for candidate in initial {
        api.delete_token(candidate["id"].as_i64().ok_or("fixture id missing")?)
            .await
            .map_err(|_| "fixture deletion failed")?;
    }
    db_a.conn
        .lock()
        .map_err(|_| "local lock failed")?
        .execute("DELETE FROM aicli_managed_token", [])
        .map_err(|_| "local reset failed")?;
    secrets_a
        .delete(&format!("token:{expected}"))
        .map_err(|_| "local secret reset failed")?;
    let groups = api.user_groups().await.map_err(|_| "groups failed")?;
    let pricing = api.pricing().await.map_err(|_| "pricing failed")?;
    let mut catalog = Catalog::from_snapshots(&groups, &pricing).map_err(|_| "catalog invalid")?;
    let base = catalog
        .models
        .get("mock-model-a")
        .ok_or("fixture model absent")?
        .clone();
    catalog.models.clear();
    for i in 0..69 {
        catalog
            .models
            .insert(format!("fixture-{i:02}"), base.clone());
    }
    // The seed exposes usable groups but does not configure an auto-group
    // recommendation. Use the cost preset, which includes those real groups.
    let routes = compile(
        &preset("low_cost", &catalog).map_err(|_| "preset failed")?,
        &catalog,
        10,
    )
    .map_err(|_| "compile failed")?;
    require(routes.len() == 69, "model fixture count differs")?;
    let orders: Vec<_> = routes.into_iter().map(|r| r.ordered_groups).collect();
    let before = remote_a.batches.load(Ordering::SeqCst);
    let first = service_a
        .ensure_compilation(&orders, true, &orders)
        .await
        .map_err(|error| {
            // Only public, fixed error variants and counters reach CI logs;
            // never print TokenLimit candidates, keys or HTTP bodies.
            let kind = match error {
                TokenError::Api(api) => match api {
                    NewApiError::InvalidResponse => "invalid API response",
                    NewApiError::Rejected => "API rejected",
                    NewApiError::Http(_) => "HTTP status",
                    NewApiError::TokenInvalid => "access invalid",
                    _ => "other API error",
                },
                TokenError::InvalidGroups => "invalid groups",
                TokenError::Database => "local metadata",
                TokenError::Secret(_) => "secret storage",
                TokenError::RateLimited { .. } => "rate limited",
                _ => "other token error",
            };
            eprintln!(
                "C1-04: FAIL; C1-09 compilation category={kind}; creates={}; batches={}; groups={}",
                remote_a.creates.load(Ordering::SeqCst),
                remote_a.batches.load(Ordering::SeqCst),
                orders.first().map_or(0, Vec::len),
            );
            "first compilation failed"
        })?;
    require(
        remote_a.batches.load(Ordering::SeqCst) == before + 1,
        "69 models did not use exactly one batch",
    )?;
    let created = remote_a.creates.load(Ordering::SeqCst);
    require(created == first.len(), "duplicate first-device creation")?;
    require(
        service_a
            .ensure_compilation(&orders, true, &orders)
            .await
            .map_err(|_| "repeat failed")?
            == first,
        "repeat identity changed",
    )?;
    require(
        remote_a.creates.load(Ordering::SeqCst) == created
            && remote_a.batches.load(Ordering::SeqCst) == before + 1,
        "repeat used HTTP key operations",
    )?;
    let second = service_b
        .ensure_compilation(&orders, true, &orders)
        .await
        .map_err(|_| "second device failed")?;
    require(
        first == second && remote_b.creates.load(Ordering::SeqCst) == 0,
        "second device did not reuse identical tokens",
    )?;
    let id = *first.values().next().ok_or("compiled token absent")?;
    request(origin, secrets_a.as_ref(), id).await?;
    request(origin, secrets_b.as_ref(), id).await?;
    require(
        home_a.join(".aicli-client/secrets.json").is_file()
            && home_b.join(".aicli-client/secrets.json").is_file(),
        "device credential files not isolated",
    )?;
    let account_a = AccountService::new(db_a, secrets_a.clone(), Arc::new(NoTakeovers))
        .map_err(|_| "account lifecycle failed")?;
    account_a
        .unbind(false)
        .await
        .map_err(|_| "first unbind failed")?;
    require(
        secrets_a
            .get("access-token")
            .map_err(|_| "unbound read failed")?
            .is_none(),
        "first access retained",
    )?;
    require(
        secrets_b
            .get("access-token")
            .map_err(|_| "second access read failed")?
            .as_deref()
            == Some(access),
        "second access changed",
    )?;
    request(origin, secrets_b.as_ref(), id).await?;
    api.delete_token(id)
        .await
        .map_err(|_| "web deletion failed")?;
    let removed = secrets_b
        .get(&format!("token:{id}"))
        .map_err(|_| "old key missing")?
        .ok_or("old key absent")?;
    let status = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|_| "revocation client failed")?
        .get(format!("{}/v1/models", origin.trim_end_matches('/')))
        .bearer_auth(removed)
        .send()
        .await
        .map_err(|_| "revocation request failed")?
        .status();
    require(
        status.as_u16() == 401,
        "deleted model key remains authorized",
    )?;
    let record = service_b
        .records()
        .map_err(|_| "recovery metadata failed")?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or("recovery order absent")?;
    let recovered = service_b
        .recover_unauthorized(id, &record.groups, true, &orders)
        .await
        .map_err(|_| "401 recovery failed")?;
    require(recovered != id, "deleted id reused")?;
    request(origin, secrets_b.as_ref(), recovered).await?;
    Ok(())
}
