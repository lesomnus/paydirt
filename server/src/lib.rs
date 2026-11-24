//! Native deployment listeners for generated applications.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request},
    middleware::{self, Next},
    response::Response,
};
use connectrpc::{ConnectRpcService, DeadlinePolicy, Limits};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use opentelemetry::{
    KeyValue,
    propagation::{Extractor, TextMapPropagator},
    trace::{Span, SpanKind, Tracer, TracerProvider},
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, propagation::TraceContextPropagator, trace::SdkTracerProvider};
use paydirt::spin::Stop;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    error::Error, fs::File, io::BufReader, net::SocketAddr, path::PathBuf, sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
    task::JoinSet,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{self, server::WebPkiClientVerifier},
};
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Debug)]
pub struct PeerCertificate(pub String);
#[derive(Deserialize)]
pub struct Tls {
    pub cert: PathBuf,
    pub key: PathBuf,
    pub client_ca: Option<PathBuf>,
}
#[derive(Deserialize)]
pub struct Web {
    pub listen: SocketAddr,
    pub origins: Vec<String>,
    pub tls: Option<Tls>,
}
#[derive(Deserialize, Default)]
pub struct Serving {
    pub tls: Option<Tls>,
    pub web: Option<Web>,
    pub deadline_ms: Option<u64>,
    pub max_message_bytes: Option<usize>,
    pub otlp_traces_endpoint: Option<String>,
}
fn tls(spec: Tls) -> Result<TlsAcceptor> {
    let cert = rustls_pemfile::certs(&mut BufReader::new(File::open(spec.cert)?))
        .collect::<std::io::Result<Vec<_>>>()?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(File::open(spec.key)?))?
        .ok_or("missing TLS private key")?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?;
    let builder = if let Some(path) = spec.client_ca {
        let mut roots = rustls::RootCertStore::empty();
        for der in rustls_pemfile::certs(&mut BufReader::new(File::open(path)?)) {
            roots.add(der?)?;
        }
        let verifier =
            WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider).build()?;
        builder.with_client_cert_verifier(verifier)
    } else {
        builder.with_no_client_auth()
    };
    let mut config = builder.with_single_cert(cert, key)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(TlsAcceptor::from(Arc::new(config)))
}

struct Headers<'a>(&'a HeaderMap);
impl Extractor for Headers<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key)?.to_str().ok()
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}
async fn traced(provider: SdkTracerProvider, request: Request<Body>, next: Next) -> Response {
    let parent = TraceContextPropagator::new().extract(&Headers(request.headers()));
    let tracer = provider.tracer("paydirt.rpc");
    let mut span = tracer
        .span_builder(request.uri().path().to_owned())
        .with_kind(SpanKind::Server)
        .start_with_context(&tracer, &parent);
    let response = next.run(request).await;
    span.set_attribute(KeyValue::new(
        "http.response.status_code",
        i64::from(response.status().as_u16()),
    ));
    span.end();
    response
}

