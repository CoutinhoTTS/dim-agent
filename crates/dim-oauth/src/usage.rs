//! Projects `GET {issuer}/api/me/usage` into the account usage DTO. The
//! service wraps its report as `{success, data}`; a bare report is read as it
//! is. Absent server fields stay absent: no number is coerced to 0, and no raw
//! body or credential is retained.
use crate::http::{Body, safe_integer};
use dim_app_server::managed_provider::{
    ManagedError,
    management::{
        Referral, ResetApplied, ResetBatch, ResetBatches, ResetKind, ResetSummary, Usage,
    },
};
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

fn data(envelope: &Body) -> Result<&Body, ManagedError> {
    if envelope.get("success") == Some(&Value::Bool(false)) {
        return Err(ManagedError::InvalidResponse);
    }
    Ok(match envelope.get("data") {
        Some(data) => data.as_object().ok_or(ManagedError::InvalidResponse)?,
        None => envelope,
    })
}

pub(super) fn project_usage(envelope: &Body) -> Result<Usage, ManagedError> {
    let body = data(envelope)?;
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
        (
            "cancelAtPeriodEnd",
            flag(object(subscription.get("subscription")).get("cancel_at_period_end")),
        ),
        (
            "currentTermStart",
            label(object(subscription.get("current_term")).get("start_at")),
        ),
        (
            "lastTermStatus",
            label(object(subscription.get("last_term")).get("status")),
        ),
        (
            "lastTermStart",
            label(object(subscription.get("last_term")).get("start_at")),
        ),
        (
            "lastTermEnd",
            label(object(subscription.get("last_term")).get("end_at")),
        ),
    ]);
    let totals = object(body.get("credits"));
    let bucket = object(totals.get("subscription_bucket"));
    let credits = defined([
        (
            "usedUnits",
            number(bucket.get("used_units")).or_else(|| number(totals.get("used_units"))),
        ),
        (
            "totalUnits",
            number(bucket.get("total_units")).or_else(|| number(totals.get("total_units"))),
        ),
        (
            "remainingUnits",
            number(bucket.get("remaining_units")).or_else(|| number(totals.get("remaining_units"))),
        ),
        ("unlimited", flag(bucket.get("unlimited"))),
        ("resetsAt", label(bucket.get("expires_at"))),
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
                ("startedAt", label(window.get("window_started_at"))),
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
    let mut usage: Usage =
        serde_json::from_value(Value::Object(usage)).map_err(|_| ManagedError::InvalidResponse)?;
    usage.subscription_id =
        safe_integer(object(subscription.get("subscription")).get("id")).filter(|id| *id > 0);
    usage.resets = body.get("resets").map(project_resets).transpose()?;
    Ok(usage)
}

fn required_number(body: &Body, key: &str) -> Result<u64, ManagedError> {
    safe_integer(body.get(key)).ok_or(ManagedError::InvalidResponse)
}
fn required_label(body: &Body, key: &str) -> Result<String, ManagedError> {
    label(body.get(key))
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(ManagedError::InvalidResponse)
}
fn project_resets(value: &Value) -> Result<ResetSummary, ManagedError> {
    let body = value.as_object().ok_or(ManagedError::InvalidResponse)?;
    let kind = |key| {
        let value = body
            .get(key)
            .and_then(Value::as_object)
            .ok_or(ManagedError::InvalidResponse)?;
        Ok(ResetKind {
            available_count: required_number(value, "available_count")?,
            nearest_expires_at: label(value.get("nearest_expires_at"))
                .and_then(|value| value.as_str().map(str::to_owned)),
        })
    };
    Ok(ResetSummary {
        window: kind("window")?,
        monthly_full: kind("monthly_full")?,
    })
}

pub(super) fn project_referral(envelope: &Body, issuer: &str) -> Result<Referral, ManagedError> {
    let body = data(envelope)?;
    let enabled = body
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or(ManagedError::InvalidResponse)?;
    let eligible = body
        .get("eligible")
        .and_then(Value::as_bool)
        .ok_or(ManagedError::InvalidResponse)?;
    let aff_code = label(body.get("aff_code")).and_then(|value| value.as_str().map(str::to_owned));
    if eligible && aff_code.is_none() {
        return Err(ManagedError::InvalidResponse);
    }
    let invite_url = aff_code
        .as_ref()
        .map(|code| {
            let mut url = url::Url::parse(&format!("{}/register", issuer.trim_end_matches('/')))
                .map_err(|_| ManagedError::InvalidResponse)?;
            url.query_pairs_mut().append_pair("aff", code);
            Ok(url.to_string())
        })
        .transpose()?;
    Ok(Referral {
        enabled,
        eligible,
        aff_code,
        invite_url,
        invited_count: required_number(body, "invited_count")?,
        paid_count: required_number(body, "paid_count")?,
        reward_granted_units: safe_integer(body.get("reward_granted_units")),
        inviter_rebate_bp: required_number(body, "inviter_rebate_bp")?,
        invitee_rebate_bp: required_number(body, "invitee_rebate_bp")?,
    })
}

pub(super) fn project_batches(envelope: &Body) -> Result<ResetBatches, ManagedError> {
    let body = data(envelope)?;
    let batches = |key| -> Result<Vec<ResetBatch>, ManagedError> {
        object(body.get(key))
            .get("batches")
            .and_then(Value::as_array)
            .ok_or(ManagedError::InvalidResponse)?
            .iter()
            .map(|value| {
                let batch = value.as_object().ok_or(ManagedError::InvalidResponse)?;
                let projected = defined([
                    ("grantId", Some(json!(required_number(batch, "grant_id")?))),
                    (
                        "remaining",
                        Some(json!(required_number(batch, "remaining")?)),
                    ),
                    (
                        "expiresAt",
                        Some(json!(required_label(batch, "expires_at")?)),
                    ),
                    ("scopeType", label(batch.get("scope_type"))),
                    (
                        "subscriptionId",
                        safe_integer(batch.get("subscription_id")).map(|id| json!(id)),
                    ),
                    ("sourceType", label(batch.get("source_type"))),
                ]);
                let batch: ResetBatch = serde_json::from_value(Value::Object(projected))
                    .map_err(|_| ManagedError::InvalidResponse)?;
                match batch.scope_type {
                    dim_app_server::managed_provider::management::ResetScope::Account
                        if batch.subscription_id.is_some() =>
                    {
                        return Err(ManagedError::InvalidResponse);
                    }
                    dim_app_server::managed_provider::management::ResetScope::Subscription
                        if batch.subscription_id.is_none_or(|id| id == 0) =>
                    {
                        return Err(ManagedError::InvalidResponse);
                    }
                    _ => {}
                }
                Ok(batch)
            })
            .collect()
    };
    Ok(ResetBatches {
        window: batches("window")?,
        monthly_full: batches("monthly_full")?,
    })
}

pub(super) fn project_applied(envelope: &Body) -> Result<ResetApplied, ManagedError> {
    let body = data(envelope)?;
    let mut bucket = body
        .get("bucket")
        .and_then(Value::as_object)
        .ok_or(ManagedError::InvalidResponse)?
        .clone();
    let windows = body
        .get("window_states")
        .and_then(Value::as_array)
        .ok_or(ManagedError::InvalidResponse)?;
    bucket.insert("window_states".into(), json!(windows));
    let projected = json!({"credits":{"subscription_bucket":bucket}});
    let usage = project_usage(projected.as_object().unwrap())?;
    if usage.windows.len() != windows.len() {
        return Err(ManagedError::InvalidResponse);
    }
    Ok(ResetApplied {
        credits: usage.credits.ok_or(ManagedError::InvalidResponse)?,
        windows: usage.windows,
        resets: project_resets(body.get("resets").ok_or(ManagedError::InvalidResponse)?)?,
        replayed: body
            .get("replayed")
            .and_then(Value::as_bool)
            .ok_or(ManagedError::InvalidResponse)?,
        applied_at: required_label(body, "applied_at")?,
    })
}
