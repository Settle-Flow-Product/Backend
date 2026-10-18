//! The nine endpoints.
//!
//! Three conventions, each enforcing something from the specification:
//!
//! * **Every chain-touching endpoint returns `202`, never `200`.** There is no path by which a
//!   client can read "the API returned OK" as "the money moved".
//! * **Every amount crosses the wire as a string.** JSON numbers are IEEE doubles.
//! * **Every error has the same envelope**, with a stable code, the field to highlight, and a
//!   trace id.
//!
//! DTOs are private and defined immediately above the handler that uses them, so a change to a
//! response shape is visible in the same screenful as the handler it belongs to.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use settle_domain::{ErrorEnvelope, Field, RequestId, Stroops};
use uuid::Uuid;

use crate::state::{now_secs, AppState};

type Reply = axum::response::Response;

pub fn router(state: Arc<AppState>) -> Router {
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(
            state
                .settings
                .allowed_origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect::<Vec<_>>(),
        )
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    Router::new()
        // Public — liveness and readiness.
        .route("/health", get(health))
        .route("/ready", get(ready))
        // Public — where a session comes from.
        .route("/auth/wallet/challenge", post(auth_challenge))
        .route("/auth/wallet/verify", post(auth_verify))
        // Session required.
        .route("/requests", get(list_requests).post(create_request))
        .route("/requests/{id}", get(get_request))
        .route("/requests/{id}/prepare", post(prepare))
        .route("/requests/{id}/signatures", post(submit_signature))
        .route("/requests/{id}/reject", post(reject))
        .route("/requests/{id}/evidence", get(evidence))
        .route("/accounts/{address}/can-receive", get(can_receive))
        .route("/treasury", get(treasury))
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

fn fail(status: StatusCode, code: &str, message: &str) -> Reply {
    (status, Json(ErrorEnvelope::new(code, message))).into_response()
}

fn fail_field(status: StatusCode, code: &str, message: &str, field: Field) -> Reply {
    (
        status,
        Json(ErrorEnvelope::new(code, message).with_field(field)),
    )
        .into_response()
}

fn internal(what: &str, e: impl std::fmt::Display) -> Reply {
    // The detail goes to the operator, never to the client.
    tracing::error!(error = %e, "{what}");
    fail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL",
        "Something went wrong. The trace id is in the logs.",
    )
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

async fn health() -> Reply {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// Ready means the database answers. It deliberately does **not** call the chain: a readiness
/// probe that depends on an external network takes the service out of rotation when the network
/// hiccups, which is precisely when the read paths still work fine.
async fn ready(State(state): State<Arc<AppState>>) -> Reply {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ready"}))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "readiness check failed");
            fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "NOT_READY",
                "The database is not reachable.",
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ChallengeRequest {
    address: String,
}

#[derive(Serialize)]
struct ChallengeResponse {
    nonce: String,
    /// The SEP-10 challenge transaction, base64 XDR. **Sequence number 0 — unsubmittable.**
    transaction_xdr: String,
    domain: String,
    network_passphrase: String,
    expires_at: u64,
}

async fn auth_challenge(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ChallengeRequest>,
) -> Reply {
    let Ok(address) = settle_domain::StellarAddress::parse(&body.address, Field::SignerAddress)
    else {
        return fail_field(
            StatusCode::BAD_REQUEST,
            "INVALID_ADDRESS",
            "That is not a valid Stellar address.",
            Field::SignerAddress,
        );
    };

    let nonce = match settle_auth::generate_nonce() {
        Ok(n) => n,
        Err(e) => return internal("could not generate a nonce", e),
    };

    // One timestamp, used for both the stored row and the transaction's timebounds. Calling
    // `now_secs()` twice would build a transaction that verification could never reproduce.
    let issued_at = now_secs();

    if let Err(e) = settle_db::challenges::issue(
        &state.pool,
        &nonce,
        address.as_str(),
        &state.settings.network_passphrase,
        &state.settings.home_domain,
        state.settings.challenge_ttl_seconds,
    )
    .await
    {
        return internal("could not store the challenge", e);
    }

    let challenge = settle_auth::Challenge::new(
        nonce,
        address.as_str(),
        &state.settings.home_domain,
        &state.settings.network_passphrase,
        issued_at,
    );

    let transaction_xdr = match challenge.transaction_xdr() {
        Ok(x) => x,
        Err(e) => return internal("could not build the challenge transaction", e),
    };

    (
        StatusCode::OK,
        Json(ChallengeResponse {
            nonce: hex::encode(nonce),
            transaction_xdr,
            domain: challenge.domain.clone(),
            network_passphrase: challenge.network_passphrase.clone(),
            expires_at: challenge.expires_at,
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
struct VerifyRequest {
    nonce: String,
    /// The signed challenge transaction, base64 XDR, exactly as the wallet returned it.
    signed_xdr: String,
}

#[derive(Serialize)]
struct VerifyResponse {
    token: String,
    wallet: String,
    expires_at: u64,
}

async fn auth_verify(State(state): State<Arc<AppState>>, Json(body): Json<VerifyRequest>) -> Reply {
    // Every failure returns the same reply. Nothing distinguishes "no such challenge" from "bad
    // signature", because that difference tells an attacker whether an address is enrolled.
    //
    // The REASON is logged every single time, though. An earlier version returned early from two
    // of these paths without logging, and the result was a sign-in that failed with no way to find
    // out why — from either side.
    let refuse = |reason: &str| {
        tracing::warn!(reason, "sign-in refused");
        fail(
            StatusCode::UNAUTHORIZED,
            "AUTHENTICATION_FAILED",
            "Authentication failed.",
        )
    };

    let Some(nonce_bytes) = hex::decode(body.nonce.trim())
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
    else {
        return refuse("nonce_not_32_hex_bytes");
    };

    // Reads AND DELETES in one statement, so a captured challenge cannot be replayed.
    let stored = match settle_db::challenges::take(&state.pool, &nonce_bytes).await {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(error = %e, "challenge take failed");
            return refuse("challenge_not_found_or_replayed");
        }
    };

    let challenge = settle_auth::Challenge {
        nonce: nonce_bytes,
        address: stored.address.clone(),
        domain: stored.domain.clone(),
        network_passphrase: stored.network.clone(),
        issued_at: stored.issued_at.timestamp() as u64,
        expires_at: stored.expires_at.timestamp() as u64,
    };

    if let Err(e) = challenge.check(
        &state.settings.home_domain,
        &state.settings.network_passphrase,
        now_secs(),
    ) {
        return refuse(e.operator_reason());
    }

    // The transaction inside the returned envelope is ignored. Verification rebuilds the expected
    // challenge from the STORED nonce, address and issue time, and checks the signature against
    // that — otherwise a caller could sign a transaction of their own choosing.
    if let Err(e) = settle_auth::verify_signed_challenge(&challenge, body.signed_xdr.trim()) {
        return refuse(e.operator_reason());
    }

    let now = now_secs();
    let token = settle_auth::issue(&stored.address, &state.settings.session_key, now);

    (
        StatusCode::OK,
        Json(VerifyResponse {
            token,
            wallet: stored.address,
            expires_at: now + settle_auth::SESSION_SECONDS,
        }),
    )
        .into_response()
}

/// Require a valid session, returning the wallet it belongs to.
///
/// The error is boxed because an axum `Response` is large, and this sits on the hot path of every
/// authenticated request — an unboxed `Result` would widen every caller's stack frame for a value
/// that is almost never constructed.
fn require_session(state: &AppState, headers: &HeaderMap) -> Result<String, Box<Reply>> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();

    settle_auth::verify(raw, &state.settings.session_key, now_secs())
        .map(|s| s.wallet)
        .map_err(|_| {
            Box::new(fail(
                StatusCode::UNAUTHORIZED,
                "AUTHENTICATION_FAILED",
                "Sign in again.",
            ))
        })
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ListQuery {
    status: Option<String>,
    limit: Option<i64>,
}

async fn list_requests(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Reply {
    if let Err(r) = require_session(&state, &headers) {
        return *r;
    }
    // Read from the projection. No synchronous Horizon call sits in a read path.
    match settle_db::requests::list(
        &state.pool,
        q.status.as_deref(),
        q.limit.unwrap_or(100).min(500),
    )
    .await
    {
        Ok(rows) => (StatusCode::OK, Json(rows)).into_response(),
        Err(e) => internal("could not list requests", e),
    }
}

#[derive(Serialize)]
struct CreateResponse {
    request_id: String,
    status: String,
    create_tx_hash: Option<String>,
    server_time: DateTime<Utc>,
}

async fn create_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<settle_domain::NewRequest>,
) -> Reply {
    // The wallet is KEPT, not just checked. It was `_wallet` — verified and dropped — which left
    // `audit_events` unable to answer the first half of "who did what", and the approver screen with
    // no "Người tạo" to show. The company is on chain; the person can only be recorded here.
    let Ok(wallet) = require_session(&state, &headers) else {
        return fail(
            StatusCode::UNAUTHORIZED,
            "AUTHENTICATION_FAILED",
            "Sign in again.",
        );
    };

    // Mandatory. Without it there is no protection against the second of two clicks.
    let Some(idem) = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v.trim()).ok())
    else {
        return fail(
            StatusCode::BAD_REQUEST,
            "IDEMPOTENCY_KEY_REQUIRED",
            "An Idempotency-Key header is required.",
        );
    };

    let now = now_secs();
    let expiry = now + 24 * 3_600;
    let validated = match settle_domain::validate(&body, now, expiry) {
        Ok(v) => v,
        Err(e) => {
            let env: ErrorEnvelope = e.into();
            return (StatusCode::BAD_REQUEST, Json(env)).into_response();
        }
    };

    // Claim the key BEFORE doing anything else, so the race resolves in the database.
    let body_hash = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(
            serde_json::to_vec(&serde_json::json!({
                "payee": validated.payee.as_str(),
                "amount": validated.amount.to_string(),
                "asset": validated.asset.code(),
                "reference": validated.reference,
            }))
            .unwrap_or_default(),
        );
        let d = h.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&d);
        out
    };

    // A stable business identity for this deployment. The registry's creator is the treasury's
    // operator, and the id derivation needs a namespace that does not change between restarts.
    let business_id = business_namespace(&state.settings.treasury_address);
    let request_id = RequestId::derive(business_id, idem);

    match settle_db::idempotency::claim(&state.pool, "create_request", idem, &body_hash).await {
        Ok(settle_db::idempotency::Claim::Claimed) => {}
        Ok(settle_db::idempotency::Claim::AlreadyDone { result_ref }) => {
            // The same call again. Return the original result rather than doing the work twice.
            return (
                StatusCode::ACCEPTED,
                Json(CreateResponse {
                    request_id: result_ref,
                    status: "pending".into(),
                    create_tx_hash: None,
                    server_time: Utc::now(),
                }),
            )
                .into_response();
        }
        Ok(settle_db::idempotency::Claim::InFlight) => {
            return fail(
                StatusCode::CONFLICT,
                "IDEMPOTENCY_IN_FLIGHT",
                "That request is already being created. Wait and poll — do not change the key.",
            )
        }
        Ok(settle_db::idempotency::Claim::BodyMismatch) => {
            return fail(
                StatusCode::CONFLICT,
                "IDEMPOTENCY_CONFLICT",
                "That idempotency key was used for a different payment.",
            )
        }
        Err(e) => return internal("could not claim the idempotency key", e),
    }

    let expiry_at = Utc
        .timestamp_opt(expiry as i64, 0)
        .single()
        .unwrap_or_else(Utc::now);
    let asset_contract = match validated.asset {
        settle_domain::Asset::Usdc => state.settings.usdc_contract.clone(),
        settle_domain::Asset::Xlm => state.settings.xlm_contract.clone(),
    };
    if asset_contract.is_empty() {
        // Refuse here rather than store a request the chain write can only fail on. Without this
        // the request is accepted, two people sign a payment for it, and only then does anybody
        // discover it never reached the registry.
        return fail_field(
            StatusCode::UNPROCESSABLE_ENTITY,
            "ASSET_NOT_CONFIGURED",
            "This asset has no contract configured on this deployment.",
            Field::AssetCode,
        );
    }

    // A trace id per action, so the interface click, the audit row and the chain write that follows
    // minutes later can all be tied together — the acceptance criteria ask for exactly that link,
    // and a trace that stops at the API boundary joins nothing.
    let trace_id = Uuid::new_v4().to_string();

    let new = settle_db::requests::NewRequestRow {
        id: *request_id.as_bytes(),
        // The BUSINESS creates the request; the treasury only pays it. Recording the treasury here
        // made `create_request` require the treasury's own 2-of-3 authorisation, which no single
        // backend key can ever satisfy.
        creator_address: state.settings.business_address.clone(),
        // The person. `creator_address` above is the company, which is what the contract
        // authorises as and what the chain records.
        created_by: Some(wallet.clone()),
        trace_id: Some(trace_id.clone()),
        payee_address: validated.payee.as_str().to_string(),
        asset_code: validated.asset.code().to_string(),
        asset_contract: asset_contract.clone(),
        amount: validated.amount.to_string(),
        reference: validated.reference.clone(),
        reference_hash: validated.reference_hash,
        expiry_at,
        contract_id: state.settings.registry_contract_id.clone(),
        idempotency_key: idem,
    };

    let row = match settle_db::requests::insert(&state.pool, &new).await {
        Ok(r) => r,
        Err(e) if e.is_duplicate() => {
            // The contract's RequestExists gate, arrived at from the database side.
            return (
                StatusCode::ACCEPTED,
                Json(CreateResponse {
                    request_id: request_id.to_hex(),
                    status: "pending".into(),
                    create_tx_hash: None,
                    server_time: Utc::now(),
                }),
            )
                .into_response();
        }
        Err(e) => return internal("could not record the request", e),
    };

    let _ = settle_db::idempotency::finish(&state.pool, "create_request", idem, &row.id).await;
    tracing::info!(request = %row.id, actor = %wallet, trace_id = %trace_id, "payment request created");

    let _ = settle_db::audit::record(
        &state.pool,
        "business",
        Some(&wallet),
        "payment_request",
        &row.id,
        "created",
        None,
        Some("pending"),
        Some(&trace_id),
    )
    .await;

    // 202, not 200. The chain write is submitted by the background task and confirmed by a
    // readback; at this instant nobody knows whether it landed.
    (
        StatusCode::ACCEPTED,
        Json(CreateResponse {
            request_id: row.id,
            status: row.status,
            create_tx_hash: row.create_tx_hash,
            server_time: Utc::now(),
        }),
    )
        .into_response()
}

/// A stable namespace for request id derivation.
///
/// Derived from the treasury address rather than random, so the same inputs derive the same id
/// across restarts — which is the whole point of a deterministic id.
pub fn business_namespace(treasury: &str) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(treasury.as_bytes());
    let d = h.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&d[..16]);
    Uuid::from_bytes(bytes)
}

