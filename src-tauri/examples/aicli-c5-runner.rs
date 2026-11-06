//! C5-GATE real-host runner: binds the real account, starts the proxy, takes
//! over the Tier 1 clients on this machine, then keeps running until a stop
//! file appears. On stop it unbinds (byte-exact restore of every takeover and
//! deletion of the `aicli:` tokens created during the run) and writes a JSON
//! report. The access token is read from the environment and never logged.
//!
//! Usage:
//!   AICLI_TEST_ACCESS_TOKEN=... aicli-c5-runner <workdir>
//! Controls (files inside <workdir>):
//!   stop        -> unbind(delete_created_tokens=true), write cleanup.json, exit
//!   status.json -> written after bind; per-client takeover results
use std::sync::Arc;

use cc_switch_lib::aicli::cache::CacheEvents;
use cc_switch_lib::aicli::secrets::KeyringStore;
use cc_switch_lib::{AicliServices, AicliTakeoverOutcome};
use cc_switch_lib::{AppState, Database};

struct NoEvents;

impl CacheEvents for NoEvents {
    fn data_updated(&self, _: &str) {}

    fn takeover_results(&self, _: &str) {}
}

fn write_json(path: &std::path::Path, value: &serde_json::Value) {
    if let Err(error) = std::fs::write(path, serde_json::to_vec_pretty(value).unwrap_or_default()) {
        eprintln!("c5-runner: cannot write {}: {error}", path.display());
    }
}

#[tokio::main]
async fn main() {
    // 与应用启动一致：安装进程级 rustls CryptoProvider（否则出站 TLS panic）。
    let _ = rustls::crypto::ring::default_provider().install_default();
    let workdir: std::path::PathBuf = std::env::args()
        .nth(1)
        .expect("usage: aicli-c5-runner <workdir>")
        .into();
    std::fs::create_dir_all(&workdir).expect("create workdir");
    let token = std::env::var("AICLI_TEST_ACCESS_TOKEN").expect("AICLI_TEST_ACCESS_TOKEN");
    let line = std::env::var("AICLI_C5_LINE").unwrap_or_else(|_| "main".to_owned());
    let confirm_conflicts = std::env::var("AICLI_C5_CONFIRM_CONFLICTS").is_ok();

    let db = Arc::new(Database::init().expect("open ~/.aicli-client database"));
    // 与应用启动一致：播种隐藏种子 provider（接管与解析都依赖 aicli-main）。
    cc_switch_lib::aicli::seed::init(&db).expect("seed aicli providers");
    let store = Arc::new(KeyringStore::default());
    let state = AppState::new(db.clone());
    let services = AicliServices::new(state.clone(), store, Arc::new(NoEvents));

    let mut report = serde_json::json!({
        "line": line,
        "phase": "binding",
        "proxyStarted": null,
        "proxyError": null,
        "takeovers": [],
    });

    let bind = services.bind(token, line.clone()).await;
    let mut result = match bind {
        Ok(result) => result,
        Err(error) => {
            report["phase"] = "bind-failed".into();
            report["error"] = format!("{}/{}", error.code, error.message).into();
            write_json(&workdir.join("status.json"), &report);
            eprintln!("c5-runner: bind failed: {}", error.code);
            std::process::exit(2);
        }
    };

    // cc-switch 残留触发的冲突按环境变量决定是否覆盖（负责人已备份并授权还原）。
    if confirm_conflicts {
        let conflicted: Vec<String> = result
            .takeovers
            .iter()
            .filter(|outcome| outcome.error_code.as_deref() == Some("cc_switch_conflict"))
            .map(|outcome| outcome.app.clone())
            .collect();
        for app in conflicted {
            match services.take_over(app.clone(), true).await {
                Ok(()) => eprintln!("c5-runner: takeover confirmed for {app}"),
                Err(error) => eprintln!(
                    "c5-runner: confirmed takeover for {app} failed: {}",
                    error.code
                ),
            }
        }
        if let Ok(status) = services.takeover_status() {
            result.takeovers = status
                .into_iter()
                .map(|entry| AicliTakeoverOutcome {
                    app: entry.app,
                    success: entry.state == "active",
                    error_code: (entry.state != "active").then(|| "takeover_state".to_owned()),
                    error_message: None,
                })
                .collect();
        }
    }

    // 显式接管本次验收需要的三个 CLI（负责人已备份并授权覆盖；cc-switch
    // 冲突时带 confirmed=true 重试）。bind_targets 的空目标另行诊断。
    let mut explicit: Vec<serde_json::Value> = Vec::new();
    for app in ["claude", "codex", "gemini"] {
        let outcome = match services.take_over(app.to_owned(), false).await {
            Ok(()) => "ok".to_owned(),
            Err(first) => match services.take_over(app.to_owned(), true).await {
                Ok(()) => format!("ok-after-confirm({})", first.code),
                Err(second) => format!("failed({}/{})", first.code, second.code),
            },
        };
        println!("c5-runner: explicit takeover {app}: {outcome}");
        explicit.push(serde_json::json!({ "app": app, "outcome": outcome }));
    }
    let home = cc_switch_lib::get_home_dir();
    println!(
        "c5-runner: home={} .claude={} .codex={} .gemini={} .grok={}",
        home.display(),
        home.join(".claude").is_dir(),
        home.join(".codex").is_dir(),
        home.join(".gemini").is_dir(),
        home.join(".grok").is_dir(),
    );
    report["explicitTakeovers"] = explicit.into();

    report["phase"] = "running".into();
    report["proxyStarted"] = result.proxy_started.into();
    report["proxyError"] = result.proxy_error.into();
    report["takeovers"] = serde_json::to_value(&result.takeovers).unwrap_or_default();
    write_json(&workdir.join("status.json"), &report);
    println!(
        "c5-runner: bound line={line} proxy_started={} takeovers={}",
        result.proxy_started,
        serde_json::to_string(&result.takeovers).unwrap_or_default()
    );

    // 驻留：等 stop 文件，期间每 5 秒刷一次状态。
    loop {
        if workdir.join("stop").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
    let _ = std::fs::remove_file(workdir.join("stop"));

    report["phase"] = "unbinding".into();
    write_json(&workdir.join("status.json"), &report);
    match services.unbind(true).await {
        Ok(()) => {
            report["phase"] = "done".into();
            report["unbind"] = "ok".into();
        }
        Err(error) => {
            report["phase"] = "unbind-failed".into();
            report["unbind"] = format!("{}/{}", error.code, error.message).into();
            eprintln!("c5-runner: unbind failed: {}", error.code);
        }
    }
    write_json(&workdir.join("status.json"), &report);
    println!("c5-runner: finished phase={}", report["phase"]);
}
