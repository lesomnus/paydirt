#[rustfmt::skip]
#[path = "../../generated/messages/mod.rs"]
pub mod proto;
#[rustfmt::skip]
#[path = "../../generated/services/mod.rs"]
pub mod services;
pub mod orm {
    include!("../../generated/orm/app/robot_svc.g.orm.rs");
}
mod backend;
pub use backend::Backend;

use buffa_types::google::protobuf::Timestamp;
use connectrpc::{
    ConnectError, RequestContext, Response, Router, ServiceRequest, ServiceResult, ServiceStream,
};
use paydirt_robot_protocol::{Batch, Failure, Key, Operation, Request, Row};
use proto::app::*;
use std::{collections::HashMap, sync::Arc, time::Duration};

struct Robots {
    backend: Backend,
    // Provisioned by the host. RPC callers can only choose a tenant they hold a token for.
    principals: paydirt::tenant::TenantTokens,
}

pub fn router(backend: Backend, config: &str) -> Result<Router, String> {
    let pairs: HashMap<String, String> = serde_json::from_str(config).map_err(|e| e.to_string())?;
    let mut principals = HashMap::new();
    for (token, tenant) in pairs {
        let id = hex::decode(tenant).map_err(|e| e.to_string())?;
        principals.insert(token, id);
    }
    let principals = paydirt::tenant::TenantTokens::new(principals, 1)?;
    Ok(orm::register_robot_service(
        Router::new(),
        Arc::new(Robots {
            backend,
            principals,
        }),
    ))
}

fn missing() -> ConnectError {
    ConnectError::not_found("robot not found")
}
fn supported(fields: &buffa::UnknownFields) -> Result<(), ConnectError> {
    if fields.is_empty() {
        Ok(())
    } else {
        Err(ConnectError::unimplemented("unsupported protobuf field"))
    }
}
fn check_tenant(reference: &TenantRef, tenant: &[u8]) -> Result<(), ConnectError> {
    supported(&reference.__buffa_unknown_fields)?;
    match &reference.key {
        Some(tenant_ref::Key::Id(id)) if id == tenant => Ok(()),
        Some(tenant_ref::Key::Id(_)) => Err(missing()),
        Some(tenant_ref::Key::Alias(_)) => Err(ConnectError::unimplemented("tenant alias")),
        None => Err(ConnectError::invalid_argument("tenant reference required")),
    }
}
fn key(reference: &RobotRef, tenant: &[u8]) -> Result<Key, ConnectError> {
    supported(&reference.__buffa_unknown_fields)?;
    match &reference.key {
        Some(robot_ref::Key::Id(id)) if paydirt::has_domain(id, 7) => Ok(Key::Id(id.clone())),
        Some(robot_ref::Key::Slug(slug)) => {
            supported(&slug.__buffa_unknown_fields)?;
            check_tenant(&slug.tenant, tenant)?;
            Ok(Key::Alias(slug.alias.clone().ok_or_else(|| {
                ConnectError::invalid_argument("alias required")
            })?))
        }
        _ => Err(ConnectError::invalid_argument("Robot ID or slug required")),
    }
}
fn filter(
    filters: &[RobotFilter],
    tenant: &[u8],
    watch: bool,
) -> Result<Option<Vec<Key>>, ConnectError> {
    if filters.len() > 32 || (watch && filters.is_empty()) {
        return Err(ConnectError::invalid_argument(
            "Watch requires 1–32 filters; List accepts at most 32",
        ));
    }
    let mut keys = Vec::new();
    let mut all = filters.is_empty();
    for f in filters {
        supported(&f.__buffa_unknown_fields)?;
        if !f.r#ref.is_set() && (watch || !f.tenant.is_set()) {
            return Err(ConnectError::invalid_argument(
                "filter requires a Robot reference",
            ));
        }
        if let Some(t) = f.tenant.as_option() {
            match check_tenant(t, tenant) {
                Err(e) if !watch && e.code == connectrpc::ErrorCode::NotFound => continue,
                result => result?,
            }
        }
        if let Some(reference) = f.r#ref.as_option() {
            match key(reference, tenant) {
                Ok(k) => keys.push(k),
                Err(e) if !watch && e.code == connectrpc::ErrorCode::NotFound => continue,
                Err(e) => return Err(e),
            }
        } else {
            all = true;
        }
    }
    Ok(if all { None } else { Some(keys) })
}

