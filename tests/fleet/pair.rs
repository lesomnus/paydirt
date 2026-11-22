use crate::{
    proto::app::*,
    services::app::{RobotService, RobotServiceLayer},
    storage::RobotCalls,
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};

pub struct Pair(pub crate::storage::Next);

#[allow(refining_impl_trait)]
impl<N: RobotService> RobotServiceLayer<N> for Pair {
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
