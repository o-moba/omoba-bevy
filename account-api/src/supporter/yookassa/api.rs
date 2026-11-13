//! The YooKassa REST client (`/v3`): plain HTTPS with the shop's Basic
//! credentials, no SDK. Responses are parsed into the few fields settlement
//! needs and checked (ids, RUB, amounts as integer kopecks, the `paid` flag).
//! A provider body is never logged: it can carry card and customer data. A
//! failed call logs only its operation, the HTTP status and YooKassa's
//! `x-request-id`.
use super::{KOPECKS, PaymentStatus, VerifiedPayment, VerifiedRefund, YooKassaConfig};
use crate::supporter::{bounded_response, payment_invalid, unavailable};
use crate::*;
use serde::Deserialize;

/// The largest provider reply read (a refund list page of 100 is far smaller).
const MAX_RESPONSE: usize = 262_144;
/// The most refunds one list page asks for (YooKassa allows 1–100).
pub const MAX_PAGE: usize = 100;

/// A provider id (payment, refund): YooKassa uses UUID-like ids.
pub fn provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A list cursor YooKassa returns: opaque printable ASCII (the cursor column's CHECK).
pub fn list_cursor(value: &str) -> bool {
    (1..=512).contains(&value.len()) && value.bytes().all(|b| (b'!'..=b'~').contains(&b))
}

/// `"1000.00"` → 100000 kopecks; anything else (signs, exponents, other
/// precision, more than 12 integer digits) is refused. No floats.
pub fn amount_minor(value: &str) -> Option<i64> {
    let (rubles, kopecks) = value.split_once('.')?;
    if !(1..=12).contains(&rubles.len())
        || kopecks.len() != 2
        || !rubles.bytes().all(|b| b.is_ascii_digit())
        || !kopecks.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    rubles
        .parse::<i64>()
        .ok()?
        .checked_mul(KOPECKS)?
        .checked_add(kopecks.parse().ok()?)
}

/// 100000 kopecks → `"1000.00"`.
pub fn amount_value(minor: i64) -> String {
    format!("{}.{:02}", minor / KOPECKS, minor % KOPECKS)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// An ISO 8601 provider time (`2026-10-25T10:51:18.139Z`, or with a `±HH:MM`
/// offset) as epoch seconds; fractions are dropped.
pub fn parse_time(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if !(20..=64).contains(&bytes.len()) {
        return None;
    }
    let number = |range| number_in(value, range);
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let mut rest = &value[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=9).contains(&digits) {
            return None;
        }
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" => 0,
        _ if rest.len() == 6 && rest.as_bytes()[3] == b':' => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let hours = number_in(rest, 1..3)?;
            let minutes = number_in(rest, 4..6)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            sign * (hours * 3600 + minutes * 60)
        }
        _ => return None,
    };
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    // Refuse 31 April and friends: the day must survive the round trip.
    if civil_from_days(days) != (year, month, day) {
        return None;
    }
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

fn number_in(value: &str, range: std::ops::Range<usize>) -> Option<i64> {
    let part = value.get(range)?;
    part.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| part.parse().ok())?
}

/// Epoch seconds as the UTC time YooKassa's list filters take.
pub fn format_time(epoch: i64) -> String {
    let (year, month, day) = civil_from_days(epoch.div_euclid(86_400));
    let second = epoch.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        second / 3600,
        second % 3600 / 60,
        second % 60
    )
}

/// Only a provider HTTPS redirect without embedded credentials may reach the
/// browser.
pub fn safe_confirmation_url(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 2048
        || value
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return None;
    }
    let url = reqwest::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some_and(|host| !host.is_empty())
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

#[derive(Deserialize)]
struct RawAmount {
    value: String,
    currency: String,
}

#[derive(Deserialize)]
struct RawConfirmation {
    #[serde(rename = "type")]
    kind: String,
    confirmation_url: Option<String>,
}

#[derive(Deserialize)]
struct RawPayment {
    id: String,
    status: String,
    paid: bool,
    amount: RawAmount,
    created_at: Option<String>,
    captured_at: Option<String>,
    confirmation: Option<RawConfirmation>,
    metadata: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize)]
