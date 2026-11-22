use crate::transport::Database;
use ::connectrpc::{RequestContext, Response, Router, ServiceRequest, ServiceResult};
mod sink_app_cell {
    use super::*;
    pub struct Sink(pub Database);
    #[allow(refining_impl_trait)]
    impl<
        N: crate::services::r#app::CellService,
    > crate::services::r#app::CellServiceLayer<N> for Sink {
        async fn add(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#CellAddRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Cell> {
            Response::ok(
                self.0.rpc(&ctx, "app.Cell", "Add", &request.to_owned_message()).await?,
            )
        }
        async fn apply(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#CellApplyRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Cell> {
            Response::ok(
                self.0.rpc(&ctx, "app.Cell", "Apply", &request.to_owned_message()).await?,
            )
        }
        async fn erase(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#CellRef>,
        ) -> ServiceResult<crate::proto::r#app::r#CellEraseResponse> {
            Response::ok(
                self.0.rpc(&ctx, "app.Cell", "Erase", &request.to_owned_message()).await?,
            )
        }
        async fn get(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#CellGetRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Cell> {
            Response::ok(
                self.0.rpc(&ctx, "app.Cell", "Get", &request.to_owned_message()).await?,
            )
        }
        async fn patch(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#CellPatchRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Cell> {
            Response::ok(
                self.0.rpc(&ctx, "app.Cell", "Patch", &request.to_owned_message()).await?,
            )
        }
    }
    impl<
        N: crate::services::r#app::CellService,
    > crate::services::r#app::CellServiceLayer<N> for Gate {
        fn add<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::CellAddRequest>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Cell",
                        "Add",
                        self.public.contains(&"/app.CellService/Add"),
                        false,
                    )?;
                next.add(ctx, request).await
            }
        }
        fn get<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::CellGetRequest>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Cell",
                        "Get",
                        self.public.contains(&"/app.CellService/Get"),
                        false,
                    )?;
                next.get(ctx, request).await
            }
        }
        fn patch<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::CellPatchRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Cell",
                        "Patch",
                        self.public.contains(&"/app.CellService/Patch"),
                        false,
                    )?;
                next.patch(ctx, request).await
            }
        }
        fn apply<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::CellApplyRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Cell",
                        "Apply",
                        self.public.contains(&"/app.CellService/Apply"),
                        false,
                    )?;
                next.apply(ctx, request).await
            }
        }
        fn erase<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::CellRef>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::CellEraseResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Cell",
                        "Erase",
                        self.public.contains(&"/app.CellService/Erase"),
                        false,
                    )?;
                next.erase(ctx, request).await
            }
        }
    }
    impl<
        N: crate::services::r#app::CellService,
    > crate::services::r#app::CellServiceLayer<N> for Audit {
        fn add<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::CellAddRequest>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.add(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn get<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::CellGetRequest>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.get(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn patch<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::CellPatchRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.patch(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn apply<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::CellApplyRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.apply(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn erase<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::CellRef>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::CellEraseResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.erase(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
    }
}
mod sink_app_robot {
    use super::*;
    pub struct Sink(pub Database);
    #[allow(refining_impl_trait)]
    impl<
        N: crate::services::r#app::RobotService,
    > crate::services::r#app::RobotServiceLayer<N> for Sink {
        async fn add(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#RobotAddRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Robot> {
            Response::ok(
                self.0.rpc(&ctx, "app.Robot", "Add", &request.to_owned_message()).await?,
            )
        }
        async fn add_many(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#RobotAddManyRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#RobotAddManyResponse> {
            Response::ok(
                self
                    .0
                    .rpc(&ctx, "app.Robot", "AddMany", &request.to_owned_message())
                    .await?,
            )
        }
        async fn apply(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#RobotApplyRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Robot> {
            Response::ok(
                self
                    .0
                    .rpc(&ctx, "app.Robot", "Apply", &request.to_owned_message())
                    .await?,
            )
        }
        async fn erase(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#RobotRef>,
        ) -> ServiceResult<crate::proto::r#app::r#RobotEraseResponse> {
            Response::ok(
                self
                    .0
                    .rpc(&ctx, "app.Robot", "Erase", &request.to_owned_message())
                    .await?,
            )
        }
        async fn get(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#RobotGetRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Robot> {
            Response::ok(
                self.0.rpc(&ctx, "app.Robot", "Get", &request.to_owned_message()).await?,
            )
        }
        async fn patch(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#RobotPatchRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Robot> {
            Response::ok(
                self
                    .0
                    .rpc(&ctx, "app.Robot", "Patch", &request.to_owned_message())
                    .await?,
            )
        }
    }
    impl<
        N: crate::services::r#app::RobotService,
    > crate::services::r#app::RobotServiceLayer<N> for Gate {
        fn add<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotAddRequest>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Robot",
                        "Add",
                        self.public.contains(&"/app.RobotService/Add"),
                        false,
                    )?;
                next.add(ctx, request).await
            }
        }
        fn add_many<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotAddManyRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::RobotAddManyResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Robot",
                        "AddMany",
                        self.public.contains(&"/app.RobotService/AddMany"),
                        false,
                    )?;
                next.add_many(ctx, request).await
            }
        }
        fn get<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotGetRequest>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Robot",
                        "Get",
                        self.public.contains(&"/app.RobotService/Get"),
                        false,
                    )?;
                next.get(ctx, request).await
            }
        }
        fn patch<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotPatchRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Robot",
                        "Patch",
                        self.public.contains(&"/app.RobotService/Patch"),
                        false,
                    )?;
                next.patch(ctx, request).await
            }
        }
        fn apply<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotApplyRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Robot",
                        "Apply",
                        self.public.contains(&"/app.RobotService/Apply"),
                        false,
                    )?;
                next.apply(ctx, request).await
            }
        }
        fn erase<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotRef>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::RobotEraseResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Robot",
                        "Erase",
                        self.public.contains(&"/app.RobotService/Erase"),
                        false,
                    )?;
                next.erase(ctx, request).await
            }
        }
        fn list<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotListRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::RobotListResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Robot",
                        "List",
                        self.public.contains(&"/app.RobotService/List"),
                        false,
                    )?;
                next.list(ctx, request).await
            }
        }
        fn watch(
            &self,
            next: &N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotWatchRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                ::connectrpc::ServiceStream<
                    impl ::connectrpc::Encodable<
                        crate::proto::app::RobotWatchResponse,
                    > + Send + use<N>,
                >,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Robot",
                        "Watch",
                        self.public.contains(&"/app.RobotService/Watch"),
                        false,
                    )?;
                next.watch(ctx, request).await
            }
        }
        fn r#move<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotMoveRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Robot",
                        "Move",
                        self.public.contains(&"/app.RobotService/Move"),
                        true,
                    )?;
                next.r#move(ctx, request).await
            }
        }
    }
    impl<
        N: crate::services::r#app::RobotService,
    > crate::services::r#app::RobotServiceLayer<N> for Audit {
        fn add<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotAddRequest>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.add(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn add_many<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotAddManyRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::RobotAddManyResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.add_many(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn get<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotGetRequest>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.get(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn patch<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotPatchRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.patch(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn apply<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotApplyRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.apply(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn erase<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotRef>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::RobotEraseResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.erase(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn list<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotListRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::RobotListResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.list(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
        fn r#move<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::RobotMoveRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Robot,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self.0.begin(&mut ctx).await?;
                let result = next.r#move(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
    }
}
struct Gate {
    db: Database,
    public: &'static [&'static str],
}
struct Audit(Database);
#[derive(Clone)]
pub struct Next(pub Database);
impl Next {
    pub fn app_cell(&self) -> impl crate::services::r#app::CellService {
        crate::services::r#app::CellServiceOverlay {
            next: crate::services::r#app::CellServiceUnimplemented,
            layer: sink_app_cell::Sink(self.0.clone()),
        }
    }
    pub fn app_robot(&self) -> impl crate::services::r#app::RobotService {
        crate::services::r#app::RobotServiceOverlay {
            next: crate::services::r#app::RobotServiceUnimplemented,
            layer: sink_app_robot::Sink(self.0.clone()),
        }
    }
}
#[allow(async_fn_in_trait)]
pub trait CellCalls: crate::services::r#app::CellService {
    async fn add_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#CellAddRequest,
    ) -> Result<crate::proto::r#app::r#Cell, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#CellAddRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .add(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Cell::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn get_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#CellGetRequest,
    ) -> Result<crate::proto::r#app::r#Cell, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#CellGetRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .get(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Cell::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn patch_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#CellPatchRequest,
    ) -> Result<crate::proto::r#app::r#Cell, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#CellPatchRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .patch(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Cell::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn apply_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#CellApplyRequest,
    ) -> Result<crate::proto::r#app::r#Cell, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#CellApplyRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .apply(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Cell::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn erase_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#CellRef,
    ) -> Result<crate::proto::r#app::r#CellEraseResponse, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#CellRef::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .erase(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#CellEraseResponse::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
}
impl<N: crate::services::r#app::CellService> CellCalls for N {}
#[allow(async_fn_in_trait)]
pub trait RobotCalls: crate::services::r#app::RobotService {
    async fn add_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#RobotAddRequest,
    ) -> Result<crate::proto::r#app::r#Robot, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#RobotAddRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .add(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Robot::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn add_many_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#RobotAddManyRequest,
    ) -> Result<
        crate::proto::r#app::r#RobotAddManyResponse,
        ::connectrpc::ConnectError,
    > {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#RobotAddManyRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .add_many(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#RobotAddManyResponse::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn get_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#RobotGetRequest,
    ) -> Result<crate::proto::r#app::r#Robot, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#RobotGetRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .get(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Robot::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn patch_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#RobotPatchRequest,
    ) -> Result<crate::proto::r#app::r#Robot, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#RobotPatchRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .patch(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Robot::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn apply_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#RobotApplyRequest,
    ) -> Result<crate::proto::r#app::r#Robot, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#RobotApplyRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .apply(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Robot::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn erase_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#RobotRef,
    ) -> Result<crate::proto::r#app::r#RobotEraseResponse, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#RobotRef::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .erase(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#RobotEraseResponse::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn list_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#RobotListRequest,
    ) -> Result<crate::proto::r#app::r#RobotListResponse, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#RobotListRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .list(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#RobotListResponse::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    async fn move_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#RobotMoveRequest,
    ) -> Result<crate::proto::r#app::r#Robot, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#RobotMoveRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .r#move(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Robot::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
}
impl<N: crate::services::r#app::RobotService> RobotCalls for N {}
pub trait Layers {
    fn public_methods(&self) -> &'static [&'static str] {
        &[]
    }
    fn app_cell(
        &self,
        next: impl crate::services::r#app::CellService,
    ) -> impl crate::services::r#app::CellService {
        next
    }
    fn app_robot(
        &self,
        next: impl crate::services::r#app::RobotService,
    ) -> impl crate::services::r#app::RobotService {
        next
    }
}
pub fn mount(router: Router, db: Database, layers: &impl Layers) -> Router {
    let next = Next(db.clone());
    let core = layers.app_cell(next.app_cell());
    let audit = crate::services::r#app::CellServiceOverlay {
        next: core,
        layer: Audit(db.clone()),
    };
    let gate = crate::services::r#app::CellServiceOverlay {
        next: audit,
        layer: Gate {
            db: db.clone(),
            public: layers.public_methods(),
        },
    };
    let router = crate::services::r#app::CellServiceExt::register(
        ::std::sync::Arc::new(gate),
        router,
    );
    let core = layers.app_robot(next.app_robot());
    let audit = crate::services::r#app::RobotServiceOverlay {
        next: core,
        layer: Audit(db.clone()),
    };
    let gate = crate::services::r#app::RobotServiceOverlay {
        next: audit,
        layer: Gate {
            db: db.clone(),
            public: layers.public_methods(),
        },
    };
    let router = crate::services::r#app::RobotServiceExt::register(
        ::std::sync::Arc::new(gate),
        router,
    );
    router
}
