//! F-09：隐私脱敏记录（隐私页「没让上游看到的」）。
//!
//! 每个真正被改动的请求记一行：时间、客户端、线路、结果（遮盖 / 降级放行 /
//! 阻断 / 媒体放行）、数量、各类别的个数，以及占位符的 SHA-256 前 16 位（只用来
//! 跨请求统计「不同的值」）。**不存原值，也不存占位符本身**；占位符原本就会发给
//! 上游，再做一次哈希存在本机不会多泄露任何信息。
//!
//! 写库走 `spawn_blocking`，扫描占位符也在那里做，不占请求路径；保留 30 天，
//! 写入时最多每 6 小时顺带清理一次。表不参与同步（`database/backup.rs`）。

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

use chrono::{Duration, Local, NaiveDate, TimeZone};
use regex::Regex;
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::placeholder::PLACEHOLDER_PATTERN;
use crate::error::AppError;
use crate::Database;

/// 记录保留天数（页脚文案与清理一致）。
pub const RETENTION_DAYS: i64 = 30;
/// 单行最多记录的不同值 id 个数（防止超大请求撑大一行）。
pub const MAX_VALUE_IDS: usize = 200;
const PRUNE_INTERVAL_MS: i64 = 6 * 60 * 60 * 1000;
static LAST_PRUNE_MS: AtomicI64 = AtomicI64::new(0);

/// 本地鉴权认出的客户端（`local_auth::authenticate` 放进请求扩展）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientApp(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 遮盖了值并照常发出。
    Masked,
    /// 可用性优先下引擎失败，原文放行。
    Degraded,
    /// 被拦下（隐私优先下的引擎失败、媒体或压缩流）。
    Blocked,
    /// 含未扫描媒体，按设置放行。
    MediaPassed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Masked => "masked",
            Outcome::Degraded => "degraded",
            Outcome::Blocked => "blocked",
            Outcome::MediaPassed => "media_passed",
        }
    }
}

/// 一条待写的记录；`masked` 是遮盖后的请求体（只在后台线程里扫描占位符，随即丢弃）。
pub struct PendingEvent {
    pub ts_ms: i64,
    pub app_type: Option<String>,
    pub line_id: Option<String>,
    pub outcome: Outcome,
    pub reason: Option<&'static str>,
    pub masked: Option<bytes::Bytes>,
}

/// 占位符扫描结果。
#[derive(Debug, Default, PartialEq)]
pub struct KindScan {
    pub values: u64,
    pub kinds: BTreeMap<String, u64>,
    /// 去重后的值 id（占位符 SHA-256 前 16 位十六进制），最多 [`MAX_VALUE_IDS`] 个。
    pub ids: Vec<String>,
    pub distinct: u64,
}

fn placeholder_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(PLACEHOLDER_PATTERN).expect("placeholder pattern"))
}

