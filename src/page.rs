//! Lazy traversal. Dropping the stream cancels an in-flight List or stops before the next one.
pub use futures_util::{Stream, StreamExt};
use std::{collections::HashSet, future::Future};

pub fn all<Q: Clone, T, E, F, Fut, C, R>(
    request: &Q,
    cursor: C,
    mut list: F,
    repeated: R,
) -> impl Stream<Item = Result<T, E>> + use<Q, T, E, F, Fut, C, R>
where
    C: Fn(&mut Q) -> &mut String,
    R: FnOnce() -> E,
    F: FnMut(Q) -> Fut,
    Fut: Future<Output = Result<(Vec<T>, String), E>>,
{
    let mut req = request.clone();
    async_stream::try_stream! {
        let mut seen=HashSet::from([cursor(&mut req).clone()]);
        loop {
            let (items,next)=list(req.clone()).await?;
            for item in items {yield item;}
            if next.is_empty(){break;}
            if !seen.insert(next.clone()){Err(repeated())?;break;}
            *cursor(&mut req)=next;
        }
    }
}