struct RawRefund {
    id: String,
    payment_id: String,
    status: String,
    amount: RawAmount,
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct RawList {
    #[serde(rename = "type")]
    kind: String,
    items: Vec<RawRefund>,
    next_cursor: Option<String>,
}

/// A payment as YooKassa reported it to this shop.
#[derive(Clone, Debug)]
pub struct ProviderPayment {
    pub payment: VerifiedPayment,
    /// The redirect of a pending payment (HTTPS, no credentials).
    pub confirmation_url: Option<String>,
}

/// A refund's provider status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefundStatus {
    Pending,
    Succeeded,
    Canceled,
}

/// A refund as YooKassa reported it to this shop.
#[derive(Clone, Debug)]
pub struct ProviderRefund {
    pub refund: VerifiedRefund,
    pub status: RefundStatus,
}

/// A page of the shop's refunds.
#[derive(Clone, Debug)]
pub struct RefundPage {
    pub items: Vec<ProviderRefund>,
    pub next_cursor: Option<String>,
}

fn rubles(amount: &RawAmount) -> Result<i64> {
    if amount.currency != "RUB" {
        return Err(payment_invalid());
    }
    amount_minor(&amount.value).ok_or_else(payment_invalid)
}

fn payment(raw: RawPayment) -> Result<ProviderPayment> {
    let status = match raw.status.as_str() {
        "pending" => PaymentStatus::Pending,
        "waiting_for_capture" => PaymentStatus::WaitingForCapture,
        "succeeded" => PaymentStatus::Succeeded,
        "canceled" => PaymentStatus::Canceled,
        _ => return Err(payment_invalid()),
    };
    // `paid` must agree with the status, or the object is not trusted.
    let paid = matches!(
        status,
        PaymentStatus::Succeeded | PaymentStatus::WaitingForCapture
    );
    if !provider_id(&raw.id) || raw.paid != paid {
        return Err(payment_invalid());
    }
    let paid_at = if status == PaymentStatus::Succeeded {
        let at = raw.captured_at.as_deref().or(raw.created_at.as_deref());
        Some(at.and_then(parse_time).ok_or_else(payment_invalid)?)
    } else {
        None
    };
    let attempt_id = match raw.metadata.as_ref().and_then(|m| m.get("attempt_id")) {
        None => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(_) => return Err(payment_invalid()),
    };
    let confirmation_url = raw
        .confirmation
        .filter(|c| c.kind == "redirect")
        .and_then(|c| c.confirmation_url)
        .and_then(|url| safe_confirmation_url(&url));
    Ok(ProviderPayment {
        payment: VerifiedPayment {
            payment_id: raw.id,
            status,
            amount_minor: rubles(&raw.amount)?,
            paid_at,
            attempt_id,
        },
        confirmation_url,
    })
}

fn refund(raw: RawRefund) -> Result<ProviderRefund> {
    let status = match raw.status.as_str() {
        "pending" => RefundStatus::Pending,
        "succeeded" => RefundStatus::Succeeded,
        "canceled" => RefundStatus::Canceled,
        _ => return Err(payment_invalid()),
    };
    if !provider_id(&raw.id) || !provider_id(&raw.payment_id) {
        return Err(payment_invalid());
    }
    let amount_minor = rubles(&raw.amount)?;
    let refunded_at = raw
        .created_at
        .as_deref()
        .and_then(parse_time)
        .ok_or_else(payment_invalid)?;
    if amount_minor <= 0 {
        return Err(payment_invalid());
    }
    Ok(ProviderRefund {
        refund: VerifiedRefund {
            refund_id: raw.id,
            payment_id: raw.payment_id,
            amount_minor,
            refunded_at,
        },
        status,
    })
}

/// Parse a payment object (a create reply or a re-read).
pub fn parse_payment(bytes: &[u8]) -> Result<ProviderPayment> {
    payment(serde_json::from_slice(bytes).map_err(|_| payment_invalid())?)
}

/// Parse a refund object.
pub fn parse_refund(bytes: &[u8]) -> Result<ProviderRefund> {
    refund(serde_json::from_slice(bytes).map_err(|_| payment_invalid())?)
}

