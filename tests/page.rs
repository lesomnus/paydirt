use futures_util::{StreamExt, pin_mut};
use paydirt::page;
use std::cell::RefCell;

#[tokio::test]
async fn starts_at_the_callers_cursor_preserves_filters_and_reports_a_cycle_once() {
    let request = ("filter", String::from("start"));
    let visited = RefCell::new(Vec::new());
    let pages = page::all(
        &request,
        |q| &mut q.1,
        |q| {
            visited.borrow_mut().push(q.clone());
            std::future::ready(Ok(match q.1.as_str() {
                "start" => (vec![], "more".into()),
                _ => (vec![2, 3], "start".into()),
            }))
        },
        || "cycle",
    );
    assert_eq!(
        pages.collect::<Vec<_>>().await,
        [Ok(2), Ok(3), Err("cycle")]
    );
    assert_eq!(request.1, "start");
    assert_eq!(
        *visited.borrow(),
        [("filter", "start".into()), ("filter", "more".into())]
    );
}

#[tokio::test]
async fn drop_or_failure_does_not_fetch_an_extra_page() {
    let calls = std::cell::Cell::new(0);
    let walk = || {
        page::all(
            &String::new(),
            |q| q,
            |q| {
                calls.set(calls.get() + 1);
                std::future::ready(if q.is_empty() {
                    Ok((vec![1, 2], "next".into()))
                } else {
                    Err("unavailable")
                })
            },
            || "cycle",
        )
    };
    {
        let stream = walk();
        pin_mut!(stream);
        assert_eq!(stream.next().await, Some(Ok(1)));
    }
    assert_eq!(calls.get(), 1);
    assert_eq!(
        walk().collect::<Vec<_>>().await,
        [Ok(1), Ok(2), Err("unavailable")]
    );
    assert_eq!(calls.get(), 3);
}