struct Listener {
    socket: TcpListener,
    tls: Option<TlsAcceptor>,
    routes: Router,
}
pub struct Host {
    listeners: Vec<Listener>,
    telemetry: Option<SdkTracerProvider>,
}
pub use axum;
impl Host {
    pub async fn bind(
        addr: SocketAddr,
        spec: Serving,
        service_name: &str,
        router: connectrpc::Router,
    ) -> Result<Self> {
        Self::bind_with_routes(addr, spec, service_name, router, Router::new()).await
    }
    /// Mount application HTTP handlers on the same listener and CORS stack as Connect and gRPC.
    pub async fn bind_with_routes(
        addr: SocketAddr,
        spec: Serving,
        service_name: &str,
        router: connectrpc::Router,
        app: Router,
    ) -> Result<Self> {
        let primary = spec.tls.map(tls).transpose()?;
        let web = spec
            .web
            .map(|web| -> Result<_> {
                let mut origins = Vec::new();
                for origin in web.origins {
                    let uri: axum::http::Uri = origin.parse()?;
                    if !matches!(uri.scheme_str(), Some("http" | "https"))
                        || uri.authority().is_none()
                        || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
                    {
                        return Err("web origin must be an HTTP(S) origin".into());
                    }
                    origins.push(HeaderValue::from_str(&origin)?);
                }
                let cors = CorsLayer::new()
                    .allow_origin(origins)
                    .allow_methods([Method::POST, Method::GET, Method::OPTIONS])
                    .allow_headers(tower_http::cors::AllowHeaders::mirror_request())
                    .expose_headers([
                        HeaderName::from_static("grpc-status"),
                        HeaderName::from_static("grpc-message"),
                        HeaderName::from_static("grpc-status-details-bin"),
                    ]);
                Ok((web.listen, web.tls.map(tls).transpose()?, cors))
            })
            .transpose()?;
        let mut service = ConnectRpcService::new(router);
        if let Some(ms) = spec.deadline_ms {
            if ms == 0 {
                return Err("deadline_ms must be positive".into());
            }
            let duration = Duration::from_millis(ms);
            service = service.with_deadline_policy(
                DeadlinePolicy::new()
                    .with_max(duration)
                    .with_default_timeout(duration),
            );
        }
        if let Some(bytes) = spec.max_message_bytes {
            if bytes == 0 {
                return Err("max_message_bytes must be positive".into());
            }
            service = service.with_limits(
                Limits::default()
                    .max_request_body_size(bytes)
                    .max_message_size(bytes),
            );
        }
        let mut routes = app.fallback_service(service);
        let telemetry = if let Some(endpoint) = spec.otlp_traces_endpoint {
            if service_name.is_empty() {
                return Err("telemetry requires a service name".into());
            }
            let uri: axum::http::Uri = endpoint.parse()?;
            if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.authority().is_none() {
                return Err("invalid OTLP traces endpoint".into());
            }
            let resource = Resource::builder()
                .with_service_name(service_name.to_owned())
                .build();
            let provider = tokio::task::spawn_blocking(move || -> Result<_> {
                let exporter = opentelemetry_otlp::SpanExporter::builder()
                    .with_http()
                    .with_endpoint(endpoint)
                    .build()?;
                Ok(SdkTracerProvider::builder()
                    .with_resource(resource)
                    .with_batch_exporter(exporter)
                    .build())
            })
            .await??;
            let p = provider.clone();
            routes = routes.layer(middleware::from_fn(move |req, next| {
                traced(p.clone(), req, next)
            }));
            Some(provider)
        } else {
            None
        };
        let mut listeners = vec![Listener {
            socket: TcpListener::bind(addr).await?,
            tls: primary,
            routes: routes.clone(),
        }];
        if let Some((addr, tls, cors)) = web {
            listeners.push(Listener {
                socket: TcpListener::bind(addr).await?,
                tls,
                routes: routes.layer(cors),
            });
        }
        Ok(Self {
            listeners,
            telemetry,
        })
    }
    /// Actual address of the primary listener, including the assigned port
    /// when `bind` was called with port zero. Additional web listeners are
    /// independent of this address.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listeners[0].socket.local_addr()
    }

    pub async fn serve(self, stop: Stop) -> Result<()> {
        let mut tasks = JoinSet::new();
        for l in self.listeners {
            tasks.spawn(l.serve(stop.clone()));
        }
        let mut err = None;
        while let Some(res) = tasks.join_next().await {
            if let Err(e) = res
                .map_err(|e| -> Box<dyn Error + Send + Sync> { Box::new(e) })
                .and_then(|r| r)
            {
                err = Some(e);
                break;
            }
        }
        tasks.shutdown().await;
        if let Some(provider) = self.telemetry {
            tokio::task::spawn_blocking(move || provider.shutdown()).await??;
        }
        err.map_or(Ok(()), Err)
    }
}
impl Listener {
    async fn serve(self, stop: Stop) -> Result<()> {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                biased;
                _=stop.cancelled()=>break,
                Some(_)=connections.join_next()=>{},
                accepted=self.socket.accept()=>{
                    let (socket,remote)=accepted?;let tls=self.tls.clone();let routes=self.routes.clone();let stop=stop.clone();
                    connections.spawn(async move{
                        if let Some(tls)=tls{
                            let stream=tokio::select!{_=stop.cancelled()=>return,v=tokio::time::timeout(Duration::from_secs(10),tls.accept(socket))=>match v{Ok(Ok(s))=>s,_=>return}};
                            let peer=stream.get_ref().1.peer_certificates().and_then(|c|c.first()).map(|c|PeerCertificate(hex::encode(Sha256::digest(c.as_ref()))));
                            serve_connection(stream,routes,stop,peer,remote).await;
                        }else{serve_connection(socket,routes,stop,None,remote).await;}
                    });
                }
            }
        }
        while connections.join_next().await.is_some() {}
        Ok(())
    }
}
async fn serve_connection(
    io: impl AsyncRead + AsyncWrite + Unpin + Send + 'static,
    routes: Router,
    stop: Stop,
    peer: Option<PeerCertificate>,
    remote: SocketAddr,
) {
    let http = hyper::service::service_fn(move |mut request: Request<hyper::body::Incoming>| {
        request.extensions_mut().insert(remote);
        if let Some(peer) = &peer {
            request.extensions_mut().insert(peer.clone());
        }
        routes.clone().oneshot(request.map(Body::new))
    });
    let builder = Builder::new(TokioExecutor::new());
    let connection = builder.serve_connection(TokioIo::new(io), http);
    tokio::pin!(connection);
    tokio::select! {
        _=&mut connection=>{},
        _=stop.cancelled()=>{connection.as_mut().graceful_shutdown();let _=tokio::time::timeout(Duration::from_secs(5),connection).await;}
    }
}