fn value_id(placeholder: &str) -> String {
    let digest = Sha256::digest(placeholder.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 数出文本里的占位符：总数、每个类别的个数、去重后的值 id。
pub fn scan_kinds(text: &str) -> KindScan {
    let mut scan = KindScan::default();
    let mut seen: HashSet<&str> = HashSet::new();
    for found in placeholder_regex().find_iter(text) {
        let placeholder = found.as_str();
        scan.values += 1;
        let kind = super::rules::known::kind_of(placeholder);
        *scan.kinds.entry(kind.to_owned()).or_default() += 1;
        if seen.insert(placeholder) && scan.ids.len() < MAX_VALUE_IDS {
            scan.ids.push(value_id(placeholder));
        }
    }
    scan.distinct = seen.len() as u64;
    scan
}

/// 类别 → 隐私页的四组（机密 / 个人信息 / 网络与设备 / 自定义）。用户自定义规则
/// 的类别名不在表里时归为自定义。
pub fn category_of(kind: &str) -> &'static str {
    match kind {
        "API_KEY" | "SECRET" | "TOKEN" | "PASSWORD" | "DB_PASSWORD" | "PRIVATE_KEY"
        | "CREDENTIAL" | "KEY" | "AKIA" | "ASIA" | "LTAI" | "JWT" => "secret",
        "EMAIL" | "PHONE" | "ID_CARD" | "BANK_CARD" | "IBAN" | "SSN" | "PASSPORT" | "BIRTHDAY"
        | "PERSON" => "personal",
        "HOSTNAME" | "IP" | "MAC" | "USERNAME" | "BUCKET" | "SERIAL" => "network",
        _ => "custom",
    }
}

fn local_day(ts_ms: i64) -> String {
    Local
        .timestamp_millis_opt(ts_ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// 本地日期 `day` 的 0 点（毫秒）。
fn day_start_ms(day: NaiveDate) -> i64 {
    day.and_hms_opt(0, 0, 0)
        .and_then(|naive| Local.from_local_datetime(&naive).earliest())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

/// 写一条记录（同步；调用方在后台线程里调用）。`masked` 为空或没有占位符的
/// 「遮盖」记录不写：只记真正改过的请求。
pub fn insert(conn: &Connection, event: &PendingEvent) -> Result<bool, AppError> {
    let scan = event
        .masked
        .as_ref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(scan_kinds)
        .unwrap_or_default();
    if event.outcome == Outcome::Masked && scan.values == 0 {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO aicli_privacy_event
            (ts, day, app_type, line_id, outcome, reason, value_count, distinct_count,
             kinds_json, value_ids_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            event.ts_ms,
            local_day(event.ts_ms),
            event.app_type,
            event.line_id,
            event.outcome.as_str(),
            event.reason,
            scan.values as i64,
            scan.distinct as i64,
            serde_json::to_string(&scan.kinds).unwrap_or_else(|_| "{}".into()),
            serde_json::to_string(&scan.ids).unwrap_or_else(|_| "[]".into()),
        ],
    )?;
    Ok(true)
}

/// 删掉保留期以前的记录，返回删除行数。
pub fn prune(conn: &Connection, now_ms: i64) -> Result<usize, AppError> {
    let cutoff = now_ms - RETENTION_DAYS * 24 * 60 * 60 * 1000;
    Ok(conn.execute("DELETE FROM aicli_privacy_event WHERE ts < ?1", [cutoff])?)
}

/// 写一条记录并按需清理（同步版本，测试与后台线程共用）。
pub fn record_blocking(db: &Database, event: &PendingEvent) -> Result<bool, AppError> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let wrote = insert(&conn, event)?;
    let last = LAST_PRUNE_MS.load(Ordering::Relaxed);
    if event.ts_ms - last >= PRUNE_INTERVAL_MS {
        LAST_PRUNE_MS.store(event.ts_ms, Ordering::Relaxed);
        prune(&conn, event.ts_ms)?;
    }
    Ok(wrote)
}

/// 中间件用：放到阻塞线程池里写，失败只记一条警告（日志不含内容）。
pub fn record(db: Arc<Database>, event: PendingEvent) {
    tokio::task::spawn_blocking(move || {
        if let Err(error) = record_blocking(&db, &event) {
            log::warn!("隐私脱敏记录写入失败：{error}");
        }
    });
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---- 查询 -------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrivacyDay {
    pub day: String,
    pub values: u64,
    pub requests: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrivacyCount {
    pub key: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrivacySummary {
    /// 从旧到新，正好 `range_days` 天（没有记录的一天 values=0）。
    pub days: Vec<PrivacyDay>,
    pub total_values: u64,
    pub distinct_values: u64,
    /// 遮盖过值的请求数。
    pub requests: u64,
    pub blocked: u64,
    pub degraded: u64,
    pub media_passed: u64,
    /// 四组类别各自的个数。
    pub categories: Vec<PrivacyCount>,
    /// 细分类别，按个数从多到少。
    pub kinds: Vec<PrivacyCount>,
    /// 按客户端的遮盖个数，从多到少（未识别的客户端为 "other"）。
    pub apps: Vec<PrivacyCount>,
    pub range_days: u32,
    pub retention_days: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrivacyEventView {
    pub id: i64,
    pub ts: i64,
    pub app_type: Option<String>,
    pub line_id: Option<String>,
    pub outcome: String,
    pub reason: Option<String>,
    pub value_count: u64,
    pub distinct_count: u64,
    pub kinds: Vec<PrivacyCount>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrivacyEventPage {
    pub items: Vec<PrivacyEventView>,
    pub next_before_id: Option<i64>,
}

struct Row {
    id: i64,
    ts: i64,
    day: String,
    app_type: Option<String>,
    line_id: Option<String>,
    outcome: String,
    reason: Option<String>,
    value_count: i64,
    distinct_count: i64,
    kinds: BTreeMap<String, u64>,
    ids: Vec<String>,
}

fn clamp_days(days: u32) -> u32 {
    days.clamp(1, RETENTION_DAYS as u32)
}

fn range_start(today: NaiveDate, days: u32) -> NaiveDate {
    today - Duration::days(i64::from(clamp_days(days)) - 1)
}

fn read_rows(
    conn: &Connection,
    since_ms: i64,
    before_id: Option<i64>,
) -> Result<Vec<Row>, AppError> {
    let mut statement = conn.prepare(
        "SELECT id, ts, day, app_type, line_id, outcome, reason, value_count, distinct_count,
                kinds_json, value_ids_json
         FROM aicli_privacy_event
         WHERE ts >= ?1 AND (?2 IS NULL OR id < ?2)
         ORDER BY id DESC",
    )?;
    let rows = statement
        .query_map(params![since_ms, before_id], |row| {
            let kinds: String = row.get(9)?;
            let ids: String = row.get(10)?;
            Ok(Row {
                id: row.get(0)?,
                ts: row.get(1)?,
                day: row.get(2)?,
                app_type: row.get(3)?,
                line_id: row.get(4)?,
                outcome: row.get(5)?,
                reason: row.get(6)?,
                value_count: row.get(7)?,
                distinct_count: row.get(8)?,
                kinds: serde_json::from_str(&kinds).unwrap_or_default(),
                ids: serde_json::from_str(&ids).unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn sorted_counts(map: BTreeMap<String, u64>) -> Vec<PrivacyCount> {
    let mut counts: Vec<PrivacyCount> = map
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(key, count)| PrivacyCount { key, count })
        .collect();
    counts.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    counts
}

/// 最近 `days` 天（含今天，本地日期）的汇总。
pub fn summary(conn: &Connection, days: u32, today: NaiveDate) -> Result<PrivacySummary, AppError> {
    let days = clamp_days(days);
    let start = range_start(today, days);
    let rows = read_rows(conn, day_start_ms(start), None)?;

    let mut by_day: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut categories: BTreeMap<String, u64> = ["secret", "personal", "network", "custom"]
        .into_iter()
        .map(|key| (key.to_owned(), 0))
        .collect();
    let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
    let mut apps: BTreeMap<String, u64> = BTreeMap::new();
    let mut ids: HashSet<String> = HashSet::new();
    let (mut total, mut requests, mut blocked, mut degraded, mut media) = (0u64, 0u64, 0, 0, 0);

    for row in rows {
        match row.outcome.as_str() {
            "blocked" => blocked += 1,
            "degraded" => degraded += 1,
            "media_passed" => media += 1,
            _ => {}
        }
        let values = u64::try_from(row.value_count).unwrap_or(0);
        if values == 0 {
            continue;
        }
        total += values;
        requests += 1;
        let entry = by_day.entry(row.day.clone()).or_default();
        entry.0 += values;
        entry.1 += 1;
        for (kind, count) in &row.kinds {
            *kinds.entry(kind.clone()).or_default() += count;
            *categories.entry(category_of(kind).to_owned()).or_default() += count;
        }
        *apps
            .entry(row.app_type.clone().unwrap_or_else(|| "other".into()))
            .or_default() += values;
        ids.extend(row.ids);
    }

    let days_view = (0..days)
        .map(|offset| {
            let day = (start + Duration::days(i64::from(offset)))
                .format("%Y-%m-%d")
                .to_string();
            let (values, requests) = by_day.get(&day).copied().unwrap_or_default();
            PrivacyDay {
                day,
                values,
                requests,
            }
        })
        .collect();

    Ok(PrivacySummary {
        days: days_view,
        total_values: total,
        distinct_values: ids.len() as u64,
        requests,
        blocked,
        degraded,
        media_passed: media,
        categories: ["secret", "personal", "network", "custom"]
            .into_iter()
            .map(|key| PrivacyCount {
                key: key.to_owned(),
                count: categories.get(key).copied().unwrap_or(0),
            })
            .collect(),
        kinds: sorted_counts(kinds),
        apps: sorted_counts(apps),
        range_days: days,
        retention_days: RETENTION_DAYS,
    })
}

/// 记录列表（新 → 旧，按 id 翻页）。`category` 为空时列出全部（含阻断、降级）；
/// 指定类别时只列含该类别的遮盖记录。
pub fn list(
    conn: &Connection,
    days: u32,
    today: NaiveDate,
    category: Option<&str>,
    before_id: Option<i64>,
    limit: u32,
) -> Result<PrivacyEventPage, AppError> {
    let limit = limit.clamp(1, 100) as usize;
    let start = range_start(today, days);
    let rows = read_rows(conn, day_start_ms(start), before_id)?;
    let mut items = Vec::new();
    let mut more = false;
    for row in rows {
        if let Some(category) = category {
            if !row.kinds.keys().any(|kind| category_of(kind) == category) {
                continue;
            }
        }
        if items.len() == limit {
            more = true;
            break;
        }
        items.push(PrivacyEventView {
            id: row.id,
            ts: row.ts,
            app_type: row.app_type,
            line_id: row.line_id,
            outcome: row.outcome,
            reason: row.reason,
            value_count: u64::try_from(row.value_count).unwrap_or(0),
            distinct_count: u64::try_from(row.distinct_count).unwrap_or(0),
            kinds: sorted_counts(row.kinds),
        });
    }
    let next_before_id = if more {
        items.last().map(|item| item.id)
    } else {
        None
    };
    Ok(PrivacyEventPage {
        items,
        next_before_id,
    })
}

/// 清空全部记录（隐私页「清空记录」、解绑）。
pub fn clear(conn: &Connection) -> Result<usize, AppError> {
    Ok(conn.execute("DELETE FROM aicli_privacy_event", [])?)
}

/// 今天（本地日期）。
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

#[cfg(test)]
mod tests;