/// Parse a refund list page of at most `limit` items.
pub fn parse_refund_page(bytes: &[u8], limit: usize) -> Result<RefundPage> {
    let raw: RawList = serde_json::from_slice(bytes).map_err(|_| payment_invalid())?;
    if raw.kind != "list"
        || raw.items.len() > limit
        || raw.next_cursor.as_deref().is_some_and(|c| !list_cursor(c))
    {
        return Err(payment_invalid());
    }
    Ok(RefundPage {
        items: raw
            .items
            .into_iter()
            .map(refund)
            .collect::<Result<Vec<_>>>()?,
        next_cursor: raw.next_cursor,
    })
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        // Two sequential calls (a refund, then its payment) fit one webhook request.
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| unavailable())
}

/// The provider's correlation id, when it looks like one.
fn request_id(response: &reqwest::Response) -> String {
    response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|id| {
            (1..=128).contains(&id.len())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
        })
        .map(|id| format!(" request={id}"))
        .unwrap_or_default()
}

fn rejected() -> Error {
    Error(StatusCode::BAD_GATEWAY, "payment_provider_rejected")
}

/// The body of a successful reply, `None` for 404 (the shop has no such
/// object). A non-2xx reply is logged by operation, status and request id
/// only: another 4xx is `payment_provider_rejected`, anything else
/// `payment_provider_unavailable`.
async fn found(
    operation: &str,
    response: reqwest::Result<reqwest::Response>,
) -> Result<Option<Vec<u8>>> {
    let response = response.map_err(|_| {
        eprintln!("yookassa {operation}: unreachable");
        unavailable()
    })?;
    let status = response.status();
    if !status.is_success() {
        eprintln!(
            "yookassa {operation}: {}{}",
            status.as_u16(),
            request_id(&response)
        );
        return match status {
            StatusCode::NOT_FOUND => Ok(None),
            StatusCode::TOO_MANY_REQUESTS => Err(unavailable()),
            _ if status.is_client_error() => Err(rejected()),
            _ => Err(unavailable()),
        };
    }
    bounded_response(response, MAX_RESPONSE).await.map(Some)
}

/// [`found`] where a 404 is `payment_provider_rejected` too.
async fn body(operation: &str, response: reqwest::Result<reqwest::Response>) -> Result<Vec<u8>> {
    found(operation, response).await?.ok_or_else(rejected)
}

/// `POST /payments` with the persisted Idempotence-Key. The caller stored the
/// key (and the attempt) before this call; a replay with the same key and body
/// returns the same payment.
pub async fn create_payment(
    config: &YooKassaConfig,
    idempotence_key: &str,
    request: &Value,
) -> Result<ProviderPayment> {
    let response = client()?
        .post(format!("{}/payments", config.api_base))
        .basic_auth(&config.shop_id, Some(&config.secret_key))
        .header("Idempotence-Key", idempotence_key)
        .json(request)
        .send()
        .await;
    parse_payment(&body("create payment", response).await?)
}

/// `GET /payments/{id}`: the source of truth, never a webhook body. `None`
/// when the shop has no such payment.
pub async fn find_payment(config: &YooKassaConfig, id: &str) -> Result<Option<ProviderPayment>> {
    if !provider_id(id) {
        return Err(payment_invalid());
    }
    let response = client()?
        .get(format!("{}/payments/{id}", config.api_base))
        .basic_auth(&config.shop_id, Some(&config.secret_key))
        .send()
        .await;
    let Some(bytes) = found("get payment", response).await? else {
        return Ok(None);
    };
    let payment = parse_payment(&bytes)?;
    if payment.payment.payment_id != id {
        return Err(payment_invalid());
    }
    Ok(Some(payment))
}

/// [`find_payment`] of a payment that must exist (404 is `payment_provider_rejected`).
pub async fn get_payment(config: &YooKassaConfig, id: &str) -> Result<ProviderPayment> {
    find_payment(config, id).await?.ok_or_else(rejected)
}

