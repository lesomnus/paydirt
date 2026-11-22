#[path = "intercept.rs"]
pub mod intercept;
use crate::transport::Database;
use ::connectrpc::{RequestContext, Response, Router, ServiceRequest, ServiceResult};
impl<
    N: crate::services::r#app::AccessService,
> crate::services::r#app::AccessServiceLayer<N> for crate::storage::intercept::Layer {
    fn get<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::AccessGetRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Access> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.AccessService/Get", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.get(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Access, _>(reply)
        }
    }
    fn list<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::AccessListRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::app::AccessListResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.AccessService/List", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.list(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#AccessListResponse, _>(reply)
        }
    }
}
impl<
    N: crate::services::r#app::ArchivedService,
> crate::services::r#app::ArchivedServiceLayer<N> for crate::storage::intercept::Layer {}
impl<
    N: crate::services::r#app::AuditService,
> crate::services::r#app::AuditServiceLayer<N> for crate::storage::intercept::Layer {
    fn add<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::AuditAddRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Audit> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.AuditService/Add", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.add(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Audit, _>(reply)
        }
    }
    fn get<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::AuditGetRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Audit> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.AuditService/Get", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.get(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Audit, _>(reply)
        }
    }
    fn patch<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::AuditPatchRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Audit> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.AuditService/Patch", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.patch(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Audit, _>(reply)
        }
    }
    fn apply<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::AuditApplyRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Audit> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.AuditService/Apply", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.apply(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Audit, _>(reply)
        }
    }
    fn erase<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::AuditRef>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::app::AuditEraseResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.AuditService/Erase", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.erase(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#AuditEraseResponse, _>(reply)
        }
    }
    fn list<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::AuditListRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::app::AuditListResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.AuditService/List", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.list(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#AuditListResponse, _>(reply)
        }
    }
}
impl<
    N: crate::services::r#app::TenantService,
> crate::services::r#app::TenantServiceLayer<N> for crate::storage::intercept::Layer {
    fn add<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::TenantAddRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Tenant> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.TenantService/Add", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.add(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Tenant, _>(reply)
        }
    }
    fn get<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::TenantGetRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Tenant> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.TenantService/Get", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.get(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Tenant, _>(reply)
        }
    }
    fn patch<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::TenantPatchRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Tenant> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.TenantService/Patch", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.patch(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Tenant, _>(reply)
        }
    }
    fn apply<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::TenantApplyRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Tenant> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.TenantService/Apply", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.apply(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Tenant, _>(reply)
        }
    }
    fn erase<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::TenantRef>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::app::TenantEraseResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.TenantService/Erase", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.erase(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#TenantEraseResponse, _>(reply)
        }
    }
    fn list<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::TenantListRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::app::TenantListResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.TenantService/List", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.list(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#TenantListResponse, _>(reply)
        }
    }
}
impl<
    N: crate::services::r#app::HolderService,
> crate::services::r#app::HolderServiceLayer<N> for crate::storage::intercept::Layer {
    fn add<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::HolderAddRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Holder> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.HolderService/Add", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.add(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Holder, _>(reply)
        }
    }
    fn get<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::HolderGetRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Holder> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.HolderService/Get", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.get(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Holder, _>(reply)
        }
    }
    fn patch<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::HolderPatchRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Holder> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.HolderService/Patch", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.patch(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Holder, _>(reply)
        }
    }
    fn apply<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::HolderApplyRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Holder> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.HolderService/Apply", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.apply(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Holder, _>(reply)
        }
    }
    fn erase<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::HolderRef>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::app::HolderEraseResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.HolderService/Erase", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.erase(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#HolderEraseResponse, _>(reply)
        }
    }
    fn list<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::HolderListRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::app::HolderListResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.HolderService/List", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.list(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#HolderListResponse, _>(reply)
        }
    }
    fn watch(
        &self,
        next: &N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::HolderWatchRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::proto::app::HolderWatchResponse,
                > + Send + use<N>,
            >,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.HolderService/Watch", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.watch(call.ctx.clone(), request).await;
            call.stream::<crate::proto::r#app::r#HolderWatchResponse, _>(reply)
        }
    }
}
impl<
    N: crate::services::r#app::OutboxService,
