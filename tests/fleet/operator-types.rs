#[cfg(test)]
mod operator_tests {
    use super::*;
    #[test]
    fn uuid_options_distinguish_trace_bytes_from_identifiers() {
        let schema: Value = serde_json::from_str(fleet::transport::ADMISSION_SCHEMA).unwrap();
        let bytes = STANDARD.encode([0xfe; 16]);
        let mut audit = json!({"id":bytes,"traceId":bytes});
        translate(&mut audit, "app.Audit", &schema, false, None).unwrap();
        assert_eq!(audit["traceId"], bytes);
        assert_eq!(audit["id"], "fefefefe-fefe-fefe-fefe-fefefefefefe");
        translate(&mut audit, "app.Audit", &schema, true, None).unwrap();
        assert_eq!(audit["id"], bytes);
        let mut shown = json!({"id":bytes,"traceId":bytes});
        translate(&mut shown, "app.Audit", &schema, false, None).unwrap();
        display::pretty(&mut shown, "app.Audit", &schema);
        assert_eq!(shown["traceId"], "(16 bytes)");
        assert_eq!(shown["id"], "fefefefe-fefe-fefe-fefe-fefefefefefe");
    }
}

#[cfg(test)]
mod operator_tree_tests {
    use super::*;
    use crate::tree::{Command, Tree};
    fn rpc() -> Command {
        Command::Rpc {
            entity: "app.Robot".into(),
            rpc: "Get".into(),
        }
    }
    #[test]
    fn package_selection_alias_collisions_and_narrowing_follow_the_schema() {
        let policies: paydirt_store::Policies = serde_json::from_str(POLICIES).unwrap();
        assert!(Tree::packages(&policies).is_superset(&["app", "shared"].into()));
        assert!(
            Tree::new_in(&policies, &["payday"])
                .unwrap()
                .complete("")
                .is_empty()
        );
        let t = Tree::new_in(&policies, &["shared"]).unwrap();
        assert!(t.complete("").contains(&"thing".into()));
        assert!(t.complete("robot").is_empty());
        let mut t = Tree::new(&policies).unwrap();
        for verb in ["get", "list", "watch", "add", "patch", "erase"] {
            assert!(t.complete("robot").contains(&verb.into()));
        }
        assert!(!t.complete("cell").contains(&"list".into()));
        t.aliases("tenant", &["ws"]).unwrap();
        assert_eq!(t.complete("ws"), t.complete("tenant"));
        assert!(!t.complete("").contains(&"ws".into()));
        assert!(t.complete("robot/ls").contains(&"--next".into()));
        assert!(!t.complete("robot").contains(&"ls".into()));
        for (path, words) in [
            ("tenant", vec!["robot"]),
            ("robot/get", vec!["list"]),
            ("robot", vec!["ws"]),
            ("robot/get", vec!["ls"]),
            ("unknown", vec!["ws"]),
            ("robot", vec![]),
            ("robot", vec!["valid", "two words"]),
        ] {
            assert!(t.aliases(path, &words).is_err());
        }
        assert!(t.select(&["valid".into(), "get".into()]).is_err());
        t.aliases("robot/list", &["ls"]).unwrap();
        for route in ["robot/ls", "ws", "robot/get", "robot/get/child"] {
            assert!(t.add(route, rpc()).is_err());
        }
        assert!(t.add_with_aliases("robot/count", rpc(), &["get"]).is_err());
        assert!(!t.complete("robot").contains(&"count".into()));
        assert!(
            t.replace_with_aliases("robot/erase", rpc(), &["get"])
                .is_err()
        );
        assert_eq!(
            t.complete_words(&["robot".into(), "ls".into(), "--n".into()]),
            ["--next"]
        );
        assert!(t.hide("missing").is_err());
        assert!(t.replace("cell/list", rpc()).is_err());
        assert!(t.replace("holder/gett", rpc()).is_err());
        t.drop_path("holder/erase").unwrap();
        assert!(t.select(&["holder".into(), "erase".into()]).is_err());
        assert!(t.complete("holder").contains(&"get".into()));
        t.drop_path("audit").unwrap();
        assert!(!t.complete("").contains(&"audit".into()));
        assert!(t.select(&["app.audit".into(), "get".into()]).is_err());
    }
    #[tokio::test]
    async fn commands_open_and_release_their_connector_once_and_return_its_errors() {
        use std::cell::Cell;
        struct Close<'a>(&'a Cell<usize>);
        impl Drop for Close<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        let policies = serde_json::from_str(POLICIES).unwrap();
        let mut tree = Tree::new(&policies).unwrap();
        tree.add(
            "local",
            Command::Custom(|_, _, _| Box::pin(async { Ok(()) })),
        )
        .unwrap();
        let opened = Cell::new(0);
        let closed = Cell::new(0);
        let connect = || {
            opened.set(opened.get() + 1);
            let (c, v) = rpc_client("http://127.0.0.1:1", None)?;
            Ok((c, v, Close(&closed)))
        };
        run_tree(
            &["robot".into(), "--help".into()],
            &policies,
            &tree,
            connect,
        )
        .await
        .unwrap();
        assert_eq!(opened.get(), 0);
        run_tree(&["local".into()], &policies, &tree, connect)
            .await
            .unwrap();
        assert_eq!((opened.get(), closed.get()), (1, 1));
        let e = run_tree::<()>(&["local".into()], &policies, &tree, || {
            Err("server.addr names nothing".into())
        })
        .await
        .unwrap_err();
        assert_eq!(e.to_string(), "server.addr names nothing");
    }
}
