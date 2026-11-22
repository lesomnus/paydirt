use crate::transport::{
    Database,
    presence::{Authentication, Lease, Person},
};
use chrono::{TimeDelta, Utc};
use std::sync::Arc;

/// Reference app enrollment. Operations passes verified device, badge and revocation state.
pub fn install(db: Database) -> Database {
    let (Ok(device), Ok(person), Ok(revocation)) = (
        std::env::var("FLEET_DEVICE"),
        std::env::var("FLEET_BADGE"),
        std::env::var("FLEET_REVOCATION"),
    ) else {
        return db;
    };
    let device = paydirt::parse_id(&device).unwrap();
    let person = paydirt::parse_id(&person).unwrap();
    db.with_presence(move |claim| {
        let deny = || {
            connectrpc::ConnectError::permission_denied(
                "badge enrollment does not authorize this claim",
            )
        };
        if claim.caller.actor.as_deref() != Some(device.as_slice())
            || person != claim.subject
            || claim.authentication.method != "oidc"
        {
            return Err(deny());
        }
        let now = Utc::now();
        let mut authenticated = now;
        let mut until = now + TimeDelta::minutes(15);
        match claim.reference {
            "live" if claim.recorded.is_none() => {}
            "short" if claim.recorded.is_none() => until = now + TimeDelta::milliseconds(600),
            "stale" if claim.recorded.is_none() => authenticated = now - TimeDelta::hours(1),
            "offline"
                if claim
                    .recorded
                    .is_some_and(|v| v.to_rfc3339() == "2026-01-03T12:00:00+00:00")
                    && claim.method == "/app.RobotService/Add"
                    && claim
                        .request
                        .is_some_and(|v| v["alias"] == "offline-signed") => {}
            _ => return Err(deny()),
        }
        let path = revocation.clone();
        let mut frame = claim.caller.clone();
        frame.actor = Some(person.to_vec());
        Ok(Person {
            frame,
            name: Some("badge-user".into()),
            methods: Some(
                ["Recent", "PresenceWait", "Add"]
                    .map(|m| format!("/app.RobotService/{m}"))
                    .to_vec(),
            ),
            authentication: Authentication {
                method: "badge".into(),
                at: Some(authenticated),
            },
            lease: Lease {
                until,
                active: Arc::new(move || !std::path::Path::new(&path).exists()),
            },
        })
    })
}
