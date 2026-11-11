//! 系统托盘（G-04，负责人 2026-11-10 选定方案 A）。
//!
//! 菜单只有：打开主窗口、当前线路（子菜单可切换，未绑定时禁用）、轻量模式开关、
//! 打开官方网站（`branding::WEBSITE_URL`）、退出。供应商切换菜单、额度托盘面板
//! （`tray_panel` + `src/tray-panel/`）、用量标题刷新与问题圆点已随 G-04 移除；
//! 状态以主界面为准。
//!
//! Windows：左键打开主界面、右键出菜单（`show_menu_on_left_click(false)` + 图标事件）。
//! macOS：左右键都出原生菜单，模板图标跟随菜单栏深浅色。Linux（AppIndicator）同样原生菜单。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use tauri::menu::{CheckMenuItem, Menu, MenuBuilder, MenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::error::AppError;
use crate::store::AppState;

/// B-03：内部名不改（与旧版本升级共存时靠它找回已注册的托盘图标）。
pub const TRAY_ID: &str = "cc-switch";

// ─── 文案 ─────────────────────────────────────────────────────────────────────

struct TrayTexts {
    show_main: &'static str,
    lines_label: &'static str,
    line_main: &'static str,
    line_global: &'static str,
    line_apac: &'static str,
    not_bound: &'static str,
    lightweight_mode: &'static str,
    open_website: &'static str,
    quit: &'static str,
    /// 有应用在用路由服务时的「退出」：写明后果（客户端指回直连、路由服务停止）。
    quit_stops_routing: &'static str,
}

impl TrayTexts {
    fn from_language(language: &str) -> Self {
        match language {
            "zh" | "zh-CN" | "zh-SG" | "zh-Hans" | "zh-Hans-CN" => Self {
                show_main: "打开 AICLI Client",
                lines_label: "线路",
                line_main: "主线路",
                line_global: "全球备用",
                line_apac: "亚太加速",
                not_bound: "未绑定",
                lightweight_mode: "轻量模式",
                open_website: "打开官方网站",
                quit: "退出 AICLI Client",
                quit_stops_routing: "退出 AICLI Client（停止路由服务）",
            },
            "zh-TW" => Self {
                show_main: "開啟 AICLI Client",
                lines_label: "線路",
                line_main: "主線路",
                line_global: "全球備用",
                line_apac: "亞太加速",
                not_bound: "未綁定",
                lightweight_mode: "輕量模式",
                open_website: "開啟官方網站",
                quit: "結束 AICLI Client",
                quit_stops_routing: "結束 AICLI Client（停止路由服務）",
            },
            "ja" => Self {
                show_main: "AICLI Client を開く",
                lines_label: "回線",
                line_main: "メイン回線",
                line_global: "グローバルバックアップ",
                line_apac: "アジア太平洋",
                not_bound: "未バインド",
                lightweight_mode: "ライトモード",
                open_website: "公式サイトを開く",
                quit: "AICLI Client を終了",
                quit_stops_routing: "AICLI Client を終了（ルーティングサービスを停止）",
            },
            // 土耳其语是社区翻译：没有人翻译前显示英文。
            "tr" => Self {
                show_main: "AICLI Client uygulamasını aç",
                lines_label: "Hat",
                line_main: "Ana hat",
                line_global: "Yedek hat",
                line_apac: "Asya-Pasifik",
                not_bound: "Bağlı değil",
                lightweight_mode: "Hafif mod",
                open_website: "Resmî web sitesini aç",
                quit: "AICLI Client uygulamasından çık",
                quit_stops_routing: "AICLI Client uygulamasından çık (yönlendirme hizmeti durur)",
            },
            _ => Self {
                show_main: "Open AICLI Client",
                lines_label: "Line",
                line_main: "Main",
                line_global: "Global backup",
                line_apac: "Asia-Pacific",
                not_bound: "Not bound",
                lightweight_mode: "Lightweight mode",
                open_website: "Open official website",
                quit: "Quit AICLI Client",
                quit_stops_routing: "Quit AICLI Client (stops the routing service)",
            },
        }
    }

    fn current() -> Self {
        Self::from_language(&tray_language())
    }
}

/// 托盘用的语言：设置里的语言，没设过时按系统区域。
pub(crate) fn tray_language() -> String {
    fn detect() -> String {
        let locale = sys_locale::get_locale().unwrap_or_else(|| "en".into());
        let locale = locale.to_lowercase();
        // Windows 常见 `zh-hans-cn` 这类 BCP 47；归一到托盘文案支持的语言。
        if locale.starts_with("zh-hant") || locale.starts_with("zh-tw") {
            "zh-TW".into()
        } else if locale.starts_with("zh") {
            "zh".into()
        } else if locale.starts_with("ja") {
            "ja".into()
        } else if locale.starts_with("tr") {
            "tr".into()
        } else {
            "en".into()
        }
    }
    crate::settings::get_settings()
        .language
        .unwrap_or_else(detect)
}

