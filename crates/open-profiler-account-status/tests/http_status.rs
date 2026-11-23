use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use opensoft_open_profiler_account_status::{
    auth::{Auth, IdentityConfig},
    http, model,
    store::{Engine, Store},
};
use rsa::{
    pkcs8::{EncodePrivateKey, LineEnding},
    traits::PublicKeyParts,
    RsaPrivateKey,
};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

struct Tokens {
    key: EncodingKey,
    issuer: String,
}
impl Tokens {
    fn create(&self, subject: &str, audience: &str, expires: i64) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("fixture".into());
        encode(
            &header,
            &json!({"iss":self.issuer, "aud":audience, "azp":"fixture-client",
            "sub":subject, "exp":expires}),
            &self.key,
        )
        .unwrap()
    }
}

async fn send(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["Cache-Control"], "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

fn principal(subject: &str) -> Value {
    json!({"subject":subject,"client_id":"fixture-client"})
}

fn projection(generation: i64, assigned: bool) -> Value {
    let account = |key| json!({"provider":"claude","account_ref":key});
    let workload = |factory| json!({"environment":"qa","factory_instance":factory,"role":"runner","host":null,"persona":null});
    let grant = |subject, name, use_account, manage, scope| {
        json!({
            "principal":principal(subject), "account":account(name),
            "use_account":use_account, "manage":manage, "workload":scope
        })
    };
    let mut grants = vec![
        grant("operator", "A", true, true, Value::Null),
        grant("operator", "a", true, true, Value::Null),
        grant("desktop2", "A", true, false, Value::Null),
        grant("factory2", "a", true, false, workload("ledger")),
    ];
    if assigned {
        grants.push(grant("hermes", "A", true, false, workload("hermes")));
        grants.push(grant("factory1", "A", true, false, workload("medx")));
    }
    json!({"generation":generation,"expires_at":model::now()+3600,
        "accounts":[{"account":account("A"),"display_name":"Company account A"},
                   {"account":account("a"),"display_name":"A distinct account"}],
        "members":(["operator","desktop2","hermes","factory1","factory2","empty"].map(principal)),
        "grants":grants})
}

#[tokio::test]
#[ignore = "requires the selected disposable PostgreSQL, MariaDB, or MySQL instance"]
async fn account_status_contract_on_real_database() {
    let db_url = std::env::var("OPENPROFILER_TEST_DB_URL").unwrap();
    let engine = Engine::parse(&std::env::var("OPENPROFILER_TEST_DB_ENGINE").unwrap()).unwrap();
    let store = Store::connect(engine, &db_url, true).await.unwrap();
    store.migrate().await.unwrap();
    store.migrate().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let private = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
    let jwks = json!({"keys":[{"kty":"RSA","alg":"RS256","use":"sig","kid":"fixture",
        "n":URL_SAFE_NO_PAD.encode(private.n().to_bytes_be()),
        "e":URL_SAFE_NO_PAD.encode(private.e().to_bytes_be())}]});
    let pkcs8 = private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let tokens = Tokens {
        issuer: issuer.clone(),
        key: EncodingKey::from_rsa_pem(pkcs8.as_bytes()).unwrap(),
    };
    let jwks_server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/jwks", get(move || async move { Json(jwks) })),
        )
        .await
        .unwrap();
    });
    let company = format!("company-{}", Uuid::new_v4());
    let prefix = format!("/v1/companies/{company}/accounts");
    let write = format!("{prefix}/claude/A/status");
    let detail = format!("{prefix}/claude/A");
    let admission = format!("{detail}/admissions");
    let publish = format!("/v1/control/companies/{company}/binding-projection");
    let auth = Auth::connect(
        serde_json::from_value::<IdentityConfig>(json!({
            "issuer":issuer, "audience":"openprofiler", "jwks_url":format!("{issuer}/jwks"),
            "publishers":{(&company):[principal("publisher")]}
        }))
        .unwrap(),
        true,
    )
    .await
    .unwrap();
    let app = http::router(store.clone(), auth.clone());
    let fresh = http::router(Store::connect(engine, &db_url, true).await.unwrap(), auth);
    let token = |who| tokens.create(who, "openprofiler", model::now() + 300);
    let publisher = token("publisher");
    let operator = token("operator");
    assert_eq!(
        send(&app, "PUT", &publish, &publisher, projection(1, true))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let wrong = tokens.create("operator", "wrong-audience", model::now() + 300);
    let expired = tokens.create("operator", "openprofiler", model::now() - 1);
    for rejected in [wrong, expired] {
        assert_eq!(
            send(&app, "GET", &prefix, &rejected, Value::Null).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        send(&app, "GET", &prefix, &token("unknown"), Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, "GET", &prefix, &token("empty"), Value::Null)
            .await
            .1["accounts"],
        json!([])
    );
    assert_eq!(
        send(&app, "GET", &detail, &operator, Value::Null).await.1["status"],
        "unknown"
    );
    let (status, first) = send(
        &fresh,
        "GET",
        &format!("{prefix}?provider=claude&limit=1"),
        &operator,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(first["accounts"][0]["account_ref"], "A");
    assert_eq!(first["next_cursor"], "claude/A");
    let second_path = format!("{prefix}?provider=claude&limit=1&cursor=claude%2FA");
    let (status, second) = send(&fresh, "GET", &second_path, &operator, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(second["accounts"][0]["account_ref"], "a");
    assert!(second["next_cursor"].is_null());
    let (status, restricted) =
        send(&fresh, "GET", &second_path, &token("desktop2"), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(restricted["accounts"], json!([]));
    let patch = |state, revision| json!({"desired_status":state,"expected_revision":revision,"request_id":Uuid::new_v4()});
    assert_eq!(
        send(
            &app,
            "PUT",
            &write,
            &token("desktop2"),
            patch("active", Value::Null)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let same_first_a = patch("active", Value::Null);
    let same_first_b = patch("active", Value::Null);
    let (first, other) = tokio::join!(
        send(&app, "PUT", &write, &operator, same_first_a),
        send(&fresh, "PUT", &write, &operator, same_first_b)
    );
    assert!(matches!(
        (first.0, other.0),
        (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::OK)
    ));
    let pause = patch("paused", json!("1"));
    assert_eq!(
        send(&app, "PUT", &write, &operator, pause.clone()).await.1["account"]["revision"],
        "2"
    );
    let operation = json!({"operation_id":Uuid::new_v4(),"operation_kind":"turn"});
    for consumer in ["desktop2", "hermes", "factory1"] {
        let consumer = token(consumer);
        let (_, observed) = send(&fresh, "GET", &detail, &consumer, Value::Null).await;
        assert_eq!(observed["status"], "paused");
        assert_eq!(observed["revision"], "2");
        let (_, denial) = send(&fresh, "POST", &admission, &consumer, operation.clone()).await;
        assert_eq!(denial["decision"], "paused");
        assert_eq!(denial["operation_id"], operation["operation_id"]);
    }
    let factory2 = token("factory2");
    let missing = send(
        &app,
        "GET",
        &format!("{prefix}/claude/missing"),
        &factory2,
        Value::Null,
    )
    .await;
    let denied = send(&app, "GET", &detail, &factory2, Value::Null).await;
    assert_eq!(denied.0, StatusCode::NOT_FOUND);
    assert_eq!(denied.1["code"], missing.1["code"]);
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("{prefix}/claude/a"),
            &factory2,
            Value::Null
        )
        .await
        .1["status"],
        "unknown"
    );
    assert_eq!(
        send(&app, "PUT", &write, &operator, pause.clone()).await.1["outcome"],
        "replayed"
    );
    assert_eq!(
        send(&app, "PUT", &write, &operator, patch("paused", json!("2")))
            .await
            .1["outcome"],
        "noop"
    );
    assert_eq!(
        send(&app, "PUT", &write, &operator, patch("active", json!("2")))
            .await
            .1["account"]["revision"],
        "3"
    );
    let replay = send(&app, "PUT", &write, &operator, pause).await.1;
    assert_eq!(replay["outcome"], "replayed");
    assert_eq!(replay["accepted_revision"], "2");
    assert_eq!(replay["account"]["revision"], "3");
    let stale = send(&app, "PUT", &write, &operator, patch("paused", json!("1"))).await;
    assert_eq!(stale.0, StatusCode::CONFLICT);
    assert_eq!(stale.1["account"]["revision"], "3");
    assert_eq!(
        send(
            &app,
            "POST",
            &admission,
            &token("hermes"),
            operation.clone()
        )
        .await
        .1["decision"],
        "allow"
    );
    assert_eq!(
        send(&app, "PUT", &publish, &publisher, projection(2, false))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, "PUT", &publish, &publisher, projection(1, true))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        send(&fresh, "GET", &detail, &token("hermes"), Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&fresh, "POST", &admission, &token("factory1"), operation)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let pool = sqlx::AnyPool::connect(&db_url).await.unwrap();
    let count_sql = if engine == Engine::Postgres {
        "SELECT COUNT(*) FROM status_transition WHERE company_id=$1"
    } else {
        "SELECT COUNT(*) FROM status_transition WHERE company_id=?"
    };
    let count: i64 = sqlx::query_scalar(count_sql)
        .bind(&company)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);

    // Reserve the next audit key to force a failure after the status UPDATE.
    // The entire transition, including the status and request ID, must roll back.
    let insert_audit = if engine == Engine::Postgres {
        "INSERT INTO status_transition VALUES($1,'claude','A',4,'fixture','active','paused',0,'fixture')"
    } else {
        "INSERT INTO status_transition VALUES(?,'claude','A',4,'fixture','active','paused',0,'fixture')"
    };
    sqlx::query(insert_audit)
        .bind(&company)
        .execute(&pool)
        .await
        .unwrap();
    let rollback_edit = patch("paused", json!("3"));
    assert_eq!(
        send(&app, "PUT", &write, &operator, rollback_edit.clone())
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        send(&fresh, "GET", &detail, &operator, Value::Null).await.1["revision"],
        "3"
    );
    let delete_audit = if engine == Engine::Postgres {
        "DELETE FROM status_transition WHERE company_id=$1 AND revision=4"
    } else {
        "DELETE FROM status_transition WHERE company_id=? AND revision=4"
    };
    sqlx::query(delete_audit)
        .bind(&company)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        send(&app, "PUT", &write, &operator, rollback_edit).await.1["account"]["revision"],
        "4"
    );

    let approved = serde_json::from_value::<model::Projection>(projection(2, false)).unwrap();
    let superseded = serde_json::from_value::<model::Projection>(projection(1, true)).unwrap();
    assert!(matches!(
        store.renew_approved(&company, &superseded).await,
        Err(opensoft_open_profiler_account_status::store::Fault::ProjectionConflict)
    ));
    let (renewed_generation, renewed_expiry) = store
        .renew_approved(&company, &approved)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renewed_generation, 3);
    assert!(renewed_expiry > approved.expires_at);
    assert_eq!(
        send(&fresh, "GET", &detail, &operator, Value::Null).await.1["revision"],
        "4"
    );

    let expire = if engine == Engine::Postgres {
        "UPDATE company_projection SET expires_at=0 WHERE company_id=$1"
    } else {
        "UPDATE company_projection SET expires_at=0 WHERE company_id=?"
    };
    sqlx::query(expire)
        .bind(&company)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        store.renew_approved(&company, &approved).await,
        Err(opensoft_open_profiler_account_status::store::Fault::ProjectionConflict)
    ));
    assert_eq!(
        send(&fresh, "GET", &detail, &operator, Value::Null).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        send(
            &fresh,
            "POST",
            &admission,
            &operator,
            json!({"operation_id":Uuid::new_v4(),"operation_kind":"activation"})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    jwks_server.abort();
}
