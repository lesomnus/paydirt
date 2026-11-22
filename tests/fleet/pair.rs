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
    async fn journal(
        &self,
        _: &N,
        ctx: RequestContext,
        req: ServiceRequest<'_, RobotJournalRequest>,
    ) -> ServiceResult<RobotJournalReply> {
        use crate::transport::past::{Begin, Options, detach};
        let req = req.to_owned_message();
        let id: [u8; 16] = req
            .id
            .as_slice()
            .try_into()
            .map_err(|_| connectrpc::ConnectError::invalid_argument("event ID required"))?;
        if req.redact {
            self.0.0.redact_event(&ctx, "app.Robot", id).await?;
            return Response::ok(Default::default());
        }
        let subject = req
            .subject
            .as_slice()
            .try_into()
            .map_err(|_| connectrpc::ConnectError::invalid_argument("subject required"))?;
        let payload = RobotMoved {
            robot: req.subject.clone(),
            ..Default::default()
        };
        let begin = self
            .0
            .0
            .begin_event(
                &ctx,
                "app.Robot",
                &payload,
                Options {
                    id: Some(id),
                    subject: Some(subject),
                    reason: "manual review".into(),
                    occurred: Some("2026-11-01T00:00:00Z".parse().unwrap()),
                    ..Default::default()
                },
            )
            .await?;
        let Begin::Started(root) = begin else {
            return Response::ok(RobotJournalReply {
                done: true,
                root: id.to_vec(),
                ..Default::default()
            });
        };
        let make = || Options {
            subject: Some(subject),
            ..Default::default()
        };
        let Begin::Started(child) = self
            .0
            .0
            .begin_event(root.context(), "app.Robot", &payload, make())
            .await?
        else {
            unreachable!()
        };
        let Begin::Started(detached) = self
            .0
            .0
            .begin_event(&detach(root.context()), "app.Robot", &payload, make())
            .await?
        else {
            unreachable!()
        };
        if req.fail {
            return Err(connectrpc::ConnectError::aborted("roll back operation"));
        }
        Response::ok(RobotJournalReply {
            root: id.to_vec(),
            child: child.id.to_vec(),
            detached: detached.id.to_vec(),
            ..Default::default()
        })
    }
    async fn describe(
        &self,
        _: &N,
        ctx: RequestContext,
        req: ServiceRequest<'_, RobotJournalRequest>,
    ) -> ServiceResult<RobotJournalReply> {
        use crate::storage::EventCalls;
        use crate::transport::past::Catalog;
        use base64::{Engine, engine::general_purpose::STANDARD};
        let req = req.to_owned_message();
        let event = self
            .0
            .app_event()
            .get_owned(
                &ctx,
                EventGetRequest {
                    r#ref: EventRef {
                        key: Some(event_ref::Key::Id(req.id)),
                        ..Default::default()
                    }
                    .into(),
                    ..Default::default()
                },
            )
            .await?;
        let mut catalog = Catalog::new("en");
        for lang in ["en", "ko"] {
            let layer = self.0.clone();
            catalog.add("robot.move", lang, move |reading, row| {
                let layer = layer.clone();
                async move {
                    let id = STANDARD.decode(row["subjectId"].as_str().unwrap()).unwrap();
                    let robot = layer
                        .app_robot()
                        .get_owned(
                            &reading.ctx,
                            RobotGetRequest {
                                r#ref: RobotRef {
                                    key: Some(robot_ref::Key::Id(id)),
                                    ..Default::default()
                                }
                                .into(),
                                ..Default::default()
                            },
                        )
                        .await?;
                    let at = row["dateOccurred"]
                        .as_str()
                        .unwrap()
                        .parse::<chrono::DateTime<chrono::Utc>>()
                        .unwrap()
                        .with_timezone(&reading.zone);
                    Ok(format!(
                        "{} {} {}",
                        reading.language,
                        robot.alias,
                        at.format("%H:%M")
                    ))
                }
            });
        }
        catalog
            .validate(&["robot.move"])
            .map_err(connectrpc::ConnectError::internal)?;
        let pref = ctx
            .header("accept-language")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("en");
        let sentence = catalog
            .describe(
                &ctx,
                serde_json::to_value(event).unwrap(),
                pref,
                "Asia/Seoul".parse().unwrap(),
            )
            .await?;
        Response::ok(RobotJournalReply {
            sentence,
            ..Default::default()
        })
    }
    async fn export_robot(
        &self,
        next: &N,
        ctx: RequestContext,
        request: ServiceRequest<'_, RobotGetRequest>,
    ) -> ServiceResult<Robot> {
        Response::ok(next.get_owned(&ctx, request.to_owned_message()).await?)
    }
    async fn recent(
        &self,
        _: &N,
        ctx: RequestContext,
        _: ServiceRequest<'_, StreamNumber>,
    ) -> ServiceResult<StreamNumber> {
        let parties = ctx
            .extensions()
            .get::<crate::transport::presence::Parties>()
            .ok_or_else(|| connectrpc::ConnectError::unauthenticated("no authenticated person"))?;
        let now = chrono::Utc::now();
        let age = std::time::Duration::from_secs(60);
        if parties.actor.method != "badge"
            || !parties.actor.fresh(now, age)
            || !parties
                .caller
                .as_ref()
                .is_some_and(|c| c.method == "oidc" && c.fresh(now, age))
        {
            return Err(connectrpc::ConnectError::permission_denied(
                "recent proof from person and device required",
            ));
        }
        Response::ok(StreamNumber {
            value: 1,
            ..Default::default()
        })
    }
    async fn presence_wait(
        &self,
        _: &N,
        _: RequestContext,
        _: ServiceRequest<'_, StreamNumber>,
    ) -> ServiceResult<connectrpc::ServiceStream<StreamNumber>> {
        Response::ok(Box::pin(
            async_stream::stream! { yield Ok(StreamNumber{value:1,..Default::default()}); std::future::pending::<()>().await; },
        ))
    }
    #[cfg(not(target_arch = "wasm32"))]
    async fn who(
        &self,
        _: &N,
        ctx: RequestContext,
        _: ServiceRequest<'_, StreamNumber>,
    ) -> ServiceResult<StreamNumber> {
        let e = self.0.0.evidence(&ctx).ok_or_else(|| {
            connectrpc::ConnectError::unauthenticated("missing provider evidence")
        })?;
        let value = match e.method.as_str() {
            "mtls" => 1,
            "oidc" => 2,
            "token" => 3,
            _ => 0,
        };
        Response::ok(StreamNumber {
            value,
            ..Default::default()
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    async fn login(
        &self,
        _: &N,
        ctx: RequestContext,
        req: ServiceRequest<'_, StreamNumber>,
    ) -> ServiceResult<StreamNumber> {
        let e = self
            .0
            .0
            .evidence(&ctx)
            .ok_or_else(|| connectrpc::ConnectError::unauthenticated("missing login proof"))?;
        let ttl = std::time::Duration::from_secs(req.value.clamp(1, 60) as u64);
        let cookie = self.0.0.login_cookie(&ctx, &e.binding, ttl)?;
        Ok(Response::new(StreamNumber::default()).with_header("set-cookie", cookie))
    }
    #[cfg(not(target_arch = "wasm32"))]
    async fn logout(
        &self,
        _: &N,
        ctx: RequestContext,
        _: ServiceRequest<'_, StreamNumber>,
    ) -> ServiceResult<StreamNumber> {
        Ok(Response::new(StreamNumber::default())
            .with_header("set-cookie", self.0.0.logout_cookie(&ctx)?))
    }
    async fn pause(
        &self,
        _: &N,
        _: RequestContext,
        request: ServiceRequest<'_, StreamNumber>,
    ) -> ServiceResult<StreamNumber> {
        let req = request.to_owned_message();
        futures_timer::Delay::new(std::time::Duration::from_millis(
            req.value.clamp(0, 2000) as u64
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
