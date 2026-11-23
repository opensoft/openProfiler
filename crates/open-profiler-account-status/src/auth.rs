use crate::model::{self, Principal};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

const JWKS_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityConfig {
    pub issuer: String,
    pub audience: String,
    pub jwks_url: String,
    pub publishers: BTreeMap<String, Vec<Principal>>,
}

#[derive(Clone)]
pub struct Auth {
    config: Arc<IdentityConfig>,
    keys: Arc<RwLock<(Instant, JwkSet)>>,
    client: Client,
    jwks: Url,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    Invalid,
    Unavailable,
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
    azp: Option<String>,
    client_id: Option<String>,
}

pub fn endpoint(value: &str, fixture: bool) -> Result<Url, &'static str> {
    let url = Url::parse(value).map_err(|_| "invalid identity endpoint")?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(
            (url.scheme(), fixture && loopback),
            ("https", _) | ("http", true)
        )
    {
        return Err("invalid identity endpoint");
    }
    Ok(url)
}

impl Auth {
    pub async fn connect(config: IdentityConfig, fixture: bool) -> Result<Self, &'static str> {
        let issuer = endpoint(&config.issuer, fixture)?;
        let jwks = endpoint(&config.jwks_url, fixture)?;
        if issuer.origin() != jwks.origin()
            || config.audience.is_empty()
            || !config.publishers.iter().all(|(company, entries)| {
                model::key(company) && entries.iter().all(Principal::valid)
            })
        {
            return Err("invalid identity configuration");
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "identity client unavailable")?;
        let keys = Self::fetch(&client, jwks.clone()).await?;
        Ok(Self {
            config: Arc::new(config),
            keys: Arc::new(RwLock::new((Instant::now() + JWKS_TTL, keys))),
            client,
            jwks,
        })
    }

    async fn fetch(client: &Client, url: Url) -> Result<JwkSet, &'static str> {
        let mut response = client
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| "identity keys unavailable")?;
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "identity keys unavailable")?
        {
            if body.len() + chunk.len() > 128 * 1024 {
                return Err("identity keys unavailable");
            }
            body.extend_from_slice(&chunk);
        }
        let parsed: JwkSet = serde_json::from_slice(&body).map_err(|_| "invalid identity keys")?;
        if parsed.keys.is_empty() {
            return Err("invalid identity keys");
        }
        Ok(parsed)
    }

    pub fn refresh_in_background(&self) {
        let auth = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                if let Ok(keys) = Self::fetch(&auth.client, auth.jwks.clone()).await {
                    *auth.keys.write().await = (Instant::now() + JWKS_TTL, keys);
                }
            }
        });
    }

    pub async fn ready(&self) -> bool {
        let expires_at = self.keys.read().await.0;
        Instant::now() < expires_at
    }

    pub async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        if token.len() > 16 * 1024 {
            return Err(AuthError::Invalid);
        }
        let header = decode_header(token).map_err(|_| AuthError::Invalid)?;
        if header.alg != Algorithm::RS256 {
            return Err(AuthError::Invalid);
        }
        let kid = header.kid.as_deref().ok_or(AuthError::Invalid)?;
        let guard = self.keys.read().await;
        let mut candidates = guard
            .1
            .keys
            .iter()
            .filter(|jwk| jwk.common.key_id.as_deref() == Some(kid));
        let jwk = candidates.next().ok_or(AuthError::Invalid)?;
        if candidates.next().is_some() {
            return Err(AuthError::Invalid);
        }
        let key = DecodingKey::from_jwk(jwk).map_err(|_| AuthError::Invalid)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[&self.config.audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = 0;
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| AuthError::Invalid)?
            .claims;
        if let (Some(azp), Some(client_id)) = (&claims.azp, &claims.client_id) {
            if azp != client_id {
                return Err(AuthError::Invalid);
            }
        }
        let principal = Principal {
            subject: claims.sub,
            client_id: claims.azp.or(claims.client_id).ok_or(AuthError::Invalid)?,
        };
        if !principal.valid() {
            return Err(AuthError::Invalid);
        }
        // Stale keys can identify a rejected token but must never grant access.
        if Instant::now() >= guard.0 {
            return Err(AuthError::Unavailable);
        }
        Ok(principal)
    }

    pub fn may_publish(&self, company: &str, principal: &Principal) -> bool {
        self.config
            .publishers
            .get(company)
            .is_some_and(|p| p.contains(principal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use rand::{rngs::StdRng, SeedableRng};
    use rsa::{
        pkcs8::{EncodePrivateKey, LineEnding},
        traits::PublicKeyParts,
        RsaPrivateKey,
    };
    use serde_json::json;

    #[tokio::test]
    async fn stale_jwks_rejects_otherwise_valid_tokens_as_unavailable() {
        let private = RsaPrivateKey::new(&mut StdRng::seed_from_u64(1), 2048).unwrap();
        let keys = serde_json::from_value(json!({"keys":[{"kty":"RSA","alg":"RS256",
            "kid":"fixture", "n":URL_SAFE_NO_PAD.encode(private.n().to_bytes_be()),
            "e":URL_SAFE_NO_PAD.encode(private.e().to_bytes_be())}]}))
        .unwrap();
        let issuer = "https://issuer.example";
        let jwks_url = format!("{issuer}/jwks");
        let auth = Auth {
            config: Arc::new(IdentityConfig {
                issuer: issuer.into(),
                audience: "openprofiler".into(),
                jwks_url: jwks_url.clone(),
                publishers: BTreeMap::new(),
            }),
            keys: Arc::new(RwLock::new((Instant::now() + JWKS_TTL, keys))),
            client: Client::new(),
            jwks: Url::parse(&jwks_url).unwrap(),
        };
        let pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
        let key = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("fixture".into());
        let token = |audience| {
            encode(
                &header,
                &json!({"iss":issuer,"aud":audience,"sub":"operator","azp":"desktop",
                    "exp":model::now()+600}),
                &key,
            )
            .unwrap()
        };
        let valid = token("openprofiler");
        let wrong_audience = token("other");
        assert!(auth.ready().await);
        assert_eq!(auth.authenticate(&valid).await.unwrap().subject, "operator");

        auth.keys.write().await.0 = Instant::now();
        assert!(!auth.ready().await);
        assert_eq!(
            auth.authenticate(&valid).await.unwrap_err(),
            AuthError::Unavailable
        );
        for token in [wrong_audience.as_str(), "not-a-token"] {
            assert_eq!(
                auth.authenticate(token).await.unwrap_err(),
                AuthError::Invalid
            );
        }
    }
}
