use crate::{
    proto::app::*,
    services::app::{RobotService, RobotServiceLayer},
    storage::{CellCalls, Next, RobotCalls},
};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};

pub struct Fleet(pub Next);

#[allow(refining_impl_trait)]
impl<N: RobotService> RobotServiceLayer<N> for Fleet {
    async fn r#move(
        &self,
        next: &N,
        ctx: RequestContext,
        request: ServiceRequest<'_, RobotMoveRequest>,
    ) -> ServiceResult<Robot> {
        let request = request.to_owned_message();
        let robot = next
            .get_owned(
                &ctx,
                RobotGetRequest {
                    r#ref: request.r#ref.clone(),
                    ..Default::default()
                },
            )
            .await?;
        let mut patch = RobotPatchRequest {
            r#ref: request.r#ref,
            date_updated: robot.date_updated,
            ..Default::default()
        };
        if let Some(dest) = request.to.into_option() {
            let cell = self
                .0
                .app_cell()
                .get_owned(
                    &ctx,
                    CellGetRequest {
                        r#ref: dest.clone().into(),
                        ..Default::default()
                    },
                )
                .await?;
            if robot.cell.as_option().is_some_and(|old| old.id == cell.id) {
                return Err(ConnectError::invalid_argument(
                    "the robot is already in that cell",
                ));
            }
            patch.cell = dest.into();
        } else {
            patch.cell_null = Some(true);
        }
        Response::ok(next.patch_owned(&ctx, patch).await?)
    }
}
