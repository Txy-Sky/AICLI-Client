//! W stage: wallet commands and the in-app pay window.
//!
//! Everything goes through existing rc.34 endpoints. Wallet data is read live
//! (never stored in `aicli_cache`); after a redemption, a balance purchase or a
//! settled order the cached `account` snapshot is refreshed so every balance on
//! screen follows.
//!
//! The pay window loads the epay gateway only. It is not listed in any
//! capability, so it has no IPC, and no credential is ever passed to it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_opener::OpenerExt;

use super::{AicliCommandError, AicliServices, AicliState};
use crate::aicli::newapi::{EpayCheckout, NewApiClient, NewApiError, WalletApiError};
use crate::aicli::wallet::{
    self, Navigation, PayOutcome, Plan, Subscription, TopupHistory, TopupInfo, WalletInputError,
};

/// Label of the pay window; `lib.rs` exempts it from the close-to-tray handling.
pub const PAY_WINDOW_LABEL: &str = "aicli-pay";
pub const EVENT_PAY_FINISHED: &str = "aicli-pay-finished";
/// Payment method id the frontend sends to buy a plan from the wallet balance.
const BALANCE_METHOD: &str = "balance";
const HISTORY_PAGE_SIZE: u64 = 10;

impl From<WalletApiError> for AicliCommandError {
    fn from(error: WalletApiError) -> Self {
        match error {
            WalletApiError::Api(inner) => inner.into(),
            WalletApiError::Declined(reason) => Self {
                code: "wallet_declined".into(),
                message: reason,
            },
        }
    }
}

impl From<WalletInputError> for AicliCommandError {
    fn from(error: WalletInputError) -> Self {
        Self {
            code: error.code().into(),
            message: error.to_string(),
        }
    }
}

/// Everything the wallet page shows besides the balance. Each part is fetched
/// independently; a part that failed is named in `failed` and left empty.
#[derive(Debug, Serialize)]
pub struct AicliWalletView {
    pub topup: Option<TopupInfo>,
    pub plans: Vec<Plan>,
    pub subscriptions: Vec<Subscription>,
    pub billing_preference: Option<String>,
    pub aff_code: Option<String>,
    pub failed: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AicliPayStarted {
    /// The order to poll; absent when the gateway form carried no usable number.
    pub trade_no: Option<String>,
    /// Host shown to the user as "paying at"; absent for a balance purchase.
    pub host: Option<String>,
    /// True when nothing is left to wait for (balance purchase).
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize)]
struct PayFinished {
    kind: &'static str,
    trade_no: Option<String>,
    outcome: PayOutcome,
}

impl AicliServices {
    fn wallet_client(&self) -> Result<NewApiClient, AicliCommandError> {
        let (_, line, access) = Self::current_binding(&self.db, self.secrets.as_ref())?
            .ok_or_else(AicliCommandError::not_bound)?;
        Ok(NewApiClient::new(&access, &line)?)
    }

    /// Refresh the cached account snapshot in the background (emits `aicli-data-updated`).
    fn refresh_account_snapshot(&self) {
        if let Ok(cache) = self.live_cache() {
            let _ = cache.get("account");
        }
    }

    pub async fn wallet_get(&self) -> Result<AicliWalletView, AicliCommandError> {
        let client = self.wallet_client()?;
        let (topup, plans, subscriptions, aff) = tokio::join!(
            client.topup_info(),
            client.subscription_plans(),
            client.subscription_self(),
            client.aff_code(),
        );
        let mut failed = Vec::new();
        let mut part = |name: &'static str,
                        result: Result<Value, NewApiError>|
         -> Result<Option<Value>, AicliCommandError> {
            match result {
                Ok(value) => Ok(Some(value)),
                // A dead token is not a partial failure: the whole page must say so.
                Err(NewApiError::TokenInvalid) => Err(NewApiError::TokenInvalid.into()),
                Err(_) => {
                    failed.push(name);
                    Ok(None)
                }
            }
        };
        let topup = part("topup", topup)?;
        let plans = part("plans", plans)?;
        let subscriptions = part("subscriptions", subscriptions)?;
        let aff = part("aff", aff)?;
        let (subscriptions, billing_preference) = subscriptions
            .as_ref()
            .map(wallet::subscriptions)
            .unwrap_or_default();
        Ok(AicliWalletView {
            topup: topup.as_ref().map(TopupInfo::from_response),
            plans: plans.as_ref().map(wallet::plans).unwrap_or_default(),
            subscriptions,
            billing_preference,
            aff_code: aff
                .as_ref()
                .and_then(|aff| aff.get("data"))
                .and_then(Value::as_str)
                .filter(|code| !code.is_empty())
                .map(str::to_owned),
            failed,
        })
    }

