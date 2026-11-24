// Copied into the external consumer by scripts/test-composition.sh. That crate
// has no Database type, paydirt-store dependency, or SQL driver.
#[path = "../generated/messages/mod.rs"]
#[rustfmt::skip]
pub mod proto;
#[path = "../generated/services/mod.rs"]
#[rustfmt::skip]
pub mod services;
pub mod server {
    include!("../generated/server.rs");
}

#[cfg(test)]
mod tests {
    use super::*;
    use connectrpc::{
        ConnectError, Encodable, RequestContext, Response, ServiceRequest, ServiceResult,
    };
    use proto::app::*;
    use server::Server;
    use services::app::*;
    use std::sync::{Arc, Mutex};

    const ID: [u8; 16] = [0, 0, 0, 0, 0, 0, 0x80, 0, 0x80, 77, 0, 0, 0, 0, 0, 1];
    const TENANT: [u8; 16] = [0, 0, 0, 0, 0, 0, 0x80, 0, 0x80, 1, 0, 0, 0, 0, 0, 1];

    #[derive(Clone)]
    struct Memory(Arc<Mutex<Vec<&'static str>>>);
    impl<N: Server> server::Layer<N> for Memory {
        fn app_reading(&self, next: &N) -> impl ReadingService {
            ReadingServiceOverlay {
                next: next.app_reading(),
                layer: self.clone(),
            }
        }
        fn app_projection(&self, next: &N) -> impl ProjectionService {
            ProjectionServiceOverlay {
                next: next.app_projection(),
                layer: self.clone(),
            }
        }
    }
    #[allow(refining_impl_trait)]
    impl<N: ReadingService> ReadingServiceLayer<N> for Memory {
        async fn get(
            &self,
            _: &N,
            _: RequestContext,
            req: ServiceRequest<'_, ReadingGetRequest>,
        ) -> ServiceResult<Reading> {
            self.0.lock().unwrap().push("memory");
            if req.to_owned_message().r#ref.key != Some(reading_ref::Key::Id(ID.to_vec())) {
                return Err(ConnectError::not_found("reading"));
            }
            Response::ok(Reading {
                id: ID.to_vec(),
                tenant: Some(Tenant {
                    id: TENANT.to_vec(),
                    ..Default::default()
                })
                .into(),
                value: "memory".into(),
                ..Default::default()
            })
        }
    }
    #[allow(refining_impl_trait)]
    impl<N: ProjectionService> ProjectionServiceLayer<N> for Memory {
        async fn get(
            &self,
            _: &N,
            _: RequestContext,
            _: ServiceRequest<'_, ProjectionGetRequest>,
        ) -> ServiceResult<Projection> {
            Response::ok(Projection {
                id: ID.to_vec(),
                value: "projection".into(),
                ..Default::default()
            })
        }
    }

    #[derive(Clone)]
    struct Policy(Arc<Mutex<Vec<&'static str>>>);
    impl<N: Server> server::Layer<N> for Policy {
        fn app_reading(&self, next: &N) -> impl ReadingService {
            ReadingServiceOverlay {
                next: next.app_reading(),
                layer: self.clone(),
            }
        }
    }
    #[allow(refining_impl_trait)]
    impl<N: ReadingService> ReadingServiceLayer<N> for Policy {
        async fn get<'a>(
            &'a self,
            next: &'a N,
            ctx: RequestContext,
            req: ServiceRequest<'_, ReadingGetRequest>,
        ) -> ServiceResult<impl Encodable<Reading> + Send + use<'a, N>> {
            self.0.lock().unwrap().push("policy");
            if ctx
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                != Some("Bearer allowed")
            {
                return Err(ConnectError::permission_denied("policy"));
            }
            next.get(ctx, req).await
        }
    }

    async fn call(
        router: connectrpc::Router,
        method: &str,
        allowed: bool,
    ) -> (http::StatusCode, serde_json::Value) {
        use http_body_util::{BodyExt, Full};
        use tower::ServiceExt;
        let mut req = http::Request::builder()
            .method("POST")
            .uri(method)
            .header("content-type", "application/json")
            .header("connect-protocol-version", "1");
        if allowed {
            req = req.header("authorization", "Bearer allowed");
        }
        let req = req
            .body(Full::new(bytes::Bytes::from_static(
                br#"{"ref":{"id":"AAAAAAAAgACATQAAAAAAAQ=="}}"#,
            )))
            .unwrap();
        let res = connectrpc::ConnectRpcService::new(router)
            .oneshot(req)
            .await
            .unwrap();
        let status = res.status();
        let body =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        (status, body)
    }

    #[tokio::test]
    async fn entity_layers_compose_without_a_database() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let stack = Arc::new(
            server::Unimplemented
                .layer(Memory(trace.clone()))
                .layer(Policy(trace.clone())),
        );
        let router = || server::mount(connectrpc::Router::new(), &stack);
        let (status, body) = call(router(), "/app.ReadingService/Get", true).await;
        assert_eq!(status, 200);
        assert_eq!(body["value"], "memory");
        assert_eq!(body["tenant"]["id"], "AAAAAAAAgACAAQAAAAAAAQ==");
        assert_eq!(*trace.lock().unwrap(), ["policy", "memory"]);
        trace.lock().unwrap().clear();
        let (status, _) = call(router(), "/app.ReadingService/Get", false).await;
        assert_eq!(status, 403);
        assert_eq!(*trace.lock().unwrap(), ["policy"]);
        // Forwarded methods end at Unimplemented; no implicit SQL fallback.
        assert_eq!(
            call(router(), "/app.ReadingService/Refresh", true).await.0,
            501
        );
        assert_eq!(call(router(), "/app.TenantService/Get", true).await.0, 501);
        // The same stack also supports an ordinary computed projection.
        assert_eq!(
            call(router(), "/app.ProjectionService/Get", true).await.1["value"],
            "projection"
        );
    }
}