// ─── 菜单 ─────────────────────────────────────────────────────────────────────

/// 当前选中的 AICLI 线路；未绑定（`aicli_account` 无有效行）时返回 `None`。
fn selected_line(app_state: &AppState) -> Option<String> {
    use rusqlite::OptionalExtension;
    let conn = app_state.db.conn.lock().ok()?;
    conn.query_row(
        "SELECT selected_line_id FROM aicli_account WHERE id=1 AND user_id IS NOT NULL",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// 有应用在用路由服务时的「退出」要写明后果（同旧实现，G-04 保留）。
fn routing_in_use(app_state: &AppState) -> bool {
    app_state.proxy_service.running_now() == Some(true)
        && crate::mode::current::proxy_flags(crate::mode::controller::PROXY_APPS).contains(&true)
}

pub fn create_tray_menu(
    app: &tauri::AppHandle,
    app_state: &AppState,
) -> Result<Menu<tauri::Wry>, AppError> {
    let texts = TrayTexts::current();

    let show_main = MenuItem::with_id(app, "show_main", texts.show_main, true, None::<&str>)
        .map_err(menu_error)?;
    let open_website =
        MenuItem::with_id(app, "open_website", texts.open_website, true, None::<&str>)
            .map_err(menu_error)?;
    let lightweight = CheckMenuItem::with_id(
        app,
        "lightweight_mode",
        texts.lightweight_mode,
        true,
        crate::lightweight::is_lightweight_mode(),
        None::<&str>,
    )
    .map_err(menu_error)?;
    let quit_label = if routing_in_use(app_state) {
        texts.quit_stops_routing
    } else {
        texts.quit
    };
    let quit =
        MenuItem::with_id(app, "quit", quit_label, true, None::<&str>).map_err(menu_error)?;

    let menu = MenuBuilder::new(app)
        .item(&show_main)
        .separator()
        .item(&line_menu(app, &texts, app_state)?)
        .separator()
        .item(&lightweight)
        .separator()
        .item(&open_website)
        .item(&quit)
        .build()
        .map_err(menu_error)?;

    #[cfg(target_os = "windows")]
    crate::tray_compat::remember_native_menu(&menu);
    Ok(menu)
}

fn line_items(
    app: &tauri::AppHandle,
    texts: &TrayTexts,
    app_state: &AppState,
) -> Result<Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>>, AppError> {
    let selected = selected_line(app_state);
    let bound = selected.is_some();
    let current = selected.unwrap_or_else(|| "main".to_string());
    let mut items = Vec::new();
    for (id, name) in [
        ("main", texts.line_main),
        ("global", texts.line_global),
        ("apac", texts.line_apac),
    ] {
        let item = CheckMenuItem::with_id(
            app,
            format!("line:{id}"),
            name,
            bound,
            bound && current == id,
            None::<&str>,
        )
        .map_err(menu_error)?;
        items.push(Box::new(item) as Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>);
    }
    Ok(items)
}

/// 「线路」子菜单：三条线路单选（当前项打勾）；未绑定时整个子菜单禁用并标注。
fn line_menu(
    app: &tauri::AppHandle,
    texts: &TrayTexts,
    app_state: &AppState,
) -> Result<Submenu<tauri::Wry>, AppError> {
    let selected = selected_line(app_state);
    let bound = selected.is_some();
    let label = if bound {
        texts.lines_label.to_string()
    } else {
        format!("{} · {}", texts.lines_label, texts.not_bound)
    };
    let items = line_items(app, texts, app_state)?;
    let refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
        items.iter().map(|i| i.as_ref()).collect();
    Submenu::with_id_and_items(app, "lines", &label, bound, &refs).map_err(menu_error)
}

fn menu_error(e: impl std::fmt::Display) -> AppError {
    AppError::Database(format!("托盘菜单构建失败: {e}"))
}

// ─── 事件 ─────────────────────────────────────────────────────────────────────

/// 处理托盘菜单事件。
pub fn handle_tray_menu_event(app: &tauri::AppHandle, event_id: &str) {
    log::info!("处理托盘菜单事件: {event_id}");

    match event_id {
        "show_main" => show_main_window(app),
        "open_website" => {
            if let Err(e) = app
                .opener()
                .open_url(crate::branding::WEBSITE_URL, None::<String>)
            {
                log::error!("打开官方网站失败: {e}");
            }
        }
        "lightweight_mode" => {
            if crate::lightweight::is_lightweight_mode() {
                if let Err(e) = crate::lightweight::exit_lightweight_mode(app) {
                    log::error!("退出轻量模式失败: {e}");
                }
            } else if let Err(e) = crate::lightweight::enter_lightweight_mode(app) {
                log::error!("进入轻量模式失败: {e}");
            }
        }
        "quit" => {
            log::info!("退出应用");
            app.exit(0);
        }
        other => {
            if let Some(line_id) = other.strip_prefix("line:") {
                switch_line(app, line_id.to_string());
                return;
            }
            log::warn!("未处理的菜单事件: {event_id}");
        }
    }
}

/// 托盘里切换 AICLI 线路：复用 `aicli_select_line` 的服务实现，成功后广播事件
/// 让主界面刷新线路相关的缓存。
fn switch_line(app: &tauri::AppHandle, line_id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<crate::commands::AicliState>() else {
            log::warn!("AICLI 服务尚未初始化，无法切换线路");
            return;
        };
        match state.0.select_line(line_id) {
            Ok(()) => {
                if let Err(e) = app.emit("aicli-line-changed", ()) {
                    log::warn!("广播线路变更失败: {e}");
                }
                refresh_tray_menu(&app);
            }
            Err(e) => log::error!("托盘切换线路失败: {:?}", e),
        }
    });
}