#[derive(Clone, Copy, Deserialize)]
pub struct Rate {
    pub per_second: f64,
    pub burst: u32,
}
struct Bucket {
    tokens: f64,
    updated: std::time::Instant,
}
struct Buckets {
    last_sweep: std::time::Instant,
    tenants: std::collections::HashMap<Vec<u8>, Bucket>,
}
pub struct Limiter {
    rate: Rate,
    state: std::sync::Mutex<Buckets>,
}
impl Limiter {
    pub fn new(rate: Rate) -> Result<Self> {
        if !rate.per_second.is_finite() || rate.per_second <= 0. || rate.burst == 0 {
            return Err("rate requires positive per_second and burst".into());
        }
        Ok(Self {
            rate,
            state: std::sync::Mutex::new(Buckets {
                last_sweep: std::time::Instant::now(),
                tenants: Default::default(),
            }),
        })
    }
    pub fn charge(
        &self,
        key: &[u8],
        items: usize,
    ) -> std::result::Result<(), connectrpc::ConnectError> {
        let cost = items.max(1) as f64;
        let cap = self.rate.burst as f64;
        let rate = self.rate.per_second;
        if cost > cap {
            return Err(connectrpc::ConnectError::resource_exhausted(
                "batch exceeds limit; ask for less",
            ));
        }
        let now = std::time::Instant::now();
        let mut state = self.state.lock().unwrap();
        if now.duration_since(state.last_sweep) >= Duration::from_secs(60) {
            state
                .tenants
                .retain(|_, v| v.tokens + now.duration_since(v.updated).as_secs_f64() * rate < cap);
            state.last_sweep = now;
        }
        let bucket = state.tenants.entry(key.to_owned()).or_insert(Bucket {
            tokens: cap,
            updated: now,
        });
        bucket.tokens =
            (bucket.tokens + now.duration_since(bucket.updated).as_secs_f64() * rate).min(cap);
        bucket.updated = now;
        if bucket.tokens < cost {
            return Err(connectrpc::ConnectError::resource_exhausted(
                "request rate exceeded",
            ));
        }
        bucket.tokens -= cost;
        Ok(())
    }
}

pub mod authentication;
