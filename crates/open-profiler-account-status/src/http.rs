use crate::{
    auth::{Auth, AuthError},
    model::{self, AdmissionRequest, Principal, Projection, StatusWrite},
    store::{self, Fault, Store},
};
use axum::{
    extract::{
        rejection::{JsonRejection, QueryRejection},
        DefaultBodyLimit, Path, Query, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Clone)]
struct App {
    store: Store,
    auth: Auth,
}

pub fn router(store: Store, auth: Auth) -> Router {
    Router::new()
        .route("/readyz", get(ready))
        .route("/v1/companies/{company}/accounts", get(list))
        .route(
            "/v1/companies/{company}/accounts/{provider}/{account}",
            get(detail),
        )
        .route(
            "/v1/companies/{company}/accounts/{provider}/{account}/status",
            put(write),
        )
        .route(
            "/v1/companies/{company}/accounts/{provider}/{account}/admissions",
            post(admit),
        )
        .route(
            "/v1/control/companies/{company}/binding-projection",
            put(publish),
        )
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            },
        ))
        .with_state(App { store, auth })
}

#[derive(Debug)]
enum Error {
    Unauthorized,
    Store(Fault),
}

impl From<Fault> for Error {
    fn from(value: Fault) -> Self {
        Self::Store(value)
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code, account) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "authentication_required", None),
            Self::Store(fault) => match fault {
                Fault::Invalid => (StatusCode::BAD_REQUEST, "invalid_request", None),
                Fault::NotFound => (StatusCode::NOT_FOUND, "account_not_found", None),
                Fault::Scope => (StatusCode::FORBIDDEN, "scope_forbidden", None),
                Fault::Action => (StatusCode::FORBIDDEN, "action_forbidden", None),
                Fault::Stale(account) => (StatusCode::CONFLICT, "stale_revision", Some(account)),
                Fault::Conflict => (StatusCode::CONFLICT, "request_id_conflict", None),
                Fault::ProjectionConflict => {
                    (StatusCode::CONFLICT, "projection_revision_conflict", None)
                }
                Fault::Unavailable => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "authority_unavailable",
                    None,
                ),
            },
        };
        let mut body = json!({"code": code, "correlation_ref": store::correlation()});
        if let Some(account) = account {
            body["account"] = json!(account);
        }
        (status, Json(body)).into_response()
    }
}

async fn ready(State(app): State<App>) -> Result<StatusCode, Error> {
    if !app.auth.ready().await {
        return Err(Fault::Unavailable.into());
    }
    app.store.writer().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn principal(app: &App, headers: &HeaderMap) -> Result<Principal, Error> {
    let mut auth = headers.get_all(header::AUTHORIZATION).iter();
    let value = auth
        .next()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(Error::Unauthorized)?;
    if auth.next().is_some() {
        return Err(Error::Unauthorized);
    }
    app.auth
        .authenticate(value)
        .await
        .map_err(|error| match error {
            AuthError::Invalid => Error::Unauthorized,
            AuthError::Unavailable => Fault::Unavailable.into(),
        })
}

fn validate(company: &str, provider: &str, account: &str) -> Result<(), Fault> {
    if model::key(company) && model::provider(provider) && model::key(account) {
        Ok(())
    } else {
        Err(Fault::Invalid)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    provider: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

async fn list(
    State(app): State<App>,
    headers: HeaderMap,
    Path(company): Path<String>,
    page: Result<Query<Page>, QueryRejection>,
) -> Result<impl IntoResponse, Error> {
    let actor = principal(&app, &headers).await?;
    let Query(page) = page.map_err(|_| Fault::Invalid)?;
    let limit = page.limit.unwrap_or(100);
    if !model::key(&company)
        || !(1..=100).contains(&limit)
        || page
            .provider
            .as_deref()
            .is_some_and(|v| !model::provider(v))
        || page.cursor.as_deref().is_some_and(|v| {
            !v.split_once('/')
                .is_some_and(|(p, a)| model::provider(p) && model::key(a))
        })
    {
        return Err(Fault::Invalid.into());
    }
    Ok(Json(
        app.store
            .list(
                &company,
                &actor,
                page.provider.as_deref(),
                page.cursor.as_deref(),
                limit,
            )
            .await?,
    ))
}

async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path((company, provider, account)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, Error> {
    let actor = principal(&app, &headers).await?;
    validate(&company, &provider, &account)?;
    Ok(Json(
        app.store
            .account(&company, &provider, &account, &actor, false)
            .await?,
    ))
}

async fn write(
    State(app): State<App>,
    headers: HeaderMap,
    Path((company, provider, account)): Path<(String, String, String)>,
    body: Result<Json<StatusWrite>, JsonRejection>,
) -> Result<impl IntoResponse, Error> {
    let actor = principal(&app, &headers).await?;
    validate(&company, &provider, &account)?;
    let Json(request) = body.map_err(|_| Fault::Invalid)?;
    Ok(Json(
        app.store
            .write(&company, &provider, &account, &actor, request)
            .await?,
    ))
}

async fn admit(
    State(app): State<App>,
    headers: HeaderMap,
    Path((company, provider, account)): Path<(String, String, String)>,
    body: Result<Json<AdmissionRequest>, JsonRejection>,
) -> Result<impl IntoResponse, Error> {
    let actor = principal(&app, &headers).await?;
    validate(&company, &provider, &account)?;
    let Json(request) = body.map_err(|_| Fault::Invalid)?;
    let record = app
        .store
        .account(&company, &provider, &account, &actor, true)
        .await?;
    let decision = match record.status.as_str() {
        "active" => "allow",
        "paused" => "paused",
        "unknown" => "unknown",
        _ => return Err(Fault::Unavailable.into()),
    };
    Ok(Json(json!({"account": record, "decision": decision,
        "operation_id": request.operation_id, "operation_kind": request.operation_kind})))
}

async fn publish(
    State(app): State<App>,
    headers: HeaderMap,
    Path(company): Path<String>,
    body: Result<Json<Projection>, JsonRejection>,
) -> Result<StatusCode, Error> {
    let actor = principal(&app, &headers).await?;
    if !app.auth.may_publish(&company, &actor) {
        return Err(Fault::Scope.into());
    }
    let Json(projection) = body.map_err(|_| Fault::Invalid)?;
    app.store.publish(&company, projection).await?;
    Ok(StatusCode::NO_CONTENT)
}
