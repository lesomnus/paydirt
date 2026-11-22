//! Optional cross-entity ResourceService. Recording and publishing share the same configuration.
use crate::{
    proto::payday::{WatchEvent, WatchRequest},
    services::payday::{ResourceService, ResourceServiceExt},
    transport::{ADMISSION_SCHEMA, Database, POLICIES, error},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use buffa::Message;
use connectrpc::{
    ConnectError, ErrorCode, RequestContext, Response, Router, ServiceRequest, ServiceResult,
    ServiceStream,
};
use futures_util::StreamExt;
use paydirt_store::resource::{Options, PublicEvents};
use std::sync::{Arc, LazyLock};
const DESCRIPTORS: &[u8] = include_bytes!("../generated/storage/descriptors.binpb");
static OPTIONS: LazyLock<Options> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../resources.json")).expect("resource configuration")
});
static EVENTS: LazyLock<PublicEvents> = LazyLock::new(|| {
    PublicEvents::new(DESCRIPTORS, ADMISSION_SCHEMA).expect("generated resource descriptors")
});
static POLICIES_BY_NAME: LazyLock<paydirt_store::Policies> =
    LazyLock::new(|| serde_json::from_str(POLICIES).expect("generated service policies"));
pub fn record(store: paydirt_store::Store) -> paydirt_store::Result<paydirt_store::Store> {
    let options = serde_json::from_str(include_str!("../resources.json"))
        .map_err(|e| paydirt_store::Fault::invalid(e.to_string()))?;
    let store=store.with_trail(crate::transport::access::OPTIONS.clone(),DESCRIPTORS,ADMISSION_SCHEMA)?;
    store.with_resources(options, DESCRIPTORS, ADMISSION_SCHEMA)
}
pub fn mount(router: Router, db: Database) -> Router {
    if OPTIONS.entities.is_empty() {
        return router;
    }
    ResourceServiceExt::register(Arc::new(Resources(db)), router)
}
struct Resources(Database);
#[allow(refining_impl_trait)]
impl ResourceService for Resources {
    async fn watch(
        &self,
        ctx: RequestContext,
        req: ServiceRequest<'_, WatchRequest>,
    ) -> ServiceResult<ServiceStream<WatchEvent>> {
        let ctx = Database::presence_request(ctx, || req.to_owned_message())?;
        let expr = req.to_owned_message().filter;
        if expr.len() > 16 << 10 {
            return Err(ConnectError::invalid_argument("filter exceeds 16 KiB"));
        }
        #[cfg(all(feature = "resource-cel", not(target_arch = "wasm32")))]
        let filter = (!expr.is_empty())
            .then(|| paydirt_store::resource_filter::Filter::compile(&expr, EVENTS.pool()))
            .transpose()
            .map_err(error)?;
        #[cfg(not(all(feature = "resource-cel", not(target_arch = "wasm32"))))]
        if !expr.is_empty() {
            return Err(ConnectError::unimplemented(
                "this build accepts only an empty resource filter",
            ));
        }
        let db = self.0.clone();
        let ctx = db
            .authorize(ctx, "payday.Resource", "Watch", false, false, false)
            .await?;
        let mut events = db
            .broker
            .as_ref()
            .ok_or_else(|| ConnectError::unavailable("resource broker disabled"))?
            .subscribe(Default::default())
            .await
            .map_err(ConnectError::unavailable)?;
        let lifetime = ctx.clone();
        let stream = async_stream::try_stream! {
            while let Some(batch)=events.next().await{
                for change in &batch.map_err(ConnectError::resource_exhausted)?.changes{
                    if change.resource.is_empty(){continue}
                    let bytes=STANDARD.decode(&change.resource).map_err(|_|ConnectError::internal("invalid resource envelope"))?;
                    let(doc,wire)=EVENTS.public(&bytes).map_err(error)?;
                    let mut allowed=true;
                    for image in ["previous","value"].iter().filter_map(|k|doc.get(k)){
                        let ty=image["@type"].as_str().and_then(|v|v.rsplit('/').next()).ok_or_else(||ConnectError::internal("resource type missing"))?;
                        if !OPTIONS.entities.contains(ty){allowed=false;break}
                        let policy=&POLICIES_BY_NAME[ty].methods["Get"]["policy"];
                        if policy.get("download").is_some(){allowed=false;break}
                        let mut read=match db.authorize_cost(ctx.clone(),ty,"Get",false,policy.get("direct").is_some(),false,0).await{
                            Ok(c)=>c,Err(e) if matches!(e.code,ErrorCode::PermissionDenied|ErrorCode::NotFound|ErrorCode::Unimplemented)=>{allowed=false;break},Err(e)=>Err(e)?
                        };
                        let guard=db.begin(&mut read).await?.ok_or_else(||ConnectError::unauthenticated("resource watch requires a caller"))?;
                        let visible:Result<bool,_>=db.rpc(&read,ty,"$resource_visible",image).await;
                        guard.finish(visible.is_ok()).await?;
                        if !visible?{allowed=false;break}
                    }
                    if !allowed{continue}
                    #[cfg(all(feature="resource-cel",not(target_arch="wasm32")))]
                    if let Some(f)=&filter && !f.matches(&doc,EVENTS.pool()).map_err(error)?{continue}
                    yield WatchEvent::decode_from_slice(&wire).map_err(|_|ConnectError::internal("invalid resource event"))?;
                }
            }
            Err(ConnectError::resource_exhausted("resource subscription ended; reconcile with List"))?;
        };
        Ok(Response::new(Database::enforce_presence(&lifetime, Box::pin(stream))).with_header("paydirt-watch-ready","1"))
    }
}