async fn get_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Reply {
    if let Err(r) = require_session(&state, &headers) {
        return *r;
    }
    let Some(rid) = RequestId::parse_hex(&id) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST_ID",
            "That is not a request id.",
        );
    };

    match settle_db::requests::get(&state.pool, rid.as_bytes()).await {
        Ok(row) => {
            let settlement = settle_db::settlements::for_request(&state.pool, rid.as_bytes())
                .await
                .ok()
                .flatten();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "request": row, "settlement": settlement })),
            )
                .into_response()
        }
        // 404 covers "exists but is not yours" too: distinguishing them tells an unauthorised
        // caller that an id is real.
        Err(_) => fail(StatusCode::NOT_FOUND, "NOT_FOUND", "No such payment."),
    }
}

#[derive(Serialize)]
struct PrepareResponse {
    settlement_id: Uuid,
    /// The account the money actually leaves. The approver screen shows this as "From account";
    /// showing the creator there named the wrong account, since the business raises the request and
    /// the treasury is what pays it.
    treasury_address: String,
    unsigned_xdr: String,
    tx_hash: String,
    network_passphrase: String,
    valid_until: DateTime<Utc>,
    required_weight: i32,
    collected_weight: i32,
    signers: Vec<SignerView>,
}

#[derive(Serialize)]
struct SignerView {
    address: String,
    role: String,
    weight: i32,
    signed: bool,
}