fn now_ms() -> Result<i64, ConnectError> {
    Ok(web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_err(|_| ConnectError::internal("system clock predates epoch"))?
        .as_millis() as i64)
}
fn timestamp(time: i64) -> Timestamp {
    Timestamp {
        seconds: time.div_euclid(1000),
        nanos: (time.rem_euclid(1000) * 1_000_000) as i32,
        ..Default::default()
    }
}

fn robot(row: Row, tenant: &[u8]) -> Robot {
    Robot {
        id: row.id,
        alias: row.alias,
        tenant: Tenant {
            id: tenant.to_vec(),
            ..Default::default()
        }
        .into(),
        date_created: timestamp(row.created_ms).into(),
        date_updated: timestamp(row.updated_ms).into(),
        ..Default::default()
    }
}

async fn query(db: &Backend, tenant: &[u8], op: Operation) -> Result<Batch, ConnectError> {
    db.request(Request {
        tenant: tenant.to_vec(),
        op,
    })
    .await
    .map_err(|e| match e {
        Failure::Conflict => ConnectError::already_exists("duplicate robot ID or tenant alias"),
        Failure::Missing => missing(),
        Failure::Internal => ConnectError::internal("storage operation failed"),
        Failure::Unavailable => ConnectError::unavailable("storage worker unavailable"),
    })
}

impl Robots {
    fn tenant(&self, ctx: &RequestContext) -> Result<Vec<u8>, ConnectError> {
        ctx.headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| self.principals.resolve(v))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| ConnectError::unauthenticated("unknown bearer token"))
    }
    async fn create(
        &self,
        tenant: &[u8],
        items: Vec<RobotAddRequest>,
        action: &str,
    ) -> Result<Vec<Robot>, ConnectError> {
        if items.len() > 1000 {
            return Err(ConnectError::invalid_argument(
                "at most 1000 robots per call",
            ));
        }
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            supported(&item.__buffa_unknown_fields)?;
            if item.cell.is_set() || item.date_created.is_set() {
                return Err(ConnectError::unimplemented(
                    "unsupported Robot creation field",
                ));
            }
            check_tenant(&item.tenant, tenant)?;
            let id = item.id.unwrap_or_else(|| paydirt::mint(7).to_vec());
            if !paydirt::has_domain(&id, 7) {
                return Err(ConnectError::invalid_argument(
                    "expected a domain 7 Robot ID",
                ));
            }
            let created_ms = now_ms()?;
            rows.push(Row {
                id,
                alias: item.alias,
                created_ms,
                updated_ms: created_ms,
            });
        }
        Ok(query(
            &self.backend,
            tenant,
            Operation::Add {
                rows,
                action: action.into(),
            },
        )
        .await?
        .rows
        .into_iter()
        .map(|r| robot(r, tenant))
        .collect())
    }
}

// The cursor is scoped to a tenant and the fixture's sole supported order.
fn cursor(tenant: &[u8], row: &Row) -> String {
    hex::encode([tenant, &row.created_ms.to_be_bytes(), &row.id].concat())
}
fn resume(tenant: &[u8], value: &str) -> Result<Option<(i64, Vec<u8>)>, ConnectError> {
    if value.is_empty() {
        return Ok(None);
    }
    let invalid = || ConnectError::invalid_argument("invalid Robot list cursor");
    if value.len() != 80 {
        return Err(invalid());
    }
    let bytes = hex::decode(value).map_err(|_| invalid())?;
    if &bytes[..16] != tenant || !paydirt::has_domain(&bytes[24..], 7) {
        return Err(invalid());
    }
    Ok(Some((
        i64::from_be_bytes(bytes[16..24].try_into().unwrap()),
        bytes[24..].to_vec(),
    )))
}

