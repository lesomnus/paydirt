use crate::{proto::app::*, services::app::ComputedReadingService};
use connectrpc::{
    ConnectError, RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream,
};

// This fixture serves public synthetic readings, independent of the entity database.
pub struct PublicReadings;

fn reading(n: u8) -> ComputedReading {
    ComputedReading {
        id: vec![n],
        value: format!("reading-{n}"),
        ..Default::default()
    }
}
fn snapshot(revision: &str) -> ComputedReadingSnapshot {
    ComputedReadingSnapshot {
        revision: revision.into(),
        ..Default::default()
    }
}
#[allow(refining_impl_trait)]
impl ComputedReadingService for PublicReadings {
    async fn get(
        &self,
        _: RequestContext,
        req: ServiceRequest<'_, ComputedReadingGetRequest>,
    ) -> ServiceResult<ComputedReading> {
        match req.to_owned_message().r#ref.id.as_slice() {
            &[n @ (1 | 2)] => Response::ok(reading(n)),
            _ => Err(ConnectError::not_found("reading not found")),
        }
    }
    async fn list(
        &self,
        _: RequestContext,
        req: ServiceRequest<'_, ComputedReadingListRequest>,
    ) -> ServiceResult<ComputedReadingListResponse> {
        let req = req.to_owned_message();
        let (items, next) = match (req.after.as_str(), req.size) {
            ("", 1) => (vec![reading(1)], "second"),
            ("", _) => (vec![reading(1), reading(2)], ""),
            ("second", _) => (vec![reading(2)], ""),
            _ => return Err(ConnectError::invalid_argument("unknown cursor")),
        };
        Response::ok(ComputedReadingListResponse {
            items,
            next: next.into(),
            metadata: snapshot("v1").into(),
            ..Default::default()
        })
    }
    async fn watch(
        &self,
        _: RequestContext,
        req: ServiceRequest<'_, ComputedReadingWatchRequest>,
    ) -> ServiceResult<ServiceStream<ComputedReadingWatchResponse>> {
        let revision = req.to_owned_message().revision;
        let stream = async_stream::try_stream! {
            if revision.is_empty() {
                yield ComputedReadingWatchResponse{items:vec![reading(1),reading(2)],metadata:snapshot("v1").into(),..Default::default()};
            }
            yield ComputedReadingWatchResponse{metadata:snapshot("v2").into(),..Default::default()};
        };
        Response::ok(Box::pin(stream))
    }
}
