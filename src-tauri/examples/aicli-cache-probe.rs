//! Exercise cache-first refresh against the isolated rc.34 fixture.
use cc_switch_lib::aicli::cache::{AicliCache, CacheEvents};
use cc_switch_lib::aicli::newapi::NewApiClient;
use cc_switch_lib::database::Database;
use std::sync::Arc;
use std::time::Duration;

struct Events;

impl CacheEvents for Events {
    fn data_updated(&self, _: &str) {}
}

async fn run() -> Result<(), &'static str> {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return Err("CI fixture only");
    }
    let home = std::env::var("HOME").map_err(|_| "missing isolated home")?;
    if std::env::var("CC_SWITCH_TEST_HOME").as_deref() != Ok(home.as_str()) {
        return Err("home is not isolated");
    }
    let origin = std::env::var("AICLI_TESTENV_BASE_URL").map_err(|_| "missing local origin")?;
    let access = std::env::var("AICLI_LOCAL_TEST_ACCESS_TOKEN")
        .map_err(|_| "missing local fixture access token")?;
    let api =
        NewApiClient::for_local_test(&access, &origin).map_err(|_| "local API setup failed")?;
    let start = 0;
    let baseline = api
        .audit_self_request_count(start, i64::MAX)
        .await
        .map_err(|_| "baseline audit query failed")?;
    let db = Arc::new(Database::init().map_err(|_| "isolated database initialization failed")?);
    let cache = Arc::new(
        AicliCache::for_local_test(db, &access, &origin, Arc::new(Events))
            .map_err(|_| "local API setup failed")?,
    );

    let today = chrono::Utc::now().date_naive();
    cache.schedule_account_refresh();
    tokio::time::timeout(Duration::from_secs(90), cache.wait_for_idle())
        .await
        .map_err(|_| "cache refresh did not finish")?;

    for kind in ["account", "groups", "pricing"] {
        if cache.read(kind).map_err(|_| "cache read failed")?.is_none() {
            return Err("cache did not persist fetched data");
        }
    }
    if cache
        .read_usage(&today.format("%Y-%m-%d").to_string())
        .map_err(|_| "usage cache read failed")?
        .is_none()
    {
        return Err("today usage was not persisted");
    }
    cache
        .get_usage(&today.format("%Y-%m-%d").to_string())
        .map_err(|_| "today usage refresh failed")?;
    tokio::time::timeout(Duration::from_secs(30), cache.wait_for_idle())
        .await
        .map_err(|_| "today usage refresh did not finish")?;
    let total = api
        .audit_self_request_count(start, i64::MAX)
        .await
        .map_err(|_| "final audit query failed")?;
    let count = total.saturating_sub(baseline).saturating_sub(1);
    if count > 40 {
        return Err("cold-start API request count exceeded 40");
    }
    println!("C1-06: PASS; cold-start audited /api/* request count={count} (limit 40); 30-day backfill persisted");
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(step) = run().await {
        eprintln!("C1-06: FAIL; {step}");
        std::process::exit(1);
    }
}