#[allow(refining_impl_trait)]
impl services::app::RobotService for Robots {
    async fn add(
        &self,
        ctx: RequestContext,
        req: ServiceRequest<'_, RobotAddRequest>,
    ) -> ServiceResult<Robot> {
        let tenant = self.tenant(&ctx)?;
        let mut result = self
            .create(
                &tenant,
                vec![req.to_owned_message()],
                "/app.RobotService/Add",
            )
            .await?;
        Response::ok(
            result
                .pop()
                .ok_or_else(|| ConnectError::internal("missing inserted row"))?,
        )
    }
    async fn add_many(
        &self,
        ctx: RequestContext,
        req: ServiceRequest<'_, RobotAddManyRequest>,
    ) -> ServiceResult<RobotAddManyResponse> {
        let tenant = self.tenant(&ctx)?;
        let req = req.to_owned_message();
        supported(&req.__buffa_unknown_fields)?;
        Response::ok(RobotAddManyResponse {
            items: self
                .create(&tenant, req.items, "/app.RobotService/AddMany")
                .await?,
            ..Default::default()
        })
    }
    async fn get(
        &self,
        ctx: RequestContext,
        req: ServiceRequest<'_, RobotGetRequest>,
    ) -> ServiceResult<Robot> {
        let tenant = self.tenant(&ctx)?;
        let req = req.to_owned_message();
        supported(&req.__buffa_unknown_fields)?;
        let selected = req.select.as_option();
        if let Some(s) = selected {
            supported(&s.__buffa_unknown_fields)?;
            if s.cell.is_set() || s.tenant.is_set() {
                return Err(ConnectError::unimplemented("nested Robot selection"));
            }
        }
        let mut rows = query(
            &self.backend,
            &tenant,
            Operation::Get(key(&req.r#ref, &tenant)?),
        )
        .await?
        .rows;
        let mut value = robot(rows.pop().ok_or_else(missing)?, &tenant);
        if let Some(s) = selected {
            let all = s.all.unwrap_or(false);
            if !all && !s.alias.unwrap_or(false) {
                value.alias.clear();
            }
            value.tenant = Default::default();
            if !all && !s.date_created.unwrap_or(false) {
                value.date_created = Default::default();
            }
            if !all && !s.date_updated.unwrap_or(false) {
                value.date_updated = Default::default();
            }
        }
        Response::ok(value)
    }
    async fn patch(
        &self,
        ctx: RequestContext,
        req: ServiceRequest<'_, RobotPatchRequest>,
    ) -> ServiceResult<Robot> {
        let tenant = self.tenant(&ctx)?;
        let req = req.to_owned_message();
        supported(&req.__buffa_unknown_fields)?;
        if req.cell.is_set()
            || req.cell_null.is_some()
            || req.date_updated.is_set()
            || req.date_updated_force.is_some()
        {
            return Err(ConnectError::unimplemented(
                "only alias patches are supported",
            ));
        }
        let alias = req
            .alias
            .ok_or_else(|| ConnectError::invalid_argument("an alias patch is required"))?;
        let mut result = query(
            &self.backend,
            &tenant,
            Operation::Patch {
                key: key(&req.r#ref, &tenant)?,
                alias,
                now: now_ms()?,
            },
        )
        .await?;
        Response::ok(robot(result.rows.pop().ok_or_else(missing)?, &tenant))
    }
    async fn apply(
        &self,
        _: RequestContext,
        _: ServiceRequest<'_, RobotApplyRequest>,
    ) -> ServiceResult<Robot> {
        Err(ConnectError::unimplemented("patch documents"))
    }
    async fn r#move(
        &self,
        _: RequestContext,
        _: ServiceRequest<'_, RobotMoveRequest>,
    ) -> ServiceResult<Robot> {
        Err(ConnectError::unimplemented(
            "Move requires an application layer",
        ))
    }
    async fn erase(
        &self,
        ctx: RequestContext,
        req: ServiceRequest<'_, RobotRef>,
    ) -> ServiceResult<RobotEraseResponse> {
        let tenant = self.tenant(&ctx)?;
        let erased = match key(&req.to_owned_message(), &tenant) {
            Ok(key) => !query(&self.backend, &tenant, Operation::Erase(key))
                .await?
                .rows
                .is_empty(),
            Err(e) if e.code == connectrpc::ErrorCode::NotFound => false,
            Err(e) => return Err(e),
        };
        Response::ok(RobotEraseResponse {
            erased: Some(erased),
            ..Default::default()
        })
    }

    async fn list(
        &self,
        ctx: RequestContext,
        req: ServiceRequest<'_, RobotListRequest>,
    ) -> ServiceResult<RobotListResponse> {
        let tenant = self.tenant(&ctx)?;
        let req = req.to_owned_message();
        supported(&req.__buffa_unknown_fields)?;
        if req.reverse || req.order.to_i32() != 0 {
            return Err(ConnectError::unimplemented(
                "only the default forward order is implemented",
            ));
        }
        if req.size < 0 {
            return Err(ConnectError::invalid_argument("negative page size"));
        }
        let size = if req.size == 0 { 50 } else { req.size.min(100) } as usize;
        let mut rows = query(
            &self.backend,
            &tenant,
            Operation::List {
                keys: filter(&req.filters, &tenant, false)?,
                after: resume(&tenant, &req.after)?,
                limit: (size + 1) as i64,
            },
        )
        .await?
        .rows;
        let mut next = String::new();
        if rows.len() > size {
            rows.truncate(size);
            next = cursor(&tenant, &rows[size - 1]);
        }
        Response::ok(RobotListResponse {
            items: rows.into_iter().map(|r| robot(r, &tenant)).collect(),
            next,
            ..Default::default()
        })
    }
    async fn watch(
        &self,
        ctx: RequestContext,
        req: ServiceRequest<'_, RobotWatchRequest>,
    ) -> ServiceResult<ServiceStream<RobotWatchResponse>> {
        let tenant = self.tenant(&ctx)?;
        let req = req.to_owned_message();
        supported(&req.__buffa_unknown_fields)?;
        let keys = filter(&req.filters, &tenant, true)?
            .ok_or_else(|| ConnectError::invalid_argument("Robot references required"))?;
        let snapshot = query(&self.backend, &tenant, Operation::Snapshot { keys }).await?;
        let ids: Vec<_> = snapshot.rows.iter().map(|row| row.id.clone()).collect();
        let db = self.backend.clone();
        let events = async_stream::try_stream! {
            let mut since = snapshot.revision;
            let mut seen = std::collections::HashSet::new();
            if !req.skip_snapshot {
                seen.extend(ids.iter().cloned());
                let items = snapshot.rows.into_iter().map(|row| watch_item(row.id.clone(),Some(row),String::new(),&tenant)).collect();
                yield RobotWatchResponse { items, ..Default::default() };
            }
            loop {
                let batch = query(&db,&tenant,Operation::Changes { since,ids:ids.clone() }).await?;
                since = batch.revision;
                if batch.changes.is_empty() {
                    futures_timer::Delay::new(Duration::from_millis(100)).await;
                    continue;
                }
                let mut items = Vec::new();
                for change in batch.changes {
                    if change.value.is_some() { seen.insert(change.id.clone()); }
                    else if !seen.remove(&change.id) { continue; }
                    items.push(watch_item(change.id,change.value,change.action,&tenant));
                }
                if !items.is_empty() { yield RobotWatchResponse {items,..Default::default()}; }
            }
        };
        Response::stream_ok(events)
    }
}

fn watch_item(id: Vec<u8>, row: Option<Row>, action: String, tenant: &[u8]) -> RobotWatchItem {
    RobotWatchItem {
        id: Some(id),
        value: row
            .map(|row| robot(row, tenant))
            .map(Into::into)
            .unwrap_or_default(),
        action,
        ..Default::default()
    }
}

#[cfg(target_arch = "wasm32")]
mod worker {
    use super::*;
    use std::num::NonZeroU32;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct RobotPort {
        _server: drpc_rs::PortServer,
    }
    #[wasm_bindgen]
    impl RobotPort {
        #[wasm_bindgen(constructor)]
        pub fn new(
            port: web_sys::MessagePort,
            epoch: u32,
            config: &str,
        ) -> Result<RobotPort, JsError> {
            let epoch = NonZeroU32::new(epoch).ok_or_else(|| JsError::new("zero epoch"))?;
            let routes = router(Backend, config).map_err(|e| JsError::new(&e))?;
            Ok(Self {
                _server: drpc_rs::PortServer::bind(port, epoch, routes),
            })
        }
    }
}