/// 托盘图标的鼠标事件。Windows 第三方托盘下 `tray_compat` 补发的点击也走这里。
pub fn handle_tray_icon_event(tray: &TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = &event
    {
        if cfg!(target_os = "windows") {
            show_main_window(tray.app_handle());
        }
        // macOS / Linux：菜单由系统在点击时弹出（show_menu_on_left_click(true)）。
    }
}

/// 左键是否直接出原生菜单。Windows 的习惯是左键打开主界面、右键出菜单。
pub fn native_menu_on_left_click(native: bool) -> bool {
    let _ = native; // G-04 起只有原生菜单
    cfg!(not(target_os = "windows"))
}

pub fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        {
            let _ = window.set_skip_taskbar(false);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        #[cfg(target_os = "linux")]
        {
            crate::linux_fix::nudge_main_window(window.clone(), "tray-show-main");
        }
        #[cfg(target_os = "macos")]
        {
            apply_tray_policy(app, true);
        }
    } else if crate::lightweight::is_lightweight_mode() {
        if let Err(e) = crate::lightweight::exit_lightweight_mode(app) {
            log::error!("退出轻量模式重建窗口失败: {e}");
        }
    }
}

// ─── 重建 ─────────────────────────────────────────────────────────────────────

pub fn refresh_tray_menu(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match create_tray_menu(app, state.inner()) {
        Ok(menu) => {
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                if let Err(e) = tray.set_menu(Some(menu)) {
                    log::error!("更新托盘菜单失败: {e}");
                }
            }
        }
        Err(e) => log::error!("重建托盘菜单失败: {e}"),
    }
}

/// 合并多次快速触发的重建（批量刷新期间多个命令同时成功时只重建一次）。
static TRAY_REBUILD_SCHEDULED: AtomicBool = AtomicBool::new(false);

pub fn schedule_tray_refresh(app: &tauri::AppHandle) {
    if TRAY_REBUILD_SCHEDULED.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        TRAY_REBUILD_SCHEDULED.store(false, Ordering::Release);
        refresh_tray_menu(&app);
    });
}

/// 路由服务启动 / 停止之后「退出」的后果可能要跟着变。保留旧入口名，
/// 行为等同一次防抖重建。
pub fn schedule_tray_status_check(app: &tauri::AppHandle) {
    schedule_tray_refresh(app);
}

/// 旧实现等启动流程走完才开始报「路由服务没在运行」；G-04 的菜单不再报问题，
/// 保留空实现避免改动启动调用方。
pub fn mark_startup_settled(_app: &tauri::AppHandle) {}

// ─── 图标与平台策略 ───────────────────────────────────────────────────────────

/// 托盘平时的图标和它是不是 macOS 模板图：macOS 用单色模板图（跟随菜单栏深浅色），
/// 读不到时和其它平台一样用应用图标。读一次缓存起来。
pub fn base_tray_icon(app: &tauri::AppHandle) -> Option<(tauri::image::Image<'static>, bool)> {
    static BASE: OnceLock<Option<(tauri::image::Image<'static>, bool)>> = OnceLock::new();
    BASE.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            const ICON_BYTES: &[u8] =
                include_bytes!("../icons/tray/macos/statusbar_template_3x.png");
            match tauri::image::Image::from_bytes(ICON_BYTES) {
                Ok(icon) => return Some((icon, true)),
                Err(err) => {
                    log::warn!("Failed to load macOS tray icon: {err}");
                    log::warn!("Falling back to default window icon for tray");
                }
            }
        }
        match app.default_window_icon() {
            Some(icon) => Some((icon.clone().to_owned(), false)),
            None => {
                log::warn!("Failed to get default window icon for tray");
                None
            }
        }
    })
    .clone()
}

/// macOS Dock 显示策略：轻量模式藏 Dock，恢复时再显示。其它平台无操作
///（调用方都在 `#[cfg(target_os = "macos")]` 块里）。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn apply_tray_policy(app: &tauri::AppHandle, dock_visible: bool) {
    #[cfg(target_os = "macos")]
    {
        let desired_policy = if dock_visible {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        let _ = app.set_dock_visibility(dock_visible);
        if let Err(err) = app.set_activation_policy(desired_policy) {
            log::warn!("设置激活策略失败: {err}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, dock_visible);
    }
}