async fn prepare(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Reply {
    if let Err(r) = require_session(&state, &headers) {
        return *r;
    }
    let Some(rid) = RequestId::parse_hex(&id) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST_ID",
            "That is not a request id.",
        );
    };

    let request = match settle_db::requests::get(&state.pool, rid.as_bytes()).await {
        Ok(r) => r,
        Err(_) => return fail(StatusCode::NOT_FOUND, "NOT_FOUND", "No such payment."),
    };

    // The request must be ON CHAIN before anybody signs a payment for it.
    //
    // The registry is what `approve` and `mark_paid` act on, so a request that never landed there
    // cannot be settled — but nothing stopped a settlement being built from it. Two approvers signed
    // a payment for a request the contract had never heard of, and the failure surfaced as
    // `RequestNotFound` from `approve`, long after their time had been spent.
    if request.create_tx_hash.is_none() {
        return fail(
            StatusCode::CONFLICT,
            "NOT_ON_CHAIN_YET",
            "This payment has not reached the registry yet. Try again shortly.",
        );
    }

    if !matches!(request.status.as_str(), "pending" | "approved") {
        return fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "NOT_PAYABLE",
            "This payment is already paid, cancelled or expired.",
        );
    }

    // An existing settlement is returned rather than rebuilt: rebuilding would change the
    // transaction hash and void every signature already collected.
    if let Ok(Some(existing)) =
        settle_db::settlements::for_request(&state.pool, rid.as_bytes()).await
    {
        if matches!(existing.status.as_str(), "awaiting_approval") {
            return prepare_response(&state, existing).await;
        }
    }

    let policy =
        match settle_db::policy::active(&state.pool, &state.settings.treasury_address).await {
            Ok(p) => p,
            Err(e) => return internal("no active policy for this treasury", e),
        };

    // The one unavoidable network call in a user-facing path: a sequence number exists only on the
    // network and cannot be cached.
    let sequence = match state
        .horizon
        .sequence(&state.settings.treasury_address)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "could not read the treasury sequence");
            return fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "CHAIN_UNREACHABLE",
                "Could not reach the network. Try again shortly.",
            );
        }
    };

    let amount = match Stroops::parse(&request.amount) {
        Ok(a) => a,
        Err(e) => return internal("a stored amount does not parse", e),
    };
    let asset = match settle_domain::Asset::parse(&request.asset_code) {
        Ok(a) => a,
        Err(e) => return internal("a stored asset code does not parse", e),
    };

    let now = now_secs();
    let max_time = now + state.settings.tx_timebounds_seconds;

    let spec = settle_chain::PaymentSpec {
        source: state.settings.treasury_address.clone(),
        destination: request.payee_address.clone(),
        asset,
        asset_issuer: Some(state.settings.usdc_issuer.clone()),
        amount,
        sequence: sequence + 1,
        memo_request_id: *rid.as_bytes(),
        min_time: now,
        max_time,
        fee: state.settings.base_fee,
    };

    let unsigned = match settle_chain::build_payment(&spec, &state.network) {
        Ok(u) => u,
        Err(e) => return internal("could not build the payment", e),
    };
    let xdr = match unsigned.to_xdr_base64() {
        Ok(x) => x,
        Err(e) => return internal("could not encode the transaction", e),
    };

    let new = settle_db::settlements::NewSettlement {
        request_id: *rid.as_bytes(),
        treasury_address: state.settings.treasury_address.clone(),
        destination_address: request.payee_address.clone(),
        asset_code: request.asset_code.clone(),
        amount: request.amount.clone(),
        policy_version: policy.version,
        required_weight: policy.threshold,
        unsigned_xdr: xdr,
        tx_hash_to_sign: unsigned.tx_hash_hex(),
        reserved_seq: spec.sequence,
        valid_until: Utc
            .timestamp_opt(max_time as i64, 0)
            .single()
            .unwrap_or_else(Utc::now),
    };

    match settle_db::settlements::reserve(&state.pool, &new).await {
        Ok(row) => prepare_response(&state, row).await,
        // F-09 surfacing in the API: refused cleanly at the start, rather than accepted, signed by
        // two people, and killed later by tx_bad_seq.
        Err(settle_db::DbError::ReleaseInFlight) => fail(
            StatusCode::CONFLICT,
            "RELEASE_IN_FLIGHT",
            "Another payment from this treasury is still awaiting signatures.",
        ),
        Err(e) => internal("could not reserve the treasury sequence", e),
    }
}