> crate::services::r#app::OutboxServiceLayer<N> for crate::storage::intercept::Layer {}
impl<N: crate::services::r#app::OwedService> crate::services::r#app::OwedServiceLayer<N>
for crate::storage::intercept::Layer {}
impl<
    N: crate::services::r#app::RowSealService,
> crate::services::r#app::RowSealServiceLayer<N> for crate::storage::intercept::Layer {}
impl<N: crate::services::r#app::CellService> crate::services::r#app::CellServiceLayer<N>
for crate::storage::intercept::Layer {
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
            let mut call = self.enter("/app.CellService/Add", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.add(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Cell, _>(reply)
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
            let mut call = self.enter("/app.CellService/Get", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.get(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Cell, _>(reply)
        }
    }
    fn patch<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::CellPatchRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.CellService/Patch", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.patch(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Cell, _>(reply)
        }
    }
    fn apply<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::CellApplyRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Cell> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.CellService/Apply", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.apply(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Cell, _>(reply)
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
            let mut call = self.enter("/app.CellService/Erase", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.erase(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#CellEraseResponse, _>(reply)
        }
    }
}
impl<
    N: crate::services::r#app::RobotService,
> crate::services::r#app::RobotServiceLayer<N> for crate::storage::intercept::Layer {
    fn add<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotAddRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Robot> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.RobotService/Add", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.add(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Robot, _>(reply)
        }
    }
    fn add_many<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotAddManyRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::app::RobotAddManyResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.RobotService/AddMany", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.add_many(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#RobotAddManyResponse, _>(reply)
        }
    }
    fn get<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotGetRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Robot> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.RobotService/Get", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.get(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Robot, _>(reply)
        }
    }
    fn patch<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotPatchRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Robot> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.RobotService/Patch", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.patch(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Robot, _>(reply)
        }
    }
    fn apply<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotApplyRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Robot> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.RobotService/Apply", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.apply(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Robot, _>(reply)
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
            let mut call = self.enter("/app.RobotService/Erase", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.erase(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#RobotEraseResponse, _>(reply)
        }
    }
    fn list<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotListRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::app::RobotListResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.RobotService/List", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.list(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#RobotListResponse, _>(reply)
        }
    }
    fn watch(
        &self,
        next: &N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotWatchRequest>,
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
            let mut call = self.enter("/app.RobotService/Watch", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.watch(call.ctx.clone(), request).await;
            call.stream::<crate::proto::r#app::r#RobotWatchResponse, _>(reply)
        }
    }
    fn r#move<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::app::RobotMoveRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<crate::proto::app::Robot> + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/app.RobotService/Move", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.r#move(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#app::r#Robot, _>(reply)
        }
    }
}
impl<
    N: crate::services::r#payday::GrantService,
> crate::services::r#payday::GrantServiceLayer<N> for crate::storage::intercept::Layer {
    fn watch(
        &self,
        next: &N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::proto::payday::GrantWatchRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::proto::payday::GrantWatchResponse,
                > + Send + use<N>,
            >,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/payday.GrantService/Watch", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.watch(call.ctx.clone(), request).await;
            call.stream::<crate::proto::r#payday::r#GrantWatchResponse, _>(reply)
        }
    }
    fn list<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::payday::GrantListRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::payday::GrantListResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/payday.GrantService/List", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.list(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#payday::r#GrantListResponse, _>(reply)
        }
    }
}
impl<
    N: crate::services::r#payday::ResourceService,
> crate::services::r#payday::ResourceServiceLayer<N>
for crate::storage::intercept::Layer {
    fn watch(
        &self,
        next: &N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<'_, crate::proto::payday::WatchRequest>,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::proto::payday::WatchEvent,
                > + Send + use<N>,
            >,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/payday.ResourceService/Watch", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.watch(call.ctx.clone(), request).await;
            call.stream::<crate::proto::r#payday::r#WatchEvent, _>(reply)
        }
    }
}
impl<
    N: crate::services::r#payday::TokenService,
