//! Wallet (W stage): typed views over rc.34 payment responses, input checks and
//! the navigation policy of the in-app pay window. Pure code, no I/O.

use std::collections::BTreeMap;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::LINES;
use super::newapi::EpayCheckout;

/// Gateways the client does not drive; only epay methods are offered in the app.
const OTHER_GATEWAYS: [&str; 4] = ["stripe", "creem", "waffo", "waffo_pancake"];
/// URL schemes a checkout page uses to hand over to a payment app.
const PAYMENT_APP_SCHEMES: [&str; 4] = ["alipays", "alipay", "weixin", "wechat"];
/// The one account-site path the pay window may load: it settles a subscription order.
const SUBSCRIPTION_RETURN_PATH: &str = "/api/subscription/epay/return";

const REDEEM_KEY_MAX_CHARS: usize = 128;
const CHECKOUT_MAX_FIELDS: usize = 32;
const CHECKOUT_MAX_KEY_CHARS: usize = 64;
const CHECKOUT_MAX_VALUE_CHARS: usize = 2048;
const TRADE_NO_MAX_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WalletInputError {
    #[error("Online top-up is not enabled")]
    Disabled,
    #[error("The top-up amount is invalid")]
    Amount,
    #[error("The top-up amount is below the minimum of {0}")]
    BelowMinimum(i64),
    #[error("The payment method is not available")]
    Method,
    #[error("The redemption code is invalid")]
    RedeemKey,
    #[error("The payment page address is not acceptable")]
    Checkout,
}

impl WalletInputError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Disabled => "wallet_disabled",
            Self::Amount => "wallet_amount",
            Self::BelowMinimum(_) => "wallet_below_minimum",
            Self::Method => "wallet_method",
            Self::RedeemKey => "wallet_redeem_key",
            Self::Checkout => "wallet_checkout",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PayMethod {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub color: Option<String>,
    pub min_topup: Option<i64>,
}

/// `/api/user/topup/info`, reduced to what the wallet page needs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TopupInfo {
    /// Epay is switched on and at least one epay method is configured.
    pub online_topup: bool,
    pub redemption: bool,
    pub methods: Vec<PayMethod>,
    pub min_topup: i64,
    pub amount_options: Vec<i64>,
    /// Preset amount → price multiplier (0.9 means 10% off).
    pub discounts: BTreeMap<i64, f64>,
    /// The operator's external purchase link, when it is an https URL.
    pub topup_link: Option<String>,
}