    pub async fn wallet_quote(&self, amount: i64) -> Result<String, AicliCommandError> {
        if amount <= 0 {
            return Err(WalletInputError::Amount.into());
        }
        Ok(self.wallet_client()?.topup_amount(amount).await?)
    }

    /// Place a top-up order after re-reading the live payment settings.
    async fn wallet_order(
        &self,
        amount: i64,
        method: &str,
    ) -> Result<EpayCheckout, AicliCommandError> {
        let client = self.wallet_client()?;
        TopupInfo::from_response(&client.topup_info().await?).check_topup(amount, method)?;
        Ok(client.topup_pay(amount, method).await?)
    }

    /// Place a subscription order through epay; `None` means it was paid from the balance.
    async fn subscription_order(
        &self,
        plan_id: i64,
        method: &str,
    ) -> Result<Option<EpayCheckout>, AicliCommandError> {
        if plan_id <= 0 {
            return Err(AicliCommandError::invalid_input("plan_id must be positive"));
        }
        let client = self.wallet_client()?;
        if method == BALANCE_METHOD {
            client.subscription_balance_pay(plan_id).await?;
            self.refresh_account_snapshot();
            return Ok(None);
        }
        TopupInfo::from_response(&client.topup_info().await?).method(method)?;
        Ok(Some(client.subscription_epay_pay(plan_id, method).await?))
    }

    pub async fn wallet_redeem(&self, key: String) -> Result<i64, AicliCommandError> {
        let key = wallet::redeem_key(&key)?;
        let quota = self.wallet_client()?.redeem(key).await?;
        self.refresh_account_snapshot();
        Ok(quota)
    }

    pub async fn wallet_history(&self, page: u64) -> Result<TopupHistory, AicliCommandError> {
        if page == 0 {
            return Err(AicliCommandError::invalid_input("page starts at 1"));
        }
        let response = self
            .wallet_client()?
            .topup_history(page, HISTORY_PAGE_SIZE, None)
            .await?;
        Ok(wallet::history(&response))
    }

    /// The status of one top-up order (`pending`, `success`, ...); `None` while it is not listed.
    pub async fn wallet_order_status(
        &self,
        trade_no: String,
    ) -> Result<Option<String>, AicliCommandError> {
        if !wallet::valid_trade_no(&trade_no) {
            return Err(AicliCommandError::invalid_input("trade_no is malformed"));
        }
        let response = self
            .wallet_client()?
            .topup_history(1, HISTORY_PAGE_SIZE, Some(&trade_no))
            .await?;
        let status = wallet::order_status(&response, &trade_no);
        if status.as_deref() == Some("success") {
            self.refresh_account_snapshot();
        }
        Ok(status)
    }

    pub async fn aff_transfer(&self, quota: i64) -> Result<(), AicliCommandError> {
        if quota <= 0 {
            return Err(WalletInputError::Amount.into());
        }
        self.wallet_client()?.aff_transfer(quota).await?;
        self.refresh_account_snapshot();
        Ok(())
    }
}

/// Refuse a second payment while one window is open, before any order is placed.
fn ensure_no_pay_window(app: &AppHandle) -> Result<(), AicliCommandError> {
    if let Some(window) = app.get_webview_window(PAY_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.set_focus();
        return Err(AicliCommandError {
            code: "pay_window_open".into(),
            message: "A payment window is already open".into(),
        });
    }
    Ok(())
}

fn report_finished(app: &AppHandle, reported: &AtomicBool, finished: PayFinished) {
    if reported.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(error) = app.emit_to("main", EVENT_PAY_FINISHED, finished) {
        log::warn!("failed to emit {EVENT_PAY_FINISHED}: {error}");
    }
}

