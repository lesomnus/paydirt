use crate::{
    proto::app::*,
    services::app::{RobotService, RobotServiceLayer},
    storage::RobotCalls,
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};

pub struct Pair(pub crate::storage::Next);

static ACTIVE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
struct Active;
impl Drop for Active {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[allow(refining_impl_trait)]
impl<N: RobotService> RobotServiceLayer<N> for Pair {
    async fn pause(
        &self,
        _: &N,
        _: RequestContext,
        request: ServiceRequest<'_, StreamNumber>,
    ) -> ServiceResult<StreamNumber> {
        let req = request.to_owned_message();
        futures_timer::Delay::new(std::time::Duration::from_millis(
            req.value.clamp(0, 2000) as u64,
        ))
        .await;
        Response::ok(req)
    }
    async fn invited(
        &self,
        _: &N,
        ctx: RequestContext,
        _: ServiceRequest<'_, RobotAllowanceRequest>,
    ) -> ServiceResult<InvitationCount> {
        use crate::storage::InvitationCalls;
        use paydirt_store::{AccountReads, ReadFilter};
        let sink = self.0.app_invitation();
        let undeclared = sink.list_owned(&ctx, Default::default()).await?;
        if !undeclared.items.is_empty() {
            return Err(connectrpc::ConnectError::internal(
                "undeclared invitations visible",
            ));
        }
        let reads = AccountReads::default().allow(
            "app.Invitation",
            ReadFilter::AccountField("recipient".into()),
        );
        let mine =
            self.0
                .0
                .account_reads(ctx.clone(), "/app.RobotService/Invited", reads.clone())?;
        let found = sink.list_owned(&mine, Default::default()).await?;
        let ids = found
            .items
            .first()
            .map(|v| vec![v.id.clone()])
            .unwrap_or_default();
        let selected = self.0.0.account_reads(
            ctx,
            "/app.RobotService/Invited",
            reads.allow("app.Invitation", ReadFilter::Ids(ids)),
        )?;
        let one = sink.list_owned(&selected, Default::default()).await?;
        Response::ok(InvitationCount {
            mine: Some(found.items.len() as i32),
            selected: Some(one.items.len() as i32),
            ..Default::default()
        })
    }
    async fn in_flight(
        &self,
        _: &N,
        _: RequestContext,
        _: ServiceRequest<'_, StreamNumber>,
    ) -> ServiceResult<StreamNumber> {
        Response::ok(StreamNumber {
            value: ACTIVE.load(std::sync::atomic::Ordering::SeqCst),
            ..Default::default()
        })
    }
    async fn collect(
        &self,
        _: &N,
        _: RequestContext,
        mut input: connectrpc::InboundStream<StreamNumber>,
    ) -> ServiceResult<StreamNumber> {
        use futures_util::StreamExt;
        let mut value = 0;
        while let Some(v) = input.next().await {
            value += v?.to_owned_message().value;
        }
        Response::ok(StreamNumber {
            value,
            ..Default::default()
        })
    }
    async fn converse(
        &self,
        _: &N,
        _: RequestContext,
        mut input: connectrpc::InboundStream<StreamNumber>,
    ) -> ServiceResult<connectrpc::ServiceStream<StreamNumber>> {
        use futures_util::StreamExt;
        let stream = async_stream::try_stream! {
            ACTIVE.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            let _active=Active;
            while let Some(v)=input.next().await {
                yield StreamNumber{value:2*v?.to_owned_message().value,..Default::default()};
            }
        };
        Response::ok(Box::pin(stream))
    }
    async fn allowance(
        &self,
        _: &N,
        ctx: RequestContext,
        request: ServiceRequest<'_, RobotAllowanceRequest>,
    ) -> ServiceResult<RobotAllowanceResponse> {
        let req = request.to_owned_message();
        let value = self
            .0
            .0
            .entitlement(
                &ctx,
                "app.Robot",
                req.key.as_deref().unwrap_or_default(),
                req.lock.unwrap_or_default(),
            )
            .await?;
        Response::ok(RobotAllowanceResponse {
            enabled: Some(value.has()),
            allows_two: Some(value.allows(2)),
            ..Default::default()
        })
    }
    async fn rename_pair(
        &self,
        next: &N,
        ctx: RequestContext,
        request: ServiceRequest<'_, RobotRenamePairRequest>,
    ) -> ServiceResult<Robot> {
        let r = request.to_owned_message();
        if r.first_alias.is_none() && r.second_alias.is_none() {
            use paydirt_store::{Fault, Violation};
            return Err(crate::transport::error(
                Fault::validation([
                    Violation::new("first_alias", "first name required"),
                    Violation::new("second_alias", "second name required"),
                ])
                .at("pair"),
            ));
        }
        use paydirt::page::StreamExt;
        let robots = next.list_all(
            &ctx,
            &RobotListRequest {
                size: 1,
                ..Default::default()
            },
        );
        let mut robots = std::pin::pin!(robots);
        if let Some(first) = robots.next().await {
            let first = first?;
            let header = crate::transport::header(&first).map_err(crate::transport::error)?;
            assert_eq!(
                crate::transport::domains().lookup("app.Robot"),
                Some(header.id[9])
            );
        }
        next.patch_owned(
            &ctx,
            RobotPatchRequest {
                r#ref: r.first,
                alias: r.first_alias,
                date_updated_force: Some(true),
                ..Default::default()
            },
        )
        .await?;
        Response::ok(
            next.patch_owned(
                &ctx,
                RobotPatchRequest {
                    r#ref: r.second,
                    alias: r.second_alias,
                    date_updated_force: Some(true),
                    ..Default::default()
                },
            )
            .await?,
        )
    }
}