fn integer(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

impl TopupInfo {
    pub fn from_response(response: &Value) -> Self {
        let data = response.get("data");
        let field = |name: &str| data.and_then(|data| data.get(name));
        let flag = |name: &str| field(name).and_then(Value::as_bool).unwrap_or(false);
        let methods: Vec<PayMethod> = field("pay_methods")
            .and_then(Value::as_array)
            .map(|methods| {
                methods
                    .iter()
                    .filter_map(|method| {
                        let kind = text(method.get("type"))?;
                        if OTHER_GATEWAYS.contains(&kind.as_str()) {
                            return None;
                        }
                        Some(PayMethod {
                            name: text(method.get("name")).unwrap_or_else(|| kind.clone()),
                            kind,
                            color: text(method.get("color")),
                            min_topup: integer(method.get("min_topup")).filter(|min| *min > 0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let amount_options = field("amount_options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| integer(Some(option)))
                    .filter(|option| *option > 0)
                    .collect()
            })
            .unwrap_or_default();
        let discounts = field("discount")
            .and_then(Value::as_object)
            .map(|discounts| {
                discounts
                    .iter()
                    .filter_map(|(amount, ratio)| {
                        let ratio = ratio.as_f64().filter(|ratio| *ratio > 0.0)?;
                        Some((amount.parse().ok()?, ratio))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            online_topup: flag("enable_online_topup") && !methods.is_empty(),
            redemption: flag("enable_redemption"),
            methods,
            min_topup: integer(field("min_topup")).unwrap_or(1).max(1),
            amount_options,
            discounts,
            topup_link: text(field("topup_link")).filter(|link| {
                Url::parse(link).is_ok_and(|url| url.scheme() == "https" && url.has_host())
            }),
        }
    }

    pub fn method(&self, kind: &str) -> Result<&PayMethod, WalletInputError> {
        self.methods
            .iter()
            .find(|method| method.kind == kind)
            .ok_or(WalletInputError::Method)
    }

    /// The pre-payment check; the server repeats it and stays authoritative.
    pub fn check_topup(&self, amount: i64, kind: &str) -> Result<(), WalletInputError> {
        if !self.online_topup {
            return Err(WalletInputError::Disabled);
        }
        let method = self.method(kind)?;
        if amount <= 0 {
            return Err(WalletInputError::Amount);
        }
        let minimum = self.min_topup.max(method.min_topup.unwrap_or(0));
        if amount < minimum {
            return Err(WalletInputError::BelowMinimum(minimum));
        }
        Ok(())
    }
}

/// A subscription plan as `/api/subscription/plans` lists it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Plan {
    pub id: i64,
    pub title: String,
    pub subtitle: String,
    pub price_amount: f64,
    pub currency: String,
    pub duration_unit: String,
    pub duration_value: i64,
    pub custom_seconds: i64,
    pub allow_balance_pay: Option<bool>,
    pub max_purchase_per_user: i64,
    pub upgrade_group: String,
    /// Raw quota units; 0 means unlimited.
    pub total_amount: i64,
    pub quota_reset_period: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Subscription {
    pub id: i64,
    pub plan_id: i64,
    pub amount_total: i64,
    pub amount_used: i64,
    pub start_time: i64,
    pub end_time: i64,
    pub status: String,
    pub next_reset_time: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TopupOrder {
    pub id: i64,
    pub amount: i64,
    pub money: f64,
    pub trade_no: String,
    pub payment_method: String,
    pub create_time: i64,
    pub complete_time: i64,
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TopupHistory {
    pub items: Vec<TopupOrder>,
    pub total: u64,
}

fn wrapped<T: for<'de> Deserialize<'de>>(items: Option<&Value>, key: &str) -> Vec<T> {
    items
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.get(key)?.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Enabled plans; an empty list means the operator has not set subscriptions up.
pub fn plans(response: &Value) -> Vec<Plan> {
    let mut plans: Vec<Plan> = wrapped(response.get("data"), "plan");
    plans.retain(|plan| plan.id > 0);
    plans
}

/// Every subscription of the user (expired ones included) and the billing preference.
pub fn subscriptions(response: &Value) -> (Vec<Subscription>, Option<String>) {
    let data = response.get("data");
    let field = |name: &str| data.and_then(|data| data.get(name));
    let all = field("all_subscriptions")
        .filter(|all| all.is_array())
        .or_else(|| field("subscriptions"));
    let mut subscriptions: Vec<Subscription> = wrapped(all, "subscription");
    subscriptions.retain(|subscription| subscription.id > 0);
    (subscriptions, text(field("billing_preference")))
}

pub fn history(response: &Value) -> TopupHistory {
    let data = response.get("data");
    let items: Vec<TopupOrder> = data
        .and_then(|data| data.get("items"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let total = data
        .and_then(|data| data.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(items.len() as u64);
    TopupHistory { items, total }
}

/// The status of exactly this order in a history page, if it is listed.
pub fn order_status(response: &Value, trade_no: &str) -> Option<String> {
    history(response)
        .items
        .into_iter()
        .find(|order| order.trade_no == trade_no)
        .map(|order| order.status)
        .filter(|status| !status.is_empty())
}

pub fn redeem_key(input: &str) -> Result<&str, WalletInputError> {
    let key = input.trim();
    if key.is_empty()
        || key.chars().count() > REDEEM_KEY_MAX_CHARS
        || key
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(WalletInputError::RedeemKey);
    }
    Ok(key)
}

pub fn valid_trade_no(trade_no: &str) -> bool {
    !trade_no.is_empty()
        && trade_no.len() <= TRADE_NO_MAX_CHARS
        && trade_no
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// The order number new-api put into the signed form.
pub fn trade_no(checkout: &EpayCheckout) -> Option<&str> {
    checkout
        .params
        .get("out_trade_no")
        .map(String::as_str)
        .filter(|trade_no| valid_trade_no(trade_no))
}

fn line_host(host: &str) -> bool {
    LINES.iter().any(|line| {
        Url::parse(line.base_url)
            .ok()
            .and_then(|url| url.host_str().map(|line_host| line_host == host))
            .unwrap_or(false)
    })
}

/// The address the pay window opens: the gateway URL with the signed fields as
/// its query. Only an https gateway that is not one of our own lines is accepted.
pub fn checkout_target(checkout: &EpayCheckout) -> Result<Url, WalletInputError> {
    let mut url = Url::parse(&checkout.url).map_err(|_| WalletInputError::Checkout)?;
    let host = url.host_str().ok_or(WalletInputError::Checkout)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || line_host(host)
        || checkout.params.is_empty()
        || checkout.params.len() > CHECKOUT_MAX_FIELDS
        || checkout.params.iter().any(|(key, value)| {
            key.is_empty()
                || key.chars().count() > CHECKOUT_MAX_KEY_CHARS
                || value.chars().count() > CHECKOUT_MAX_VALUE_CHARS
        })
    {
        return Err(WalletInputError::Checkout);
    }
    url.set_fragment(None);
    url.query_pairs_mut().extend_pairs(&checkout.params);
    Ok(url)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PayOutcome {
    Success,
    Fail,
    /// The gateway sent the browser back without saying how it went.
    Unknown,
    /// The user closed the window.
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    Allow,
    /// Hand the URL to the operating system (a payment app).
    External,
    Deny,
    /// The gateway returned to the account site: close the window.
    Finished(PayOutcome),
}

/// What the pay window does with a navigation. The window never renders the
/// account site (it has no session there), so a return to one of our lines ends
/// the payment; the only exception settles a subscription order and redirects.
pub fn classify_navigation(url: &Url) -> Navigation {
    match url.scheme() {
        "https" | "http" => {
            if !url.host_str().is_some_and(line_host) {
                return Navigation::Allow;
            }
            if url.scheme() == "https" && url.path() == SUBSCRIPTION_RETURN_PATH {
                return Navigation::Allow;
            }
            let mut outcome = PayOutcome::Unknown;
            for (key, value) in url.query_pairs() {
                match (key.as_ref(), value.as_ref()) {
                    ("pay", "success") | ("trade_status", "TRADE_SUCCESS") => {
                        outcome = PayOutcome::Success;
                    }
                    ("pay", "fail") => return Navigation::Finished(PayOutcome::Fail),
                    _ => {}
                }
            }
            Navigation::Finished(outcome)
        }
        "about" if url.as_str() == "about:blank" => Navigation::Allow,
        scheme if PAYMENT_APP_SCHEMES.contains(&scheme) => Navigation::External,
        _ => Navigation::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> TopupInfo {
        TopupInfo::from_response(&json!({"success": true, "data": {
            "enable_online_topup": true,
            "enable_redemption": true,
            "pay_methods": [
                {"name": "支付宝", "type": "alipay", "color": "#1677FF"},
                {"name": "微信", "type": "wxpay", "min_topup": "50"},
                {"name": "Stripe", "type": "stripe", "min_topup": "1"},
                {"name": "no type"}
            ],
            "min_topup": 10,
            "amount_options": [10, 50, "100", 0, -5],
            "discount": {"100": 0.9, "50": 0, "x": 0.5},
            "topup_link": "http://insecure.example.test"
        }}))
    }

    #[test]
    fn topup_info_keeps_epay_methods_and_valid_options_only() {
        let info = info();
        assert!(info.online_topup && info.redemption);
        assert_eq!(
            info.methods
                .iter()
                .map(|m| m.kind.as_str())
                .collect::<Vec<_>>(),
            ["alipay", "wxpay"]
        );
        assert_eq!(info.methods[1].min_topup, Some(50));
        assert_eq!(info.amount_options, [10, 50, 100]);
        assert_eq!(info.discounts, BTreeMap::from([(100, 0.9)]));
        assert_eq!(info.topup_link, None);
        assert_eq!(
            serde_json::to_value(&info.methods[0]).unwrap()["type"],
            "alipay"
        );

        // Only other gateways configured: nothing to drive from the client.
        let other = TopupInfo::from_response(&json!({"data": {
            "enable_online_topup": true,
            "pay_methods": [{"name": "Stripe", "type": "stripe"}],
            "topup_link": "https://shop.example.test/buy"
        }}));
        assert!(!other.online_topup && !other.redemption);
        assert_eq!(other.min_topup, 1);
        assert_eq!(
            other.topup_link.as_deref(),
            Some("https://shop.example.test/buy")
        );
        assert!(!TopupInfo::from_response(&json!({})).online_topup);
    }

    #[test]
    fn topup_check_applies_method_and_minimums() {
        let info = info();
        assert_eq!(info.check_topup(10, "alipay"), Ok(()));
        assert_eq!(
            info.check_topup(9, "alipay"),
            Err(WalletInputError::BelowMinimum(10))
        );
        assert_eq!(
            info.check_topup(49, "wxpay"),
            Err(WalletInputError::BelowMinimum(50))
        );
        assert_eq!(info.check_topup(0, "alipay"), Err(WalletInputError::Amount));
        assert_eq!(
            info.check_topup(10, "stripe"),
            Err(WalletInputError::Method)
        );
        let mut off = info;
        off.online_topup = false;
        assert_eq!(
            off.check_topup(10, "alipay"),
            Err(WalletInputError::Disabled)
        );
        assert_eq!(
            WalletInputError::BelowMinimum(1).code(),
            "wallet_below_minimum"
        );
    }

    #[test]
    fn plans_subscriptions_and_history_tolerate_extra_and_broken_rows() {
        let plans = plans(&json!({"data": [
            {"plan": {"id": 3, "title": "Pro", "price_amount": 9.9, "allow_balance_pay": null,
                      "stripe_price_id": "ignored", "total_amount": 5000000}},
            {"plan": {"id": 0, "title": "broken"}},
            {"plan": "broken"},
            {}
        ]}));
        assert_eq!(plans.len(), 1);
        assert_eq!((plans[0].id, plans[0].allow_balance_pay), (3, None));
        assert!(super::plans(&json!({"data": []})).is_empty());

        let (all, preference) = subscriptions(&json!({"data": {
            "billing_preference": "subscription_first",
            "subscriptions": [{"subscription": {"id": 1, "plan_id": 3, "status": "active"}}],
            "all_subscriptions": [
                {"subscription": {"id": 1, "plan_id": 3, "status": "active"}},
                {"subscription": {"id": 2, "plan_id": 3, "status": "expired"}},
                {"subscription": null}
            ]
        }}));
        assert_eq!(all.len(), 2);
        assert_eq!(preference.as_deref(), Some("subscription_first"));
        let (active_only, _) = subscriptions(&json!({"data": {
            "subscriptions": [{"subscription": {"id": 1}}]
        }}));
        assert_eq!(active_only.len(), 1);

        let page = json!({"data": {"total": 7, "items": [
            {"id": 1, "trade_no": "USR7NOabc", "status": "success", "money": 7.3, "amount": 10},
            {"id": 2, "trade_no": "USR7NOabcd", "status": "pending"},
            "broken"
        ]}});
        let parsed = history(&page);
        assert_eq!((parsed.items.len(), parsed.total), (2, 7));
        assert_eq!(order_status(&page, "USR7NOabc").as_deref(), Some("success"));
        assert_eq!(
            order_status(&page, "USR7NOabcd").as_deref(),
            Some("pending")
        );
        assert_eq!(order_status(&page, "USR7NOab"), None);
    }

    #[test]
    fn redeem_key_and_trade_number_shapes() {
        assert_eq!(redeem_key("  abc-123  "), Ok("abc-123"));
        for bad in ["", "   ", "a b", "a\nb", &"x".repeat(129)] {
            assert_eq!(redeem_key(bad), Err(WalletInputError::RedeemKey));
        }
        assert!(valid_trade_no("SUBUSR7NOabc_1-2"));
        for bad in ["", "a b", "a/b", "a%", &"x".repeat(65)] {
            assert!(!valid_trade_no(bad));
        }
    }

    fn checkout(url: &str) -> EpayCheckout {
        EpayCheckout {
            url: url.into(),
            params: BTreeMap::from([
                ("out_trade_no".to_owned(), "USR7NOabc".to_owned()),
                ("name".to_owned(), "TUC10 & more".to_owned()),
            ]),
        }
    }

    #[test]
    fn checkout_target_requires_an_external_https_gateway() {
        let order = checkout("https://pay.example.test/submit.php#frag");
        assert_eq!(trade_no(&order), Some("USR7NOabc"));
        assert_eq!(
            checkout_target(&order).unwrap().as_str(),
            "https://pay.example.test/submit.php?name=TUC10+%26+more&out_trade_no=USR7NOabc"
        );
        for bad in [
            "http://pay.example.test/submit.php",
            "https://user:secret@pay.example.test/submit.php",
            "https://aicliapi.com/submit.php",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert_eq!(
                checkout_target(&checkout(bad)),
                Err(WalletInputError::Checkout)
            );
        }
        let mut empty = checkout("https://pay.example.test/submit.php");
        empty.params.clear();
        assert_eq!(checkout_target(&empty), Err(WalletInputError::Checkout));
        assert_eq!(trade_no(&empty), None);
        let mut long = checkout("https://pay.example.test/submit.php");
        long.params.insert("sign".into(), "x".repeat(2049));
        assert_eq!(checkout_target(&long), Err(WalletInputError::Checkout));
    }

    #[test]
    fn navigation_policy() {
        let classify = |url: &str| classify_navigation(&Url::parse(url).unwrap());
        assert_eq!(
            classify("https://pay.example.test/submit.php?a=1"),
            Navigation::Allow
        );
        assert_eq!(
            classify("https://openapi.alipay.com/gateway.do"),
            Navigation::Allow
        );
        assert_eq!(classify("about:blank"), Navigation::Allow);
        assert_eq!(
            classify("alipays://platformapi/startapp?x=1"),
            Navigation::External
        );
        assert_eq!(
            classify("weixin://wxpay/bizpayurl?pr=x"),
            Navigation::External
        );
        for denied in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "about:config",
            "tauri://localhost",
        ] {
            assert_eq!(classify(denied), Navigation::Deny);
        }
        // A look-alike host is just another external page, not a return.
        assert_eq!(
            classify("https://aicliapi.com.example.test/wallet"),
            Navigation::Allow
        );
        assert_eq!(
            classify("https://aicliapi.com/api/subscription/epay/return?trade_no=x"),
            Navigation::Allow
        );
        assert_eq!(
            classify("https://aicliapi.com/wallet?pay=success"),
            Navigation::Finished(PayOutcome::Success)
        );
        assert_eq!(
            classify("https://aicli.qzz.io/wallet?pay=fail&trade_status=TRADE_SUCCESS"),
            Navigation::Finished(PayOutcome::Fail)
        );
        assert_eq!(
            classify(
                "https://api.aicli.qzz.io/usage-logs?trade_status=TRADE_SUCCESS&out_trade_no=x"
            ),
            Navigation::Finished(PayOutcome::Success)
        );
        assert_eq!(
            classify("https://aicliapi.com/usage-logs"),
            Navigation::Finished(PayOutcome::Unknown)
        );
        assert_eq!(
            classify("http://aicliapi.com/api/subscription/epay/return"),
            Navigation::Finished(PayOutcome::Unknown)
        );
        assert_eq!(serde_json::to_value(PayOutcome::Closed).unwrap(), "closed");
    }
}