> crate::services::r#payday::TokenServiceLayer<N> for crate::storage::intercept::Layer {
    fn introspect<'a>(
        &'a self,
        next: &'a N,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::proto::payday::TokenIntrospectRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::payday::TokenIntrospectResponse,
            > + Send + use<'a, N>,
        >,
    > + Send {
        async move {
            let mut call = self.enter("/payday.TokenService/Introspect", ctx)?;
            call.receive(request.bytes())?;
            let reply = next.introspect(call.ctx.clone(), request).await;
            call.unary::<crate::proto::r#payday::r#TokenIntrospectResponse, _>(reply)
        }
    }
}
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Cell",
                        "Add",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Cell",
                        "Get",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Cell",
                        "Patch",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Cell",
                        "Apply",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Cell",
                        "Erase",
                        || request.to_owned_message(),
                    )
                    .await?;
                let result = next.erase(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
    }
}
mod sink_app_holder {
    use super::*;
    pub struct Sink(pub Database);
    #[allow(refining_impl_trait)]
    impl<
        N: crate::services::r#app::HolderService,
    > crate::services::r#app::HolderServiceLayer<N> for Sink {
        async fn get(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#HolderGetRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Holder> {
            Response::ok(
                self.0.rpc(&ctx, "app.Holder", "Get", &request.to_owned_message()).await?,
            )
        }
        async fn list(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#HolderListRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#HolderListResponse> {
            Response::ok(
                self
                    .0
                    .rpc(&ctx, "app.Holder", "List", &request.to_owned_message())
                    .await?,
            )
        }
    }
    impl<
        N: crate::services::r#app::HolderService,
    > crate::services::r#app::HolderServiceLayer<N> for Gate {
        fn add<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::HolderAddRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Holder,
                > + Send + use<'a, N>,
            >,
        > + Send {
            let _ = next;
            crate::services::r#app::HolderService::add(
                &crate::services::r#app::HolderServiceUnimplemented,
                ctx,
                request,
            )
        }
        fn get<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::HolderGetRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Holder,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Holder",
                        "Get",
                        self.public.contains(&"/app.HolderService/Get"),
                        false,
                        false,
                    )
                    .await?;
                next.get(ctx, request).await
            }
        }
        fn patch<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::HolderPatchRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Holder,
                > + Send + use<'a, N>,
            >,
        > + Send {
            let _ = next;
            crate::services::r#app::HolderService::patch(
                &crate::services::r#app::HolderServiceUnimplemented,
                ctx,
                request,
            )
        }
        fn apply<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::HolderApplyRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Holder,
                > + Send + use<'a, N>,
            >,
        > + Send {
            let _ = next;
            crate::services::r#app::HolderService::apply(
                &crate::services::r#app::HolderServiceUnimplemented,
                ctx,
                request,
            )
        }
        fn erase<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::HolderRef>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::HolderEraseResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            let _ = next;
            crate::services::r#app::HolderService::erase(
                &crate::services::r#app::HolderServiceUnimplemented,
                ctx,
                request,
            )
        }
        fn list<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::HolderListRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::HolderListResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Holder",
                        "List",
                        self.public.contains(&"/app.HolderService/List"),
                        false,
                        false,
                    )
                    .await?;
                next.list(ctx, request).await
            }
        }
        fn watch(
            &self,
            next: &N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::HolderWatchRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                ::connectrpc::ServiceStream<
                    impl ::connectrpc::Encodable<
                        crate::proto::app::HolderWatchResponse,
                    > + Send + use<N>,
                >,
            >,
        > + Send {
            let _ = next;
            crate::services::r#app::HolderService::watch(
                &crate::services::r#app::HolderServiceUnimplemented,
                ctx,
                request,
            )
        }
    }
    impl<
        N: crate::services::r#app::HolderService,
    > crate::services::r#app::HolderServiceLayer<N> for Audit {
        fn get<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::HolderGetRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Holder,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Holder",
                        "Get",
                        || request.to_owned_message(),
                    )
                    .await?;
                let result = next.get(ctx, request).await;
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
                crate::proto::app::HolderListRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::HolderListResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Holder",
                        "List",
                        || request.to_owned_message(),
                    )
                    .await?;
                let result = next.list(ctx, request).await;
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
        async fn list(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#RobotListRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#RobotListResponse> {
            Response::ok(
                self.0.rpc(&ctx, "app.Robot", "List", &request.to_owned_message()).await?,
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
        async fn watch(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#RobotWatchRequest>,
        ) -> ServiceResult<
            ::connectrpc::ServiceStream<crate::proto::r#app::r#RobotWatchResponse>,
        > {
            self.0.watch(ctx, "app.Robot", &request.to_owned_message()).await
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
                        false,
                    )
                    .await?;
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
                    .authorize_cost(
                        ctx,
                        "app.Robot",
                        "AddMany",
                        self.public.contains(&"/app.RobotService/AddMany"),
                        false,
                        false,
                        request.items.len(),
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                        false,
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Robot",
                        "Add",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Robot",
                        "AddMany",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Robot",
                        "Get",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Robot",
                        "Patch",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Robot",
                        "Apply",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Robot",
                        "Erase",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Robot",
                        "List",
                        || request.to_owned_message(),
                    )
                    .await?;
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
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Robot",
                        "Move",
                        || request.to_owned_message(),
                    )
                    .await?;
                let result = next.r#move(ctx, request).await;
                if let Some(guard) = guard {
                    guard.finish(result.is_ok()).await?;
                }
                result
            }
        }
    }
}
mod sink_app_tenant {
    use super::*;
    pub struct Sink(pub Database);
    #[allow(refining_impl_trait)]
    impl<
        N: crate::services::r#app::TenantService,
    > crate::services::r#app::TenantServiceLayer<N> for Sink {
        async fn get(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#TenantGetRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#Tenant> {
            Response::ok(
                self.0.rpc(&ctx, "app.Tenant", "Get", &request.to_owned_message()).await?,
            )
        }
        async fn list(
            &self,
            _: &N,
            ctx: RequestContext,
            request: ServiceRequest<'_, crate::proto::r#app::r#TenantListRequest>,
        ) -> ServiceResult<crate::proto::r#app::r#TenantListResponse> {
            Response::ok(
                self
                    .0
                    .rpc(&ctx, "app.Tenant", "List", &request.to_owned_message())
                    .await?,
            )
        }
    }
    impl<
        N: crate::services::r#app::TenantService,
    > crate::services::r#app::TenantServiceLayer<N> for Gate {
        fn add<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::TenantAddRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Tenant,
                > + Send + use<'a, N>,
            >,
        > + Send {
            let _ = next;
            crate::services::r#app::TenantService::add(
                &crate::services::r#app::TenantServiceUnimplemented,
                ctx,
                request,
            )
        }
        fn get<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::TenantGetRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Tenant,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Tenant",
                        "Get",
                        self.public.contains(&"/app.TenantService/Get"),
                        false,
                        false,
                    )
                    .await?;
                next.get(ctx, request).await
            }
        }
        fn patch<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::TenantPatchRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Tenant,
                > + Send + use<'a, N>,
            >,
        > + Send {
            let _ = next;
            crate::services::r#app::TenantService::patch(
                &crate::services::r#app::TenantServiceUnimplemented,
                ctx,
                request,
            )
        }
        fn apply<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::TenantApplyRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Tenant,
                > + Send + use<'a, N>,
            >,
        > + Send {
            let _ = next;
            crate::services::r#app::TenantService::apply(
                &crate::services::r#app::TenantServiceUnimplemented,
                ctx,
                request,
            )
        }
        fn erase<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<'_, crate::proto::app::TenantRef>,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::TenantEraseResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            let _ = next;
            crate::services::r#app::TenantService::erase(
                &crate::services::r#app::TenantServiceUnimplemented,
                ctx,
                request,
            )
        }
        fn list<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::TenantListRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::TenantListResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let ctx = self
                    .db
                    .authorize(
                        ctx,
                        "app.Tenant",
                        "List",
                        self.public.contains(&"/app.TenantService/List"),
                        false,
                        false,
                    )
                    .await?;
                next.list(ctx, request).await
            }
        }
    }
    impl<
        N: crate::services::r#app::TenantService,
    > crate::services::r#app::TenantServiceLayer<N> for Audit {
        fn get<'a>(
            &'a self,
            next: &'a N,
            ctx: ::connectrpc::RequestContext,
            request: ::connectrpc::ServiceRequest<
                '_,
                crate::proto::app::TenantGetRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::Tenant,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Tenant",
                        "Get",
                        || request.to_owned_message(),
                    )
                    .await?;
                let result = next.get(ctx, request).await;
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
                crate::proto::app::TenantListRequest,
            >,
        ) -> impl ::std::future::Future<
            Output = ::connectrpc::ServiceResult<
                impl ::connectrpc::Encodable<
                    crate::proto::app::TenantListResponse,
                > + Send + use<'a, N>,
            >,
        > + Send {
            async move {
                let mut ctx = ctx;
                let guard = self
                    .0
                    .begin_request(
                        &mut ctx,
                        "app.Tenant",
                        "List",
                        || request.to_owned_message(),
                    )
                    .await?;
                let result = next.list(ctx, request).await;
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
    pub fn app_holder(&self) -> impl crate::services::r#app::HolderService {
        crate::services::r#app::HolderServiceOverlay {
            next: crate::services::r#app::HolderServiceUnimplemented,
            layer: sink_app_holder::Sink(self.0.clone()),
        }
    }
    pub fn app_robot(&self) -> impl crate::services::r#app::RobotService {
        crate::services::r#app::RobotServiceOverlay {
            next: crate::services::r#app::RobotServiceUnimplemented,
            layer: sink_app_robot::Sink(self.0.clone()),
        }
    }
    pub fn app_tenant(&self) -> impl crate::services::r#app::TenantService {
        crate::services::r#app::TenantServiceOverlay {
            next: crate::services::r#app::TenantServiceUnimplemented,
            layer: sink_app_tenant::Sink(self.0.clone()),
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
pub trait HolderCalls: crate::services::r#app::HolderService {
    async fn get_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#HolderGetRequest,
    ) -> Result<crate::proto::r#app::r#Holder, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#HolderGetRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .get(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Holder::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    fn list_all<'a>(
        &'a self,
        ctx: &'a RequestContext,
        request: &crate::proto::r#app::r#HolderListRequest,
    ) -> impl ::paydirt::page::Stream<
        Item = Result<crate::proto::r#app::Holder, ::connectrpc::ConnectError>,
    > + 'a {
        ::paydirt::page::all(
            request,
            |q| &mut q.after,
            move |q| async move {
                let page = self.list_owned(ctx, q).await?;
                Ok((page.items, page.next))
            },
            || ::connectrpc::ConnectError::internal("repeated continuation token"),
        )
    }
    async fn list_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#HolderListRequest,
    ) -> Result<crate::proto::r#app::r#HolderListResponse, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#HolderListRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .list(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#HolderListResponse::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
}
impl<N: crate::services::r#app::HolderService> HolderCalls for N {}
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
    fn list_all<'a>(
        &'a self,
        ctx: &'a RequestContext,
        request: &crate::proto::r#app::r#RobotListRequest,
    ) -> impl ::paydirt::page::Stream<
        Item = Result<crate::proto::r#app::Robot, ::connectrpc::ConnectError>,
    > + 'a {
        ::paydirt::page::all(
            request,
            |q| &mut q.after,
            move |q| async move {
                let page = self.list_owned(ctx, q).await?;
                Ok((page.items, page.next))
            },
            || ::connectrpc::ConnectError::internal("repeated continuation token"),
        )
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
#[allow(async_fn_in_trait)]
pub trait TenantCalls: crate::services::r#app::TenantService {
    async fn get_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#TenantGetRequest,
    ) -> Result<crate::proto::r#app::r#Tenant, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#TenantGetRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .get(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#Tenant::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
    fn list_all<'a>(
        &'a self,
        ctx: &'a RequestContext,
        request: &crate::proto::r#app::r#TenantListRequest,
    ) -> impl ::paydirt::page::Stream<
        Item = Result<crate::proto::r#app::Tenant, ::connectrpc::ConnectError>,
    > + 'a {
        ::paydirt::page::all(
            request,
            |q| &mut q.after,
            move |q| async move {
                let page = self.list_owned(ctx, q).await?;
                Ok((page.items, page.next))
            },
            || ::connectrpc::ConnectError::internal("repeated continuation token"),
        )
    }
    async fn list_owned(
        &self,
        ctx: &RequestContext,
        req: crate::proto::r#app::r#TenantListRequest,
    ) -> Result<crate::proto::r#app::r#TenantListResponse, ::connectrpc::ConnectError> {
        use buffa::{Message, HasMessageView};
        use connectrpc::Encodable;
        let bytes = req.encode_to_bytes();
        let view = crate::proto::r#app::r#TenantListRequest::decode_view(&bytes)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream request",
            ))?;
        let reply = self
            .list(ctx.clone(), ServiceRequest::from_parts(&view, &bytes))
            .await?;
        let wire = reply.body.encode(connectrpc::CodecFormat::Proto)?;
        crate::proto::r#app::r#TenantListResponse::decode_from_slice(&wire)
            .map_err(|_| connectrpc::ConnectError::internal(
                "invalid downstream response",
            ))
    }
}
impl<N: crate::services::r#app::TenantService> TenantCalls for N {}
pub trait Layers {
    fn interceptors(&self) -> crate::storage::intercept::Layer {
        Default::default()
    }
    fn public_methods(&self) -> &'static [&'static str] {
        &[]
    }
    fn app_cell(
        &self,
        next: impl crate::services::r#app::CellService,
    ) -> impl crate::services::r#app::CellService {
        next
    }
    fn app_holder(
        &self,
        next: impl crate::services::r#app::HolderService,
    ) -> impl crate::services::r#app::HolderService {
        next
    }
    fn app_robot(
        &self,
        next: impl crate::services::r#app::RobotService,
    ) -> impl crate::services::r#app::RobotService {
        next
    }
    fn app_tenant(
        &self,
        next: impl crate::services::r#app::TenantService,
    ) -> impl crate::services::r#app::TenantService {
        next
    }
}
pub fn mount(router: Router, db: Database, layers: &impl Layers) -> Router {
    let next = Next(db.clone());
    let interceptors = layers.interceptors().public();
    let core = layers.app_cell(next.app_cell());
    let core = crate::services::r#app::CellServiceOverlay {
        next: core,
        layer: interceptors.clone(),
    };
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
    let core = layers.app_holder(next.app_holder());
    let core = crate::services::r#app::HolderServiceOverlay {
        next: core,
        layer: interceptors.clone(),
    };
    let audit = crate::services::r#app::HolderServiceOverlay {
        next: core,
        layer: Audit(db.clone()),
    };
    let gate = crate::services::r#app::HolderServiceOverlay {
        next: audit,
        layer: Gate {
            db: db.clone(),
            public: layers.public_methods(),
        },
    };
    let router = crate::services::r#app::HolderServiceExt::register(
        ::std::sync::Arc::new(gate),
        router,
    );
    let core = layers.app_robot(next.app_robot());
    let core = crate::services::r#app::RobotServiceOverlay {
        next: core,
        layer: interceptors.clone(),
    };
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
    let core = layers.app_tenant(next.app_tenant());
    let core = crate::services::r#app::TenantServiceOverlay {
        next: core,
        layer: interceptors.clone(),
    };
    let audit = crate::services::r#app::TenantServiceOverlay {
        next: core,
        layer: Audit(db.clone()),
    };
    let gate = crate::services::r#app::TenantServiceOverlay {
        next: audit,
        layer: Gate {
            db: db.clone(),
            public: layers.public_methods(),
        },
    };
    let router = crate::services::r#app::TenantServiceExt::register(
        ::std::sync::Arc::new(gate),
        router,
    );
    router
}
