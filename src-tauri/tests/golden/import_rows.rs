//! ① 首启导入写出的 DB 原始行。
//!
//! 重构不动数据库（不升 schema、不回填、不剥离存量行），降级后旧版直接读这些行，
//! 所以新增供应商时写进 DB 的字节要保持原样。

use cc_switch_lib::{import_default_config_test_hook, AppType};

use crate::support::{create_test_state, reset_test_fs, test_mutex};
use crate::util::{assert_golden, dump_provider_rows, write_home_file};

/// 首启：先把 live 导入成 `default`，再 seed 官方预设（顺序同 `lib.rs` 的启动流程）。
/// live 里供应商以外的部分进了通用配置片段，片段也一起锁。关键字段（模型名、推理档位
/// 等）不进片段、留在 `default` 行里：片段已冻结，收进去就再也写不回 live。
///
/// `settings_sort` 见 [`crate::util::sort_objects`]：Gemini 的 `.env` 解析顺序不稳定。
fn first_run_import(app: AppType, snapshot: &str, settings_sort: &[&str]) {
    let state = create_test_state().expect("create test state");
    import_default_config_test_hook(&state, app.clone()).expect("import live config");
    state
        .db
        .seed_official_providers_for_app(app.clone())
        .expect("seed official providers");
    let snippet = state
        .db
        .get_config_snippet(app.as_str())
        .expect("read common config snippet");
    let dump = format!(
        "{}\ncommon config snippet: {}\n",
        dump_provider_rows(&app, str::to_string, settings_sort),
        snippet.as_deref().unwrap_or("NULL"),
    );
    assert_golden(snapshot, &dump);
}

#[test]
fn first_run_claude_rows() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    write_home_file(
        ".claude/settings.json",
        r#"{
  "env": {
    "ANTHROPIC_AUTH_TOKEN": "sk-live",
    "ANTHROPIC_BASE_URL": "https://relay.example",
    "ANTHROPIC_MODEL": "claude-sonnet-4-5",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"
  },
  "permissions": {
    "allow": ["Bash(git status)"]
  },
  "hooks": {
    "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }]
  },
  "enabledPlugins": {
    "example@marketplace": true
  },
  "model": "opus",
  "statusLine": { "type": "command", "command": "~/.claude/statusline.sh" }
}"#,
    );
    first_run_import(AppType::Claude, "rows/first-run-claude.txt", &[]);
}

#[test]
fn first_run_codex_rows() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    write_home_file(".codex/auth.json", r#"{"OPENAI_API_KEY":"sk-live"}"#);
    write_home_file(
        ".codex/config.toml",
        r#"model_provider = "relay"
model = "gpt-5-codex"
model_reasoning_effort = "high"

[model_providers.relay]
name = "relay"
base_url = "https://relay.example/v1"
wire_api = "responses"

[projects."/Users/me/repo"]
trust_level = "trusted"

[mcp_servers.external]
command = "external"
"#,
    );
    first_run_import(AppType::Codex, "rows/first-run-codex.txt", &[]);
}

#[test]
fn first_run_gemini_rows() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    write_home_file(
        ".gemini/.env",
        "GEMINI_API_KEY=g-live\nGOOGLE_GEMINI_BASE_URL=https://relay.example\nGEMINI_MODEL=gemini-2.5-pro\n",
    );
    write_home_file(
        ".gemini/settings.json",
        r#"{
  "security": { "auth": { "selectedType": "gemini-api-key" } },
  "ui": { "theme": "GitHub" }
}"#,
    );
    first_run_import(AppType::Gemini, "rows/first-run-gemini.txt", &["/env"]);
}

#[test]
fn first_run_grokbuild_rows() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    write_home_file(
        ".grok/config.toml",
        r#"[models]
default = "grok-4.5"

[model."grok-4.5"]
model = "grok-4.5"
base_url = "https://relay.example/v1"
name = "Relay"
api_key = "xai-live"
api_backend = "responses"
context_window = 500000
"#,
    );
    first_run_import(AppType::GrokBuild, "rows/first-run-grokbuild.txt", &[]);
}
