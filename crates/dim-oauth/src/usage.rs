//! Projects `GET {issuer}/api/me/usage` into the account usage DTO. The
//! service wraps its report as `{success, data}`; a bare report is read as it
//! is. Absent server fields stay absent: no number is coerced to 0, and no raw
//! body or credential is retained.
use crate::http::Body;
use dim_app_server::managed_provider::{ManagedError, management::Usage};
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

type Object = Map<String, Value>;

fn object(value: Option<&Value>) -> &Object {
    static EMPTY: LazyLock<Object> = LazyLock::new(Map::new);
    value.and_then(Value::as_object).unwrap_or(&EMPTY)
}
fn label(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| json!(s))
}
fn number(value: Option<&Value>) -> Option<Value> {
    value.filter(|v| v.is_number()).cloned()
}
fn flag(value: Option<&Value>) -> Option<Value> {
    value.filter(|v| v.is_boolean()).cloned()
}
fn defined<const N: usize>(fields: [(&str, Option<Value>); N]) -> Object {
    fields
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_owned(), value?)))
        .collect()
}

pub(super) fn project_usage(envelope: &Body) -> Result<Usage, ManagedError> {
    let body = match envelope.get("data") {
        Some(data) => data.as_object().ok_or(ManagedError::InvalidResponse)?,
        None => envelope,
    };
    let subscription = object(body.get("subscription"));
    let product = object(subscription.get("product"));
    let price = object(subscription.get("price"));
    let plan = defined([
        ("name", label(product.get("name"))),
        (
            "status",
            label(object(subscription.get("subscription")).get("status"))
                .or(label(product.get("status"))),
        ),
        ("priceAmount", number(price.get("amount"))),
        ("priceCurrency", label(price.get("currency"))),
        ("priceInterval", label(price.get("billing_interval"))),
        (
            "expiresAt",
            label(object(subscription.get("current_term")).get("end_at")),
        ),
    ]);
    let bucket = object(object(body.get("credits")).get("subscription_bucket"));
    let credits = defined([
        ("usedUnits", number(bucket.get("used_units"))),
        ("totalUnits", number(bucket.get("total_units"))),
        ("remainingUnits", number(bucket.get("remaining_units"))),
        ("unlimited", flag(bucket.get("unlimited"))),
        (
            "resetsAt",
            label(object(object(body.get("resets")).get("monthly_full")).get("nearest_expires_at"))
                .or(label(bucket.get("expires_at"))),
        ),
    ]);
    let windows: Vec<Value> = bucket
        .get("window_states")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let window = object(Some(entry));
            Some(Value::Object(defined([
                (
                    "durationHours",
                    Some(number(window.get("window_duration_hours"))?),
                ),
                ("tokenUsed", Some(number(window.get("window_token_used"))?)),
                ("tokenCap", Some(number(window.get("window_token_cap"))?)),
                ("expiresAt", label(window.get("window_expires_at"))),
            ])))
        })
        .collect();
    let features: Vec<Value> = body
        .get("feature_meters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let meter = object(Some(entry));
            Some(Value::Object(defined([
                ("key", Some(label(meter.get("feature_key"))?)),
                ("used", number(meter.get("total_used"))),
                ("allowance", number(meter.get("total_allowance"))),
                ("remaining", number(meter.get("total_remaining"))),
                ("unit", label(meter.get("unit"))),
                ("unlimited", flag(meter.get("unlimited"))),
                ("periodEnd", label(meter.get("period_end"))),
            ])))
        })
        .collect();
    let mut usage = defined([
        ("plan", (!plan.is_empty()).then_some(Value::Object(plan))),
        (
            "credits",
            (!credits.is_empty()).then_some(Value::Object(credits)),
        ),
    ]);
    usage.insert("windows".into(), Value::Array(windows));
    usage.insert("features".into(), Value::Array(features));
    serde_json::from_value(Value::Object(usage)).map_err(|_| ManagedError::InvalidResponse)
}
