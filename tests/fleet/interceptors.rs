use crate::{
    proto::app::Robot,
    storage::intercept::{Hook, Layer},
};
use buffa::Message;
use connectrpc::{ConnectError, RequestContext};
use std::sync::Arc;

pub fn row() -> Robot {
    Robot {
        alias: "public".into(),
        secret: b"sample-secret".to_vec(),
        ..Default::default()
    }
}
pub fn layers() -> Layer {
    Layer::new(vec![Arc::new(Mark("outer")), Arc::new(Mark("inner"))])
}
#[derive(Clone)]
struct Order(Vec<&'static str>);
struct Mark(&'static str);
impl Hook for Mark {
    fn enter(&self, method: &'static str, ctx: &mut RequestContext) -> Result<(), ConnectError> {
        if method.starts_with("/app.RobotService/Reveal") {
            let expected = if self.0 == "outer" {
                vec![]
            } else {
                vec!["outer"]
            };
            let mut order = ctx
                .extensions()
                .get::<Order>()
                .cloned()
                .unwrap_or(Order(vec![]));
            if order.0 != expected {
                return Err(ConnectError::internal("enter order"));
            }
            order.0.push(self.0);
            ctx.extensions_mut().insert(order);
        }
        Ok(())
    }
    fn send(
        &self,
        method: &'static str,
        ctx: &RequestContext,
        bytes: &mut Vec<u8>,
    ) -> Result<(), ConnectError> {
        if method.starts_with("/app.RobotService/Reveal") {
            if ctx.extensions().get::<Order>().unwrap().0 != ["outer", "inner"] {
                return Err(ConnectError::internal("missing enter context"));
            }
            let mut row = Robot::decode_from_slice(bytes)
                .map_err(|_| ConnectError::internal("invalid row"))?;
            row.alias.push(':');
            row.alias.push_str(self.0);
            row.secret = b"hook supplied secret".to_vec();
            *bytes = row.encode_to_vec();
        }
        Ok(())
    }
}