/// Open the gateway in its own window and report how the payment page ended.
fn open_pay_window(
    app: &AppHandle,
    kind: &'static str,
    checkout: &EpayCheckout,
) -> Result<AicliPayStarted, AicliCommandError> {
    let target = wallet::checkout_target(checkout)?;
    let host = target.host_str().map(str::to_owned);
    let trade_no = wallet::trade_no(checkout).map(str::to_owned);
    let reported = Arc::new(AtomicBool::new(false));

    let navigation_app = app.clone();
    let navigation_reported = reported.clone();
    let navigation_trade_no = trade_no.clone();
    let window = WebviewWindowBuilder::new(app, PAY_WINDOW_LABEL, WebviewUrl::External(target))
        .title(format!(
            "AICLI Client · {}",
            host.as_deref().unwrap_or_default()
        ))
        .inner_size(1000.0, 760.0)
        .min_inner_size(480.0, 560.0)
        .center()
        .incognito(true)
        .on_navigation(move |url| match wallet::classify_navigation(url) {
            Navigation::Allow => true,
            Navigation::Deny => false,
            Navigation::External => {
                if let Err(error) = navigation_app
                    .opener()
                    .open_url(url.as_str(), None::<String>)
                {
                    log::warn!("failed to hand a payment link to the system: {error}");
                }
                false
            }
            Navigation::Finished(outcome) => {
                report_finished(
                    &navigation_app,
                    &navigation_reported,
                    PayFinished {
                        kind,
                        trade_no: navigation_trade_no.clone(),
                        outcome,
                    },
                );
                // Destroy from a task: this callback runs inside the webview's own event.
                let app = navigation_app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(window) = app.get_webview_window(PAY_WINDOW_LABEL) {
                        let _ = window.destroy();
                    }
                });
                false
            }
        })
        .build()
        .map_err(|error| {
            log::warn!("failed to open the payment window: {error}");
            AicliCommandError {
                code: "pay_window".into(),
                message: "The payment window could not be opened".into(),
            }
        })?;

    let closed_app = app.clone();
    let closed_trade_no = trade_no.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            report_finished(
                &closed_app,
                &reported,
                PayFinished {
                    kind,
                    trade_no: closed_trade_no.clone(),
                    outcome: PayOutcome::Closed,
                },
            );
        }
    });

    Ok(AicliPayStarted {
        trade_no,
        host,
        completed: false,
    })
}

#[tauri::command]
pub async fn aicli_wallet_get(
    state: State<'_, AicliState>,
) -> Result<AicliWalletView, AicliCommandError> {
    state.0.wallet_get().await
}

#[tauri::command]
pub async fn aicli_wallet_quote(
    state: State<'_, AicliState>,
    amount: i64,
) -> Result<String, AicliCommandError> {
    state.0.wallet_quote(amount).await
}

#[tauri::command]
pub async fn aicli_wallet_pay(
    app: AppHandle,
    state: State<'_, AicliState>,
    amount: i64,
    method: String,
) -> Result<AicliPayStarted, AicliCommandError> {
    ensure_no_pay_window(&app)?;
    let checkout = state.0.wallet_order(amount, &method).await?;
    open_pay_window(&app, "topup", &checkout)
}

#[tauri::command]
pub async fn aicli_wallet_redeem(
    state: State<'_, AicliState>,
    key: String,
) -> Result<i64, AicliCommandError> {
    state.0.wallet_redeem(key).await
}

#[tauri::command]
pub async fn aicli_wallet_history(
    state: State<'_, AicliState>,
    page: u64,
) -> Result<TopupHistory, AicliCommandError> {
    state.0.wallet_history(page).await
}

#[tauri::command]
pub async fn aicli_wallet_order_status(
    state: State<'_, AicliState>,
    trade_no: String,
) -> Result<Option<String>, AicliCommandError> {
    state.0.wallet_order_status(trade_no).await
}

#[tauri::command]
pub async fn aicli_subscription_buy(
    app: AppHandle,
    state: State<'_, AicliState>,
    plan_id: i64,
    method: String,
) -> Result<AicliPayStarted, AicliCommandError> {
    if method != BALANCE_METHOD {
        ensure_no_pay_window(&app)?;
    }
    match state.0.subscription_order(plan_id, &method).await? {
        None => Ok(AicliPayStarted {
            trade_no: None,
            host: None,
            completed: true,
        }),
        Some(checkout) => open_pay_window(&app, "subscription", &checkout),
    }
}

#[tauri::command]
pub async fn aicli_aff_transfer(
    state: State<'_, AicliState>,
    quota: i64,
) -> Result<(), AicliCommandError> {
    state.0.aff_transfer(quota).await
}
