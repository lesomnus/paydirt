/// Services supplied by an application, independent of any database.
/// An omitted service answers Unimplemented.
pub trait Server {
    fn app_access(&self) -> impl crate::services::r#app::r#AccessService {
        crate::services::r#app::r#AccessServiceUnimplemented
    }
    fn app_audit(&self) -> impl crate::services::r#app::r#AuditService {
        crate::services::r#app::r#AuditServiceUnimplemented
    }
    fn app_cell(&self) -> impl crate::services::r#app::r#CellService {
        crate::services::r#app::r#CellServiceUnimplemented
    }
    fn app_holder(&self) -> impl crate::services::r#app::r#HolderService {
        crate::services::r#app::r#HolderServiceUnimplemented
    }
    fn app_robot(&self) -> impl crate::services::r#app::r#RobotService {
        crate::services::r#app::r#RobotServiceUnimplemented
    }
    fn app_tenant(&self) -> impl crate::services::r#app::r#TenantService {
        crate::services::r#app::r#TenantServiceUnimplemented
    }
    /// Add an outer layer. The last layer added handles a request first.
    fn layer<L: Layer<Self>>(self, layer: L) -> Overlay<Self, L>
    where
        Self: Sized,
    {
        Overlay { next: self, layer }
    }
}
#[derive(Clone, Copy, Default)]
pub struct Unimplemented;
impl Server for Unimplemented {}
/// Each accessor may replace a service or wrap the service behind it.
pub trait Layer<N: Server + ?Sized> {
    fn app_access(&self, next: &N) -> impl crate::services::r#app::r#AccessService {
        next.app_access()
    }
    fn app_audit(&self, next: &N) -> impl crate::services::r#app::r#AuditService {
        next.app_audit()
    }
    fn app_cell(&self, next: &N) -> impl crate::services::r#app::r#CellService {
        next.app_cell()
    }
    fn app_holder(&self, next: &N) -> impl crate::services::r#app::r#HolderService {
        next.app_holder()
    }
    fn app_robot(&self, next: &N) -> impl crate::services::r#app::r#RobotService {
        next.app_robot()
    }
    fn app_tenant(&self, next: &N) -> impl crate::services::r#app::r#TenantService {
        next.app_tenant()
    }
}
#[derive(Clone)]
pub struct Overlay<N, L> {
    pub next: N,
    pub layer: L,
}
impl<N: Server, L: Layer<N>> Server for Overlay<N, L> {
    fn app_access(&self) -> impl crate::services::r#app::r#AccessService {
        self.layer.app_access(&self.next)
    }
    fn app_audit(&self) -> impl crate::services::r#app::r#AuditService {
        self.layer.app_audit(&self.next)
    }
    fn app_cell(&self) -> impl crate::services::r#app::r#CellService {
        self.layer.app_cell(&self.next)
    }
    fn app_holder(&self) -> impl crate::services::r#app::r#HolderService {
        self.layer.app_holder(&self.next)
    }
    fn app_robot(&self) -> impl crate::services::r#app::r#RobotService {
        self.layer.app_robot(&self.next)
    }
    fn app_tenant(&self) -> impl crate::services::r#app::r#TenantService {
        self.layer.app_tenant(&self.next)
    }
}
impl<T: Server + ?Sized> Server for ::std::sync::Arc<T> {
    fn app_access(&self) -> impl crate::services::r#app::r#AccessService {
        self.as_ref().app_access()
    }
    fn app_audit(&self) -> impl crate::services::r#app::r#AuditService {
        self.as_ref().app_audit()
    }
    fn app_cell(&self) -> impl crate::services::r#app::r#CellService {
        self.as_ref().app_cell()
    }
    fn app_holder(&self) -> impl crate::services::r#app::r#HolderService {
        self.as_ref().app_holder()
    }
    fn app_robot(&self) -> impl crate::services::r#app::r#RobotService {
        self.as_ref().app_robot()
    }
    fn app_tenant(&self) -> impl crate::services::r#app::r#TenantService {
        self.as_ref().app_tenant()
    }
}
pub fn mount(
    router: ::connectrpc::Router,
    server: &impl Server,
) -> ::connectrpc::Router {
    mount_if(router, server, |_| true)
}
/// Select services at registration; this does not authorize requests.
pub fn mount_if(
    mut router: ::connectrpc::Router,
    server: &impl Server,
    include: impl Fn(&str) -> bool,
) -> ::connectrpc::Router {
    if include("app.AccessService") {
        router = crate::services::r#app::r#AccessServiceExt::register(
            ::std::sync::Arc::new(server.app_access()),
            router,
        );
    }
    if include("app.AuditService") {
        router = crate::services::r#app::r#AuditServiceExt::register(
            ::std::sync::Arc::new(server.app_audit()),
            router,
        );
    }
    if include("app.CellService") {
        router = crate::services::r#app::r#CellServiceExt::register(
            ::std::sync::Arc::new(server.app_cell()),
            router,
        );
    }
    if include("app.HolderService") {
        router = crate::services::r#app::r#HolderServiceExt::register(
            ::std::sync::Arc::new(server.app_holder()),
            router,
        );
    }
    if include("app.RobotService") {
        router = crate::services::r#app::r#RobotServiceExt::register(
            ::std::sync::Arc::new(server.app_robot()),
            router,
        );
    }
    if include("app.TenantService") {
        router = crate::services::r#app::r#TenantServiceExt::register(
            ::std::sync::Arc::new(server.app_tenant()),
            router,
        );
    }
    router
}
