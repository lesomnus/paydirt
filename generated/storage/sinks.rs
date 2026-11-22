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
}
pub trait Layers {
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
    let sink = crate::services::r#app::CellServiceOverlay {
        next: crate::services::r#app::CellServiceUnimplemented,
        layer: sink_app_cell::Sink(db.clone()),
    };
    let router = crate::services::r#app::CellServiceExt::register(
        ::std::sync::Arc::new(layers.app_cell(sink)),
        router,
    );
    let sink = crate::services::r#app::RobotServiceOverlay {
        next: crate::services::r#app::RobotServiceUnimplemented,
        layer: sink_app_robot::Sink(db.clone()),
    };
    let router = crate::services::r#app::RobotServiceExt::register(
        ::std::sync::Arc::new(layers.app_robot(sink)),
        router,
    );
    router
}
