//! Bounded Solana Pay SPL-token checkout. The RPC, never browser input, supplies proof.
use super::*;
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const BASE58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const MONTH: i64 = 30 * 86400;
// Circle's published USDC contract list and Solana's canonical cluster hashes:
// https://developers.circle.com/stablecoins/usdc-contract-addresses
// https://github.com/solana-labs/solana/blob/master/sdk/src/genesis_config.rs
fn approved_usdc(network: &str, genesis: &str, mint: &str) -> bool {
    match network {
        "mainnet-beta" => {
            genesis == "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
                && mint == "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
        }
        "devnet" => {
            genesis == "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
                && mint == "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
        }
        _ => false,
    }
}

#[derive(Clone)]
pub struct SolanaConfig {
    pub rpc_url: String,
    pub genesis_hash: String,
    pub network: String,
    pub mint: String,
    pub recipient: String,
    pub amount: i64,
    pub decimals: u32,
}
impl SolanaConfig {
    pub fn from_env() -> std::result::Result<Option<Self>, String> {
        let names = [
            "OMOBA_SOLANA_RPC_URL",
            "OMOBA_SOLANA_GENESIS_HASH",
            "OMOBA_SOLANA_NETWORK",
            "OMOBA_SOLANA_MINT",
            "OMOBA_SOLANA_TREASURY",
            "OMOBA_SOLANA_AMOUNT_ATOMIC",
        ];
        let values: Vec<_> = names.iter().map(|n| std::env::var(n).ok()).collect();
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        if values.iter().any(Option::is_none) {
            return Err("Incomplete Solana provider configuration".into());
        }
        let v: Vec<String> = values.into_iter().map(Option::unwrap).collect();
        let uri: reqwest::Url = v[0].parse().map_err(|_| "Invalid Solana RPC URL")?;
        let amount: i64 = v[5].parse().map_err(|_| "Invalid Solana price")?;
        if uri.scheme() != "https"
            || uri.host_str().is_none()
            || uri.fragment().is_some()
            || !uri.username().is_empty()
            || uri.password().is_some()
            || !approved_usdc(&v[2], &v[1], &v[3])
            || amount <= 0
            || amount > 1_000_000_000
            || decode58(&v[1]).is_none_or(|b| b.len() != 32)
            || decode58(&v[3]).is_none_or(|b| b.len() != 32)
            || decode58(&v[4]).is_none_or(|b| b.len() != 32)
        {
            return Err("Invalid Solana provider configuration".into());
        }
        // USDC uses six decimal places. No arbitrary or transfer-fee tokens in this adapter.
        Ok(Some(Self {
            rpc_url: v[0].clone(),
            genesis_hash: v[1].clone(),
            network: v[2].clone(),
            mint: v[3].clone(),
            recipient: v[4].clone(),
            amount,
            decimals: 6,
        }))
    }
}
pub fn decimal_amount(amount: i64, decimals: u32) -> String {
    let scale = 10_i64.pow(decimals);
    let whole = amount / scale;
    let fractional = amount % scale;
    if fractional == 0 {
        return whole.to_string();
    }
    format!("{whole}.{:0width$}", fractional, width = decimals as usize)
        .trim_end_matches('0')
        .to_owned()
}
pub fn encode58(bytes: &[u8]) -> String {
    let mut digits = Vec::<u8>::new();
    for byte in bytes {
        let mut carry = u32::from(*byte);
        for d in &mut digits {
            carry += u32::from(*d) * 256;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeroes = bytes.iter().take_while(|b| **b == 0).count();
    std::iter::repeat_n('1', zeroes)
        .chain(digits.iter().rev().map(|d| BASE58[*d as usize] as char))
        .collect()
}
pub fn decode58(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || s.len() > 128 {
        return None;
    }
    let mut bytes = Vec::<u8>::new();
    for ch in s.bytes() {
        let mut carry = BASE58.iter().position(|b| *b == ch)? as u32;
        for b in &mut bytes {
            carry += u32::from(*b) * 58;
            *b = (carry % 256) as u8;
            carry /= 256;
        }
        while carry > 0 {
            bytes.push((carry % 256) as u8);
            carry /= 256;
        }
    }
    let zeroes = s.bytes().take_while(|c| *c == b'1').count();
    let mut result = vec![0; zeroes];
    result.extend(bytes.into_iter().rev());
    Some(result)
}
#[derive(Clone, Debug)]
pub struct Quote {
    pub order_id: String,
    pub profile_id: String,
    pub reference: String,
    pub genesis_hash: String,
    pub network: String,
    pub mint: String,
    pub recipient: String,
    pub amount: i64,
    pub decimals: u32,
    pub created_at: i64,
    pub expires_at: i64,
}
impl Quote {
    fn from_row(r: &sqlx::postgres::PgRow) -> Result<Self> {
        Ok(Self {
            order_id: row_text(r, "order_id")?,
            profile_id: row_text(r, "profile_id")?,
            reference: row_text(r, "reference")?,
            genesis_hash: row_text(r, "genesis_hash")?,
            network: row_text(r, "network")?,
            mint: row_text(r, "mint")?,
            recipient: row_text(r, "recipient")?,
            amount: r.try_get("amount")?,
            decimals: r.try_get::<i32, _>("decimals")? as u32,
            created_at: r.try_get("created_at")?,
            expires_at: r.try_get("expires_at")?,
        })
    }
    fn response(&self) -> Value {
        let amount = decimal_amount(self.amount, self.decimals);
        json!({"order_id":self.order_id,"payment_url":format!("solana:{}?amount={amount}&spl-token={}&reference={}&label=Open%20Moba&message=Supporter%2030%20days",self.recipient,self.mint,self.reference),"amount":amount,"asset":"USDC","network":self.network,"expires_at":self.expires_at,"duration_days":30})
    }
}
/// Account-owned invoices survive page reloads, including already-paid expired quotes.
/// No wallet keys or unrelated account references are exposed.
pub async fn orders(app: &App, profile: &str) -> Result<Value> {
    let rows = sqlx::query("SELECT * FROM portal.supporter_orders WHERE profile_id=$1 AND created_at>$2-604800 ORDER BY created_at DESC,order_id DESC LIMIT 15").bind(profile).bind(now()).fetch_all(&app.pool).await?;
    let mut orders = Vec::new();
    for row in rows {
        let mut order = Quote::from_row(&row)?.response();
        let signature: Option<String> = row.try_get("confirmed_signature")?;
        order["status"] = json!(if signature.is_some() {
            "confirmed"
        } else {
            "pending"
        });
        order["confirmed_signature"] = json!(signature);
        orders.push(order);
    }
    Ok(json!({"orders":orders}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
pub async fn checkout(app: &App, profile: &str, body: &[u8]) -> Result<Value> {
    let _: Empty = parse(body)?;
    let c = app.billing.solana.as_ref().ok_or_else(unavailable)?;
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
        .bind(profile)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(forbidden)?;
    // No Apple+crypto double subscription; renew prepaid only near its expiry.
    let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal.supporter_grants WHERE profile_id=$1 AND revoked_at IS NULL AND ((provider='apple' AND valid_until>$2) OR (provider='solana' AND valid_until>$2+604800))) OR EXISTS(SELECT 1 FROM (SELECT DISTINCT ON (COALESCE(original_transaction_id,period_id)) renewal_enabled,revoked_at FROM portal.supporter_grants WHERE profile_id=$1 AND provider='apple' ORDER BY COALESCE(original_transaction_id,period_id),valid_until DESC,event_version DESC) latest WHERE revoked_at IS NULL AND renewal_enabled IS DISTINCT FROM false)").bind(profile).bind(now()).fetch_one(&mut *tx).await?;
    if blocked {
        return Err(Error(StatusCode::CONFLICT, "already_supported"));
    }
    if let Some(row)=sqlx::query("SELECT * FROM portal.supporter_orders WHERE profile_id=$1 AND expires_at>$2 AND confirmed_signature IS NULL ORDER BY created_at DESC LIMIT 1").bind(profile).bind(now()).fetch_optional(&mut *tx).await?{return Ok(Quote::from_row(&row)?.response());}
    let mut random = [0; 32];
    getrandom::fill(&mut random).map_err(|_| unavailable())?;
    let q = Quote {
        order_id: crypto::random::<16>(),
        profile_id: profile.into(),
        reference: encode58(&random),
        genesis_hash: c.genesis_hash.clone(),
        network: c.network.clone(),
        mint: c.mint.clone(),
        recipient: c.recipient.clone(),
        amount: c.amount,
        decimals: c.decimals,
        created_at: now(),
        expires_at: now() + 900,
    };
    sqlx::query("INSERT INTO portal.supporter_orders(order_id,profile_id,reference,network,genesis_hash,mint,recipient,amount,decimals,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(&q.order_id).bind(profile).bind(&q.reference).bind(&q.network).bind(&q.genesis_hash).bind(&q.mint).bind(&q.recipient).bind(q.amount).bind(q.decimals as i32).bind(q.created_at).bind(q.expires_at).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(q.response())
}
async fn rpc(c: &SolanaConfig, method: &str, params: Value) -> Result<Value> {
    let response = http_client()
        .post(&c.rpc_url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > 2_000_000) {
        return Err(unavailable());
    }
    let data = bounded_response(response, 2_000_000).await?;
    let value: Value = serde_json::from_slice(&data).map_err(|_| unavailable())?;
    if value.get("error").is_some() || value["jsonrpc"] != "2.0" || value["id"] != 1 {
        return Err(unavailable());
    }
    value.get("result").cloned().ok_or_else(unavailable)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirm {
    order_id: String,
    #[serde(default)]
    signature: String,
}
pub async fn confirm(app: &App, profile: &str, body: &[u8]) -> Result<Value> {
    let p: Confirm = parse(body)?;
    let c = app.billing.solana.as_ref().ok_or_else(unavailable)?;
    if crypto::decode::<16>(&p.order_id).is_none()
        || (!p.signature.is_empty() && decode58(&p.signature).is_none_or(|b| b.len() != 64))
    {
        return Err(invalid());
    }
    app.rate(profile, "solana-confirm", 10).await?;
    let row =
        sqlx::query("SELECT * FROM portal.supporter_orders WHERE order_id=$1 AND profile_id=$2")
            .bind(&p.order_id)
            .bind(profile)
            .fetch_optional(&app.pool)
            .await?
            .ok_or_else(forbidden)?;
    let q = Quote::from_row(&row)?;
    if let Some(previous) = row.try_get::<Option<String>, _>("confirmed_signature")? {
        if !p.signature.is_empty() && previous != p.signature {
            return Err(conflict());
        }
        return Ok(json!({"status":"confirmed","supporter":status(app,profile).await?}));
    }
    if q.genesis_hash != c.genesis_hash {
        return Err(unavailable());
    }
    let genesis = rpc(c, "getGenesisHash", json!([])).await?;
    if genesis.as_str() != Some(q.genesis_hash.as_str()) {
        return Err(unavailable());
    }
    let signature = if p.signature.is_empty() {
        let candidates = rpc(
            c,
            "getSignaturesForAddress",
            json!([q.reference,{"commitment":"finalized","limit":5}]),
        )
        .await?;
        let candidates = candidates.as_array().ok_or_else(unavailable)?;
        let mut found = None;
        for candidate in candidates.iter().take(5) {
            if !candidate["err"].is_null() || candidate["confirmationStatus"] != "finalized" {
                continue;
            }
            let Some(signature) = candidate["signature"]
                .as_str()
                .filter(|s| decode58(s).is_some_and(|b| b.len() == 64))
            else {
                continue;
            };
            let transaction = rpc(c, "getTransaction", json!([signature,{"commitment":"finalized","encoding":"json","maxSupportedTransactionVersion":1}])).await?;
            if !transaction.is_null() && validate_transaction(&q, signature, &transaction).is_ok() {
                found = Some(signature.to_owned());
                break;
            }
        }
        found.ok_or(Error(StatusCode::CONFLICT, "payment_not_finalized"))?
    } else {
        let transaction = rpc(c, "getTransaction", json!([p.signature,{"commitment":"finalized","encoding":"json","maxSupportedTransactionVersion":1}])).await?;
        if transaction.is_null() {
            return Err(Error(StatusCode::CONFLICT, "payment_not_finalized"));
        }
        validate_transaction(&q, &p.signature, &transaction)?;
        p.signature
    };
    settle(app, &q, &signature).await?;
    Ok(json!({"status":"confirmed","supporter":status(app,profile).await?}))
}
/// A successful parsed lookup alone is not payment. Require the exact last token transfer,
/// a read-only reference on that instruction, treasury ownership and an actual balance gain.
pub fn validate_transaction(q: &Quote, signature: &str, t: &Value) -> Result<()> {
    // JSON normalizes legacy/v0/v1 instructions and account headers. v1's extra
    // transactionConfig only describes fees/compute; this receiver never sponsors fees.
    // https://solana.com/upgrades/larger-transaction-sizes
    if !matches!(t.get("version"), None | Some(Value::Null))
        && t["version"] != "legacy"
        && t["version"] != 0
        && t["version"] != 1
    {
        return Err(payment_invalid());
    }
    if t["meta"]["err"] != Value::Null
        || t["meta"].is_null()
        || t["transaction"]["signatures"][0].as_str() != Some(signature)
    {
        return Err(payment_invalid());
    }
    let block_time = t["blockTime"].as_i64().ok_or_else(payment_invalid)?;
    if block_time < q.created_at - 30 || block_time > q.expires_at || block_time > now() + 30 {
        return Err(payment_invalid());
    }
    let msg = &t["transaction"]["message"];
    let static_keys = msg["accountKeys"].as_array().ok_or_else(payment_invalid)?;
    let mut keys = static_keys
        .iter()
        .map(|k| k.as_str().map(str::to_owned).ok_or_else(payment_invalid))
        .collect::<Result<Vec<_>>>()?;
    let static_len = keys.len();
    let loaded_writable = t["meta"]["loadedAddresses"]["writable"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let loaded_readonly = t["meta"]["loadedAddresses"]["readonly"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for key in loaded_writable.iter().chain(&loaded_readonly) {
        keys.push(key.as_str().ok_or_else(payment_invalid)?.to_owned());
    }
    let instruction = msg["instructions"]
        .as_array()
        .and_then(|a| a.last())
        .ok_or_else(payment_invalid)?;
    let program = instruction["programIdIndex"]
        .as_u64()
        .ok_or_else(payment_invalid)? as usize;
    if keys.get(program).map(String::as_str) != Some(TOKEN_PROGRAM) {
        return Err(payment_invalid());
    }
    let accounts = instruction["accounts"]
        .as_array()
        .ok_or_else(payment_invalid)?
        .iter()
        .map(|a| {
            a.as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .ok_or_else(payment_invalid)
        })
        .collect::<Result<Vec<_>>>()?;
    let data = decode58(instruction["data"].as_str().ok_or_else(payment_invalid)?)
        .ok_or_else(payment_invalid)?;
    let (destination, minimum) = match data.first() {
        Some(12) if data.len() == 10 && data[9] == q.decimals as u8 => {
            if accounts.len() < 5 || keys.get(accounts[1]).map(String::as_str) != Some(&q.mint) {
                return Err(payment_invalid());
            }
            (accounts[2], 4)
        }
        Some(3) if data.len() == 9 => {
            if accounts.len() < 4 {
                return Err(payment_invalid());
            }
            (accounts[1], 3)
        }
        _ => return Err(payment_invalid()),
    };
    let amount = u64::from_le_bytes(data[1..9].try_into().map_err(|_| payment_invalid())?);
    if amount != q.amount as u64 || destination >= keys.len() {
        return Err(payment_invalid());
    }
    let reference = accounts[minimum..]
        .iter()
        .copied()
        .find(|i| keys.get(*i) == Some(&q.reference))
        .ok_or_else(payment_invalid)?;
    let signed = msg["header"]["numRequiredSignatures"]
        .as_u64()
        .ok_or_else(payment_invalid)? as usize;
    let readonly = msg["header"]["numReadonlyUnsignedAccounts"]
        .as_u64()
        .ok_or_else(payment_invalid)? as usize;
    if readonly > static_len
        || reference < signed
        || (reference < static_len && reference < static_len - readonly)
        || (reference >= static_len && reference < static_len + loaded_writable.len())
    {
        return Err(payment_invalid());
    }
    let balance = |name: &str| -> Result<Option<i128>> {
        let entries = t["meta"][name].as_array().ok_or_else(payment_invalid)?;
        let row = entries
            .iter()
            .find(|r| r["accountIndex"].as_u64() == Some(destination as u64));
        match row {
            None => Ok(None),
            Some(r) => {
                if r["mint"].as_str() != Some(&q.mint)
                    || r["owner"].as_str() != Some(&q.recipient)
                    || r["uiTokenAmount"]["decimals"].as_u64() != Some(q.decimals as u64)
                {
                    return Err(payment_invalid());
                }
                Ok(Some(
                    r["uiTokenAmount"]["amount"]
                        .as_str()
                        .and_then(|a| a.parse().ok())
                        .filter(|a| *a >= 0)
                        .ok_or_else(payment_invalid)?,
                ))
            }
        }
    };
    let gain = balance("postTokenBalances")?.ok_or_else(payment_invalid)?
        - balance("preTokenBalances")?.unwrap_or(0);
    if gain < i128::from(q.amount) {
        return Err(payment_invalid());
    }
    Ok(())
}
async fn settle(app: &App, q: &Quote, signature: &str) -> Result<()> {
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT profile_id FROM career_profiles WHERE profile_id=$1 FOR UPDATE")
        .bind(&q.profile_id)
        .fetch_one(&mut *tx)
        .await?;
    let prior: Option<String> = sqlx::query_scalar(
        "SELECT confirmed_signature FROM portal.supporter_orders WHERE order_id=$1 FOR UPDATE",
    )
    .bind(&q.order_id)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(prior) = prior {
        if prior == signature {
            return Ok(());
        }
        return Err(conflict());
    }
    let used: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM portal.supporter_orders WHERE confirmed_signature=$1)",
    )
    .bind(signature)
    .fetch_one(&mut *tx)
    .await?;
    if used {
        return Err(Error(StatusCode::CONFLICT, "payment_already_used"));
    }
    // Start when payment is claimed, preserving already-paid prepaid days. Apple periods remain separate.
    let previous:Option<i64>=sqlx::query_scalar("SELECT max(valid_until) FROM portal.supporter_grants WHERE profile_id=$1 AND provider='solana' AND revoked_at IS NULL").bind(&q.profile_id).fetch_one(&mut *tx).await?;
    let start = previous.unwrap_or(0).max(now());
    let event = VerifiedEvent {
        event_id: signature.into(),
        provider: "solana".into(),
        period_id: q.order_id.clone(),
        original_transaction_id: None,
        app_account_token: None,
        product_id: None,
        environment: Some(q.network.clone()),
        valid_from: start,
        valid_until: start + MONTH,
        revoked_at: None,
        event_version: now() * 1000,
        renewal_enabled: Some(false),
    };
    apply_event_tx(&mut tx, &q.profile_id, &event).await?;
    sqlx::query("UPDATE portal.supporter_orders SET confirmed_signature=$2,confirmed_at=$3 WHERE order_id=$1").bind(&q.order_id).bind(signature).bind(now()).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn currency_and_chain_labels_are_bound_to_canonical_usdc() {
        let main = (
            "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        );
        let dev = (
            "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG",
            "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
        );
        assert!(approved_usdc("mainnet-beta", main.0, main.1));
        assert!(approved_usdc("devnet", dev.0, dev.1));
        assert!(!approved_usdc("mainnet-beta", dev.0, main.1));
        assert!(!approved_usdc("devnet", dev.0, main.1));
        assert!(!approved_usdc("devnet", dev.0, &encode58(&[7; 32])));
        assert!(!approved_usdc("testnet", dev.0, dev.1));
        for address in [main.0, main.1, dev.0, dev.1] {
            assert_eq!(decode58(address).unwrap().len(), 32);
        }
    }
    fn fixture() -> (Quote, String, Value) {
        let signature = encode58(&[7; 64]);
        let mint = encode58(&[2; 32]);
        let recipient = encode58(&[3; 32]);
        let reference = encode58(&[4; 32]);
        let q = Quote {
            order_id: "order".into(),
            profile_id: "profile".into(),
            reference: reference.clone(),
            genesis_hash: "genesis".into(),
            network: "devnet".into(),
            mint: mint.clone(),
            recipient: recipient.clone(),
            amount: 4_990_000,
            decimals: 6,
            created_at: now() - 60,
            expires_at: now() + 60,
        };
        let mut data = vec![12];
        data.extend(4_990_000_u64.to_le_bytes());
        data.push(6);
        let t = json!({"blockTime":now(),"transaction":{"signatures":[signature],"message":{"accountKeys":["payer","source",mint,"destination",TOKEN_PROGRAM,reference],"header":{"numRequiredSignatures":1,"numReadonlyUnsignedAccounts":2},"instructions":[{"programIdIndex":4,"accounts":[1,2,3,0,5],"data":encode58(&data)}]}},"meta":{"err":null,"preTokenBalances":[],"postTokenBalances":[{"accountIndex":3,"mint":mint,"owner":recipient,"uiTokenAmount":{"decimals":6,"amount":"4990000"}}]}});
        (q, signature, t)
    }
    #[test]
    fn exact_transfer_valid_and_encodings_roundtrip() {
        let (q, s, t) = fixture();
        assert!(validate_transaction(&q, &s, &t).is_ok());
        for bytes in [vec![0; 32], vec![255; 64], vec![0, 0, 3, 9]] {
            assert_eq!(decode58(&encode58(&bytes)).unwrap(), bytes);
        }
        assert!(decode58("0OIl").is_none());
        assert_eq!(decimal_amount(4990000, 6), "4.99");
    }
    #[test]
    fn normalized_legacy_v0_and_v1_transactions_are_supported() {
        let (q, signature, mut transaction) = fixture();
        for version in [json!("legacy"), json!(0), json!(1)] {
            transaction["version"] = version;
            assert!(validate_transaction(&q, &signature, &transaction).is_ok());
        }
        transaction["transaction"]["message"]["transactionConfig"] = json!({"computeUnitLimit":30000,"loadedAccountsDataSizeLimit":200000,"heapSize":null,"priorityFee":null});
        assert!(validate_transaction(&q, &signature, &transaction).is_ok());
        transaction["version"] = json!(2);
        assert!(validate_transaction(&q, &signature, &transaction).is_err());
    }
    #[test]
    fn forged_wrong_or_reused_inputs_do_not_validate() {
        let (q, s, t) = fixture();
        for pointer in [
            "/meta/err",
            "/meta/postTokenBalances/0/owner",
            "/meta/postTokenBalances/0/mint",
            "/meta/postTokenBalances/0/uiTokenAmount/amount",
            "/transaction/signatures/0",
            "/transaction/message/accountKeys/5",
        ] {
            let mut bad = t.clone();
            *bad.pointer_mut(pointer).unwrap() = json!("wrong");
            assert!(validate_transaction(&q, &s, &bad).is_err(), "{pointer}");
        }
        let mut bad = t.clone();
        bad["blockTime"] = json!(q.expires_at + 1);
        assert!(validate_transaction(&q, &s, &bad).is_err());
        let mut bad = t.clone();
        bad["transaction"]["message"]["header"]["numReadonlyUnsignedAccounts"] = json!(0);
        assert!(validate_transaction(&q, &s, &bad).is_err());
        let mut bad = t.clone();
        bad["meta"]["preTokenBalances"] = bad["meta"]["postTokenBalances"].clone();
        assert!(validate_transaction(&q, &s, &bad).is_err());
    }
}
