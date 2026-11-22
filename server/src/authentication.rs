//! Verified native identity evidence. Deployment bindings supply application roles.
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use connectrpc::{ConnectError, RequestContext};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
type Result<T> = std::result::Result<T, ConnectError>;
fn bad() -> ConnectError {
    ConnectError::unauthenticated("invalid or expired credential")
}
fn unavailable() -> ConnectError {
    ConnectError::unavailable("identity provider unavailable")
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Oidc {
    pub issuer: String,
    pub audience: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Remote {
    pub url: String,
    pub bearer_env: String,
}
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub oidc: Option<Oidc>,
    pub introspection: Option<Remote>,
    pub bindings: BTreeMap<String, Value>,
    pub sessions: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub binding: String,
    pub method: String,
    pub authenticated_at: Option<DateTime<Utc>>,
    pub expires: Option<DateTime<Utc>>,
    pub actions: Option<Vec<String>>,
    pub tenants: Option<Vec<Vec<u8>>>,
    pub sets: Option<Vec<Vec<u8>>>,
}
impl Evidence {
    fn new(binding: String, method: &str) -> Self {
        Self {
            binding,
            method: method.into(),
            authenticated_at: None,
            expires: None,
            actions: None,
            tenants: None,
            sets: None,
        }
    }
    pub fn live(&self) -> Result<()> {
        if self.expires.is_some_and(|v| v <= Utc::now()) {
            Err(bad())
        } else {
            Ok(())
        }
    }
}
struct Keys {
    set: JwkSet,
    loaded: Instant,
}
pub struct Verifier {
    client: Client,
    oidc: Option<Oidc>,
    remote: Option<(Url, String)>,
    keys: Mutex<Option<Keys>>,
}
fn endpoint(source: &str) -> std::result::Result<Url, String> {
    let u = Url::parse(source).map_err(|e| e.to_string())?;
    if !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
        || u.query().is_some()
    {
        return Err("invalid identity provider URL".into());
    }
    if u.scheme() != "https"
        && !(u.scheme() == "http"
            && matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        return Err("identity provider requires HTTPS".into());
    }
    Ok(u)
}
async fn read<T: DeserializeOwned>(mut response: reqwest::Response, limit: usize) -> Result<T> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| unavailable())
}
impl Verifier {
    pub fn new(config: &Config) -> std::result::Result<Self, String> {
        if let Some(o) = &config.oidc {
            endpoint(&o.issuer)?;
            if o.audience.is_empty() {
                return Err("OIDC requires audience".into());
            }
        }
        let remote = config
            .introspection
            .as_ref()
            .map(|r| {
                let u = endpoint(&format!(
                    "{}/payday.TokenService/Introspect",
                    r.url.trim_end_matches('/')
                ))?;
                let token = std::env::var(&r.bearer_env)
                    .map_err(|_| "missing introspection service credential")?;
                if token.is_empty() {
                    return Err("empty introspection service credential".to_string());
                }
                Ok((u, token))
            })
            .transpose()?;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            oidc: config.oidc.clone(),
            remote,
            keys: Default::default(),
        })
    }
    async fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        read(response, 1 << 20).await
    }
    async fn jwt(&self, o: &Oidc, token: &str) -> Result<Evidence> {
        let h = decode_header(token).map_err(|_| bad())?;
        if !matches!(h.alg, Algorithm::RS256 | Algorithm::ES256)
            || !matches!(h.typ.as_deref(), Some("at+jwt" | "application/at+jwt"))
        {
            return Err(bad());
        }
        let kid = h
            .kid
            .as_deref()
            .filter(|s| !s.is_empty() && s.len() < 256)
            .ok_or_else(bad)?;
        let mut keys = self.keys.lock().await;
        if keys.as_ref().is_none_or(|cache| {
            cache.loaded.elapsed() > Duration::from_secs(300)
                || (cache.set.find(kid).is_none()
                    && cache.loaded.elapsed() > Duration::from_secs(1))
        }) {
            #[derive(Deserialize)]
            struct Discovery {
                issuer: String,
                jwks_uri: String,
            }
            let discovery: Discovery = self
                .get(
                    endpoint(&format!(
                        "{}/.well-known/openid-configuration",
                        o.issuer.trim_end_matches('/')
                    ))
                    .map_err(|_| unavailable())?,
                )
                .await?;
            if discovery.issuer != o.issuer {
                return Err(unavailable());
            }
            let jwks = endpoint(&discovery.jwks_uri).map_err(|_| unavailable())?;
            if o.issuer.starts_with("https://") && jwks.scheme() != "https" {
                return Err(unavailable());
            }
            *keys = Some(Keys {
                set: self.get(jwks).await?,
                loaded: Instant::now(),
            });
        }
        let jwk = keys
            .as_ref()
            .and_then(|v| v.set.find(kid))
            .ok_or_else(bad)?;
        use jsonwebtoken::jwk::{KeyOperations, PublicKeyUse};
        if jwk
            .common
            .public_key_use
            .as_ref()
            .is_some_and(|u| u != &PublicKeyUse::Signature)
            || jwk
                .common
                .key_operations
                .as_ref()
                .is_some_and(|ops| !ops.contains(&KeyOperations::Verify))
            || jwk
                .common
                .key_algorithm
                .is_some_and(|alg| Algorithm::try_from(alg).ok() != Some(h.alg))
        {
            return Err(bad());
        }
        let key = DecodingKey::from_jwk(jwk).map_err(|_| bad())?;
        drop(keys);
        let mut validation = Validation::new(h.alg);
        validation.leeway = 0;
        validation.validate_nbf = true;
        validation.set_issuer(&[&o.issuer]);
        validation.set_audience(&[&o.audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        let c = decode::<Value>(token, &key, &validation)
            .map_err(|_| bad())?
            .claims;
        let timestamp = |k: &str| {
            c[k].as_i64()
                .and_then(|n| DateTime::from_timestamp(n, 0))
                .ok_or_else(bad)
        };
        for k in ["sub", "client_id", "jti"] {
            if c[k].as_str().is_none_or(str::is_empty) {
                return Err(bad());
            }
        }
        let issued = timestamp("iat")?;
        let expires = timestamp("exp")?;
        if issued > Utc::now() || expires <= issued {
            return Err(bad());
        }
        let mut id = Evidence::new(format!("oidc:{}", c["sub"].as_str().unwrap()), "oidc");
        id.expires = Some(expires);
        id.authenticated_at = c
            .get("auth_time")
            .map(|_| timestamp("auth_time"))
            .transpose()?;
        if id.authenticated_at.is_some_and(|t| t > issued) {
            return Err(bad());
        }
        Ok(id)
    }
    async fn introspect(&self, url: &Url, application: &str, token: &str) -> Result<Evidence> {
        let reply = self
            .client
            .post(url.clone())
            .bearer_auth(application)
            .header("Connect-Protocol-Version", "1")
            .json(&json!({"token":token}))
            .send()
            .await
            .map_err(|_| unavailable())?;
        if !reply.status().is_success() {
            let err: Value = read(reply, 32768).await?;
            return Err(if err["code"] == "not_found" {
                bad()
            } else {
                unavailable()
            });
        }
        let c: Value = read(reply, 65536).await?;
        let id = |v: &Value| {
            STANDARD
                .decode(v.as_str().ok_or_else(unavailable)?)
                .ok()
                .filter(|b| b.len() == 16)
                .ok_or_else(unavailable)
        };
        let text = |k: &str| c[k].as_str().filter(|s| !s.is_empty());
        let binding = match (text("tenantId"), text("id"), text("tenant"), text("alias")) {
            (Some(_), Some(_), None, None) => format!(
                "remote:{}/{}",
                hex::encode(id(&c["tenantId"])?),
                hex::encode(id(&c["id"])?)
            ),
            (None, None, Some(t), Some(a)) if !t.contains('/') && !a.contains('/') => {
                format!("remote:{t}/{a}")
            }
            _ => return Err(unavailable()),
        };
        let mut e = Evidence::new(binding, "token");
        let g = &c["grant"];
        let array = |key: &str| {
            g.get(key)
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
        };
        let ids = |flag: &str, k: &str| {
            if g[flag] == true {
                Ok(None)
            } else {
                array(k)
                    .iter()
                    .map(id)
                    .collect::<Result<Vec<_>>>()
                    .map(Some)
            }
        };
        e.tenants = ids("anyTenant", "tenants")?;
        e.sets = ids("anySet", "sets")?;
        if g["anyAction"] != true {
            e.actions = Some(
                array("actions")
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned).ok_or_else(unavailable))
                    .collect::<Result<_>>()?,
            )
        }
        let stamp = |k: &str| {
            c.get(k)
                .map(|v| {
                    v.as_str()
                        .ok_or_else(unavailable)?
                        .parse::<DateTime<Utc>>()
                        .map_err(|_| unavailable())
                })
                .transpose()
        };
        e.authenticated_at = stamp("dateAuthenticated")?;
        e.expires = stamp("dateExpires")?;
        e.live()?;
        Ok(e)
    }
    pub async fn resolve(&self, ctx: &RequestContext) -> Result<Option<Evidence>> {
        if let Some(raw) = ctx.header("authorization") {
            let token = raw
                .to_str()
                .ok()
                .and_then(|s| s.split_once(' '))
                .filter(|(s, t)| {
                    s.eq_ignore_ascii_case("bearer") && !t.is_empty() && t.len() < 16384
                })
                .map(|(_, t)| t)
                .ok_or_else(bad)?;
            if let Some(o) = &self.oidc
                && (self.remote.is_none() || token.matches('.').count() == 2)
            {
                return self.jwt(o, token).await.map(Some);
            }
            if let Some((u, t)) = &self.remote {
                return self.introspect(u, t, token).await.map(Some);
            }
            return Err(bad());
        }
        Ok(ctx
            .extensions()
            .get::<crate::PeerCertificate>()
            .map(|p| Evidence::new(format!("mtls:{}", p.0), "mtls")))
    }
}