async fn prepare_response(state: &AppState, row: settle_db::settlements::SettlementRow) -> Reply {
    let policy = settle_db::policy::version(
        &state.pool,
        &state.settings.treasury_address,
        row.policy_version,
    )
    .await;

    let approvals = settle_db::settlements::approvals(&state.pool, row.id)
        .await
        .unwrap_or_default();

    let signers = match policy {
        Ok(p) => p
            .signers
            .iter()
            .map(|s| SignerView {
                address: s.address.clone(),
                role: s.role.clone(),
                weight: s.weight,
                signed: approvals.iter().any(|a| a.signer_address == s.address),
            })
            .collect(),
        Err(_) => vec![],
    };

    (
        StatusCode::OK,
        Json(PrepareResponse {
            settlement_id: row.id,
            treasury_address: row.treasury_address.clone(),
            unsigned_xdr: row.unsigned_xdr.unwrap_or_default(),
            tx_hash: row.tx_hash_to_sign.unwrap_or_default(),
            network_passphrase: state.settings.network_passphrase.clone(),
            valid_until: row.valid_until.unwrap_or_else(Utc::now),
            required_weight: row.required_weight,
            collected_weight: row.collected_weight,
            signers,
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
struct SignatureRequest {
    signer_address: String,
    /// Hex-encoded, 64 bytes.
    signature: String,
}

#[derive(Serialize)]
struct SignatureResponse {
    collected_weight: i32,
    required_weight: i32,
    threshold_met: bool,
}

async fn submit_signature(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<SignatureRequest>,
) -> Reply {
    if let Err(r) = require_session(&state, &headers) {
        return *r;
    }
    let Some(rid) = RequestId::parse_hex(&id) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST_ID",
            "That is not a request id.",
        );
    };

    let settlement = match settle_db::settlements::for_request(&state.pool, rid.as_bytes()).await {
        Ok(Some(s)) => s,
        _ => return fail(StatusCode::NOT_FOUND, "NOT_FOUND", "Nothing to sign."),
    };

    if settlement.status != "awaiting_approval" {
        return fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "NOT_AWAITING_SIGNATURES",
            "This payment is no longer collecting signatures.",
        );
    }

    if let Some(valid_until) = settlement.valid_until {
        if Utc::now() >= valid_until {
            return fail(
                StatusCode::UNPROCESSABLE_ENTITY,
                "TX_EXPIRED",
                "The signing window has closed. The transaction needs to be rebuilt.",
            );
        }
    }

    // The PINNED policy version, not the current one.
    let policy = match settle_db::policy::version(
        &state.pool,
        &settlement.treasury_address,
        settlement.policy_version,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => return internal("could not read the pinned policy", e),
    };

    let Some(signer) = policy
        .signers
        .iter()
        .find(|s| s.address == body.signer_address.trim())
    else {
        return fail_field(
            StatusCode::FORBIDDEN,
            "NOT_A_SIGNER",
            "This wallet is not an approver for this payment.",
            Field::SignerAddress,
        );
    };

    let Some(tx_hash_hex) = settlement.tx_hash_to_sign.clone() else {
        return internal(
            "a settlement has no transaction hash",
            "missing tx_hash_to_sign",
        );
    };
    let Some(tx_hash) = hex::decode(&tx_hash_hex)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
    else {
        return internal("a stored transaction hash is malformed", &tx_hash_hex);
    };

    let Ok(sig_vec) = hex::decode(body.signature.trim()) else {
        return fail_field(
            StatusCode::BAD_REQUEST,
            "BAD_SIGNATURE",
            "The signature is not valid hex.",
            Field::Signature,
        );
    };
    let Ok(sig) = <[u8; 64]>::try_from(sig_vec.clone()) else {
        return fail_field(
            StatusCode::BAD_REQUEST,
            "BAD_SIGNATURE",
            "A signature is 64 bytes.",
            Field::Signature,
        );
    };

    let Ok(public_key) = settle_chain::decode_account(&signer.address) else {
        return internal("a stored signer address does not decode", &signer.address);
    };

    // Verified against THIS transaction hash, not another.
    if settle_chain::verify_signature(public_key, &tx_hash, &sig).is_err() {
        return fail_field(
            StatusCode::BAD_REQUEST,
            "BAD_SIGNATURE",
            "The signature does not match this transaction.",
            Field::Signature,
        );
    }

    let collected = match settle_db::settlements::add_approval(
        &state.pool,
        settlement.id,
        &signer.address,
        &sig,
        signer.weight,
        &tx_hash_hex,
    )
    .await
    {
        Ok(c) => c,
        // The primary key refused it. One signer cannot reach a threshold of two alone.
        Err(e) if e.is_duplicate() => {
            return fail(
                StatusCode::CONFLICT,
                "ALREADY_SIGNED",
                "This signer has already signed.",
            )
        }
        Err(e) => return internal("could not record the signature", e),
    };

    let trace_id = Uuid::new_v4().to_string();
    tracing::info!(settlement = %settlement.id, actor = %signer.address, trace_id = %trace_id,
        "signature accepted");

    let _ = settle_db::audit::record(
        &state.pool,
        "approver",
        Some(&signer.address),
        "settlement",
        &settlement.id.to_string(),
        "signed",
        None,
        None,
        Some(&trace_id),
    )
    .await;

    // The submit happens in a worker. The approver's request returns as soon as the signature is
    // stored — it does not wait on Horizon.
    (
        StatusCode::OK,
        Json(SignatureResponse {
            collected_weight: collected,
            required_weight: settlement.required_weight,
            threshold_met: collected >= settlement.required_weight,
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
struct RejectRequest {
    signer_address: String,
}

async fn reject(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RejectRequest>,
) -> Reply {
    if let Err(r) = require_session(&state, &headers) {
        return *r;
    }
    let Some(rid) = RequestId::parse_hex(&id) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST_ID",
            "That is not a request id.",
        );
    };

    // Release the treasury first. Otherwise one refusal blocks it indefinitely.
    if let Ok(Some(settlement)) =
        settle_db::settlements::for_request(&state.pool, rid.as_bytes()).await
    {
        if let Err(e) =
            settle_db::settlements::mark_rejected(&state.pool, settlement.id, &body.signer_address)
                .await
        {
            return internal("could not record the rejection", e);
        }
    }

    if let Err(e) = settle_db::requests::set_status(
        &state.pool,
        rid.as_bytes(),
        "cancelled",
        &["pending", "approved"],
    )
    .await
    {
        return internal("could not cancel the request", e);
    }

    let _ = settle_db::audit::record(
        &state.pool,
        "approver",
        Some(&body.signer_address),
        "payment_request",
        &rid.to_hex(),
        "rejected",
        None,
        Some("cancelled"),
        Some(&Uuid::new_v4().to_string()),
    )
    .await;

    // The on-chain cancel is submitted by the background task. 202, because at this instant it has
    // not happened yet.
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "status": "cancelled" })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Destination preflight
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CanReceiveQuery {
    asset: Option<String>,
}

#[derive(Serialize)]
struct CanReceiveResponse {
    address: String,
    asset: &'static str,
    /// The account exists on this network.
    exists: bool,
    /// It exists AND holds what it needs to receive the asset.
    can_receive: bool,
}

/// Module E2's destination check: can this vendor address actually be paid in this asset?
///
/// This runs while the person is still typing, behind a debounce, and it is the difference between
/// a payment that fails at the field and one that fails at the ledger — `op_no_trust` surfaces at
/// SUBMIT, which is after both approvers have already been interrupted to sign. The trustline risk
/// is the one the delivery plan rates most likely to embarrass a demo, and this is where it gets
/// caught.
///
/// It answers with `200` and a verdict rather than an error status. "This address cannot receive
/// USDC" is a successful answer to the question asked; reserving the error envelope for genuine
/// failures keeps a red field from looking like a broken API.
async fn can_receive(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(address): Path<String>,
    Query(q): Query<CanReceiveQuery>,
) -> Reply {
    if let Err(r) = require_session(&state, &headers) {
        return *r;
    }

    // Refuse the malformed address here rather than spending a Horizon round trip on it. The same
    // parser the create path uses, so the two cannot drift into disagreeing about what is valid.
    let address = match settle_domain::StellarAddress::parse(&address, Field::PayeeAddress) {
        Ok(a) => a.as_str().to_string(),
        Err(e) => {
            return fail_field(
                StatusCode::BAD_REQUEST,
                e.code(),
                "That is not a valid Stellar address.",
                Field::PayeeAddress,
            )
        }
    };

    let asset = match settle_domain::Asset::parse(q.asset.as_deref().unwrap_or("USDC")) {
        Ok(a) => a,
        Err(_) => {
            return fail_field(
                StatusCode::BAD_REQUEST,
                "ASSET_NOT_SUPPORTED",
                "That asset is not supported.",
                Field::AssetCode,
            )
        }
    };

    let reply = |exists: bool, can: bool| {
        (
            StatusCode::OK,
            Json(CanReceiveResponse {
                address: address.clone(),
                asset: asset.code(),
                exists,
                can_receive: can,
            }),
        )
            .into_response()
    };

    // Native XLM needs no trustline, so existence is the whole question. A `Payment` operation to
    // an account that does not exist fails either way, which is why existence is still checked.
    if !asset.requires_trustline() {
        return match state.horizon.account(&address).await {
            Ok(_) => reply(true, true),
            Err(crate::horizon::HorizonError::AccountNotFound(_)) => reply(false, false),
            // Horizon refused the address itself. The local parser checks the alphabet and the length
            // but deliberately not the strkey checksum, so a single transposed character arrives here
            // looking well formed — and "that address is not valid" is the true and useful thing to
            // say about it, not "the network is unreachable".
            Err(crate::horizon::HorizonError::BadRequest(_)) => fail_field(
                StatusCode::BAD_REQUEST,
                "INVALID_ADDRESS",
                "That is not a valid Stellar address.",
                Field::PayeeAddress,
            ),
            Err(e) => {
                tracing::warn!(error = %e, "could not check the destination account");
                fail(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "CHAIN_UNREACHABLE",
                    "Could not reach the network.",
                )
            }
        };
    }

    match state
        .horizon
        .can_receive(&address, asset.code(), &state.settings.usdc_issuer)
        .await
    {
        Ok(crate::horizon::Receivable::Yes) => reply(true, true),
        Ok(crate::horizon::Receivable::NoTrustline) => reply(true, false),
        Err(crate::horizon::HorizonError::AccountNotFound(_)) => reply(false, false),
        // Horizon refused the address itself. The local parser checks the alphabet and the length
        // but deliberately not the strkey checksum, so a single transposed character arrives here
        // looking well formed — and "that address is not valid" is the true and useful thing to
        // say about it, not "the network is unreachable".
        Err(crate::horizon::HorizonError::BadRequest(_)) => fail_field(
            StatusCode::BAD_REQUEST,
            "INVALID_ADDRESS",
            "That is not a valid Stellar address.",
            Field::PayeeAddress,
        ),
        Err(e) => {
            tracing::warn!(error = %e, "could not check the destination account");
            fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "CHAIN_UNREACHABLE",
                "Could not reach the network.",
            )
        }
    }
}

#[derive(Serialize)]
struct TreasuryResponse {
    address: String,
    signer_count: usize,
    thresholds: serde_json::Value,
    master_weight: i64,
    /// The configuration SOW 1 requires: three signers, thresholds 2, master weight 0.
    ///
    /// Asserted rather than displayed. A screen that shows the numbers and leaves a human to check
    /// them is a screen that passes on demo day while the master key still signs alone.
    correct: bool,
    problems: Vec<String>,
}

/// Read the treasury's configuration back from Horizon — evidence milestone E2.
async fn treasury(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Reply {
    if let Err(r) = require_session(&state, &headers) {
        return *r;
    }

    let account = match state
        .horizon
        .account(&state.settings.treasury_address)
        .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, "could not read the treasury");
            return fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "CHAIN_UNREACHABLE",
                "Could not reach the network.",
            );
        }
    };

    let signers = account
        .get("signers")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();

    // The master key is the signer whose key equals the account id. Its weight must be zero, or
    // whoever holds the treasury secret signs alone and the whole approval process is decoration.
    let master_weight = signers
        .iter()
        .find(|s| s.get("key").and_then(|k| k.as_str()) == Some(&state.settings.treasury_address))
        .and_then(|s| s.get("weight"))
        .and_then(|w| w.as_i64())
        .unwrap_or(0);

    let extra_signers: Vec<&serde_json::Value> = signers
        .iter()
        .filter(|s| s.get("key").and_then(|k| k.as_str()) != Some(&state.settings.treasury_address))
        .collect();

    let thresholds = account
        .get("thresholds")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let threshold_of = |name: &str| thresholds.get(name).and_then(|v| v.as_i64()).unwrap_or(0);

    let mut problems = Vec::new();
    if extra_signers.len() != 3 {
        problems.push(format!(
            "expected 3 signers besides the master key, found {}",
            extra_signers.len()
        ));
    }
    if master_weight != 0 {
        problems.push(format!(
            "the master key still has weight {master_weight} — it can sign alone"
        ));
    }
    for name in ["low_threshold", "med_threshold", "high_threshold"] {
        let v = threshold_of(name);
        if v != 2 {
            problems.push(format!("{name} is {v}, expected 2"));
        }
    }

    (
        StatusCode::OK,
        Json(TreasuryResponse {
            address: state.settings.treasury_address.clone(),
            signer_count: extra_signers.len(),
            thresholds,
            master_weight,
            correct: problems.is_empty(),
            problems,
        }),
    )
        .into_response()
}