/// `GET /refunds/{id}`. `None` when the shop has no such refund.
pub async fn find_refund(config: &YooKassaConfig, id: &str) -> Result<Option<ProviderRefund>> {
    if !provider_id(id) {
        return Err(payment_invalid());
    }
    let response = client()?
        .get(format!("{}/refunds/{id}", config.api_base))
        .basic_auth(&config.shop_id, Some(&config.secret_key))
        .send()
        .await;
    let Some(bytes) = found("get refund", response).await? else {
        return Ok(None);
    };
    let refund = parse_refund(&bytes)?;
    if refund.refund.refund_id != id {
        return Err(payment_invalid());
    }
    Ok(Some(refund))
}

/// [`find_refund`] of a refund that must exist (404 is `payment_provider_rejected`).
pub async fn get_refund(config: &YooKassaConfig, id: &str) -> Result<ProviderRefund> {
    find_refund(config, id).await?.ok_or_else(rejected)
}

/// `GET /refunds` created in `(after, until]`, one page.
pub async fn list_refunds(
    config: &YooKassaConfig,
    after: i64,
    until: i64,
    cursor: Option<&str>,
    limit: usize,
) -> Result<RefundPage> {
    if after >= until || !(1..=MAX_PAGE).contains(&limit) || cursor.is_some_and(|c| !list_cursor(c))
    {
        return Err(invalid());
    }
    let mut query = vec![
        ("created_at.gt", format_time(after)),
        ("created_at.lte", format_time(until)),
        ("limit", limit.to_string()),
    ];
    if let Some(cursor) = cursor {
        query.push(("cursor", cursor.to_owned()));
    }
    let response = client()?
        .get(format!("{}/refunds", config.api_base))
        .basic_auth(&config.shop_id, Some(&config.secret_key))
        .query(&query)
        .send()
        .await;
    parse_refund_page(&body("list refunds", response).await?, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_are_integer_kopecks() {
        assert_eq!(amount_minor("1000.00"), Some(100_000));
        assert_eq!(amount_minor("123.45"), Some(12_345));
        assert_eq!(amount_minor("0.01"), Some(1));
        for bad in [
            "1000",
            "1000.0",
            "1000.000",
            "-1.00",
            "+1.00",
            "1e3.00",
            "1 000.00",
            ".50",
            "",
            "1000000000000.00",
        ] {
            assert_eq!(amount_minor(bad), None, "{bad}");
        }
        assert_eq!(amount_value(100_000), "1000.00");
        assert_eq!(amount_value(12_345), "123.45");
        assert_eq!(amount_value(10_000_000), "100000.00");
    }

    #[test]
    fn provider_times_round_trip_through_epoch_seconds() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2026-10-25T10:51:18.139Z"), Some(1_792_925_478));
        assert_eq!(parse_time("2026-10-25T13:51:18+03:00"), Some(1_792_925_478));
        assert_eq!(parse_time("2024-02-29T00:00:00.000Z"), Some(1_709_164_800));
        for bad in [
            "2026-10-25 10:51:18Z",
            "2026-10-25T10:51:18",
            "2026-02-30T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-10-25T24:00:00Z",
            "2026-10-25T10:51:18.Z",
            "2026-10-25T10:51:18+0300",
            "2026-10-25T10:51:18.1234567890Z",
        ] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
        for epoch in [0, 1_709_164_800, 1_792_925_478, 4_102_444_799] {
            assert_eq!(parse_time(&format_time(epoch)), Some(epoch));
        }
        assert_eq!(format_time(1_792_925_478), "2026-10-25T10:51:18.000Z");
    }

    #[test]
    fn only_https_confirmation_urls_without_credentials_reach_the_browser() {
        assert_eq!(
            safe_confirmation_url("https://yoomoney.ru/checkout/payments/v2/contract?orderId=1")
                .as_deref(),
            Some("https://yoomoney.ru/checkout/payments/v2/contract?orderId=1")
        );
        for bad in [
            "http://yoomoney.ru/checkout",
            "https://user:pass@yoomoney.ru/checkout",
            "https://user@yoomoney.ru/",
            "javascript:alert(1)",
            "https://yoomoney.ru/ checkout",
            "https://yoomoney.ru/\ncheckout",
            "",
        ] {
            assert_eq!(safe_confirmation_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn payments_are_parsed_and_checked() {
        let succeeded = br#"{"id":"30c5a1b2-000f-5000-9000-1a2b3c4d5e6f","status":"succeeded","paid":true,
            "amount":{"value":"1000.00","currency":"RUB"},"created_at":"2026-10-25T10:00:00.000Z",
            "captured_at":"2026-10-25T10:01:00.000Z","metadata":{"attempt_id":"0123456789abcdef0123456789abcdef"},
            "payment_method":{"type":"bank_card","card":{"last4":"4444"}},"receipt_registration":"succeeded"}"#;
        let parsed = parse_payment(succeeded).unwrap();
        assert_eq!(parsed.payment.status, PaymentStatus::Succeeded);
        assert_eq!(parsed.payment.amount_minor, 100_000);
        assert_eq!(
            parsed.payment.paid_at,
            parse_time("2026-10-25T10:01:00.000Z")
        );
        assert_eq!(
            parsed.payment.attempt_id.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        let pending = br#"{"id":"p1","status":"pending","paid":false,"amount":{"value":"250.00","currency":"RUB"},
            "confirmation":{"type":"redirect","confirmation_url":"https://yoomoney.ru/checkout/payments/v2/contract?orderId=p1"}}"#;
        let parsed = parse_payment(pending).unwrap();
        assert_eq!(parsed.payment.paid_at, None);
        assert!(parsed.confirmation_url.unwrap().starts_with("https://"));
        for bad in [
            // `paid` disagrees with the status.
            &br#"{"id":"p1","status":"succeeded","paid":false,"amount":{"value":"250.00","currency":"RUB"},"captured_at":"2026-10-25T10:01:00Z"}"#[..],
            br#"{"id":"p1","status":"pending","paid":true,"amount":{"value":"250.00","currency":"RUB"}}"#,
            // Another currency, a float amount, an unknown status, a bad id.
            br#"{"id":"p1","status":"pending","paid":false,"amount":{"value":"250.00","currency":"USD"}}"#,
            br#"{"id":"p1","status":"pending","paid":false,"amount":{"value":250.0,"currency":"RUB"}}"#,
            br#"{"id":"p1","status":"refunded","paid":false,"amount":{"value":"250.00","currency":"RUB"}}"#,
            br#"{"id":"p 1","status":"pending","paid":false,"amount":{"value":"250.00","currency":"RUB"}}"#,
            // A success without a time, a non-string attempt id.
            br#"{"id":"p1","status":"succeeded","paid":true,"amount":{"value":"250.00","currency":"RUB"}}"#,
            br#"{"id":"p1","status":"pending","paid":false,"amount":{"value":"250.00","currency":"RUB"},"metadata":{"attempt_id":7}}"#,
        ] {
            assert!(parse_payment(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        // An insecure redirect is dropped, not passed on.
        let insecure = br#"{"id":"p1","status":"pending","paid":false,"amount":{"value":"250.00","currency":"RUB"},
            "confirmation":{"type":"redirect","confirmation_url":"http://example.test/pay"}}"#;
        assert_eq!(parse_payment(insecure).unwrap().confirmation_url, None);
    }

    #[test]
    fn refunds_and_pages_are_parsed_and_checked() {
        let page = br#"{"type":"list","items":[{"id":"r1","payment_id":"p1","status":"succeeded",
            "amount":{"value":"400.00","currency":"RUB"},"created_at":"2026-10-25T11:00:00.000Z"}],
            "next_cursor":"37a5c87d-3984-51e8-a7f3-8de646d39ec15"}"#;
        let parsed = parse_refund_page(page, 10).unwrap();
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].refund.amount_minor, 40_000);
        assert_eq!(parsed.items[0].status, RefundStatus::Succeeded);
        assert_eq!(
            parsed.next_cursor.as_deref(),
            Some("37a5c87d-3984-51e8-a7f3-8de646d39ec15")
        );
        assert!(parse_refund_page(page, 0).is_err(), "more items than asked");
        assert!(parse_refund_page(br#"{"type":"payment","items":[]}"#, 10).is_err());
        assert!(
            parse_refund_page(br#"{"type":"list","items":[],"next_cursor":"a b"}"#, 10).is_err()
        );
        assert!(
            parse_refund(br#"{"id":"r1","payment_id":"p1","status":"succeeded","amount":{"value":"1.00","currency":"RUB"}}"#)
                .is_err(),
            "a refund needs its time"
        );
    }
}