#[derive(Serialize)]
struct EvidenceRow {
    milestone: &'static str,
    event: &'static str,
    value: Option<String>,
    verify_url: Option<String>,
}

/// The evidence page's data — the six milestones a reviewer opens.
async fn evidence(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Reply {
    if let Err(r) = require_session(&state, &headers) {
        return *r;
    }
    let Some(rid) = RequestId::parse_hex(&id) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST_ID",
            "That is not a request id.",
        );
    };

    let request = match settle_db::requests::get(&state.pool, rid.as_bytes()).await {
        Ok(r) => r,
        Err(_) => return fail(StatusCode::NOT_FOUND, "NOT_FOUND", "No such payment."),
    };
    let settlement = settle_db::settlements::for_request(&state.pool, rid.as_bytes())
        .await
        .ok()
        .flatten();

    let tx_url = |h: &String| format!("https://stellar.expert/explorer/testnet/tx/{h}");

    let rows = vec![
        EvidenceRow {
            milestone: "E1",
            event: "Deploy registry contract",
            value: Some(state.settings.registry_contract_id.clone()),
            verify_url: Some(format!(
                "https://stellar.expert/explorer/testnet/contract/{}",
                state.settings.registry_contract_id
            )),
        },
        EvidenceRow {
            milestone: "E2",
            event: "Treasury · 3 signers · threshold 2",
            value: Some(state.settings.treasury_address.clone()),
            verify_url: Some(format!(
                "https://stellar.expert/explorer/testnet/account/{}",
                state.settings.treasury_address
            )),
        },
        EvidenceRow {
            milestone: "E3",
            event: "create_request",
            value: request.create_tx_hash.clone(),
            verify_url: request.create_tx_hash.as_ref().map(tx_url),
        },
        EvidenceRow {
            milestone: "E4",
            event: "approve · threshold met",
            // The transaction, not the tally. "2/2" is true and unverifiable; the whole point of
            // this page is that a reviewer can check every line against the public ledger.
            value: settlement.as_ref().and_then(|s| s.approve_tx_hash.clone()),
            verify_url: settlement
                .as_ref()
                .and_then(|s| s.approve_tx_hash.as_ref().map(tx_url)),
        },
        EvidenceRow {
            milestone: "E5",
            event: "Payment to vendor",
            value: settlement.as_ref().and_then(|s| s.tx_hash.clone()),
            verify_url: settlement
                .as_ref()
                .and_then(|s| s.tx_hash.as_ref().map(tx_url)),
        },
        EvidenceRow {
            milestone: "E6",
            event: "mark_paid · get_request returns Paid",
            value: settlement
                .as_ref()
                .and_then(|s| s.mark_paid_tx_hash.clone())
                .or_else(|| (request.status == "paid").then(|| "paid".to_string())),
            verify_url: settlement
                .as_ref()
                .and_then(|s| s.mark_paid_tx_hash.as_ref().map(tx_url)),
        },
    ];

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "reference": request.reference,
            "status": request.status,
            "complete": rows.iter().all(|r| r.value.is_some()),
            "milestones": rows,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_business_namespace_is_stable_across_restarts() {
        // A random namespace would derive a different request id every restart, which defeats the
        // entire point of a deterministic id.
        let a = business_namespace("GTREASURY1");
        let b = business_namespace("GTREASURY1");
        assert_eq!(a, b);
        assert_ne!(a, business_namespace("GTREASURY2"));
    }
}
