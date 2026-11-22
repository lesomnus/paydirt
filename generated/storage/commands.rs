async fn dispatch(
    client: &HttpClient,
    config: &ClientConfig,
    entity: &str,
    rpc: &str,
    request: Value,
    mut emit: impl FnMut(Value) -> Result<()>,
) -> Result<()> {
    use connectrpc::client::{call_unary, call_server_stream};
    match (entity, rpc) {
        ("app.Access", "Get") => {
            let res = call_unary::<
                _,
                proto::r#app::r#AccessGetRequest,
                <proto::r#app::r#Access as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.AccessService",
                    "Get",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Access", "List") => {
            let res = call_unary::<
                _,
                proto::r#app::r#AccessListRequest,
                <proto::r#app::r#AccessListResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.AccessService",
                    "List",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Audit", "Add") => {
            let res = call_unary::<
                _,
                proto::r#app::r#AuditAddRequest,
                <proto::r#app::r#Audit as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.AuditService",
                    "Add",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Audit", "Apply") => {
            let res = call_unary::<
                _,
                proto::r#app::r#AuditApplyRequest,
                <proto::r#app::r#Audit as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.AuditService",
                    "Apply",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Audit", "Erase") => {
            let res = call_unary::<
                _,
                proto::r#app::r#AuditRef,
                <proto::r#app::r#AuditEraseResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.AuditService",
                    "Erase",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Audit", "Get") => {
            let res = call_unary::<
                _,
                proto::r#app::r#AuditGetRequest,
                <proto::r#app::r#Audit as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.AuditService",
                    "Get",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Audit", "List") => {
            let res = call_unary::<
                _,
                proto::r#app::r#AuditListRequest,
                <proto::r#app::r#AuditListResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.AuditService",
                    "List",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Audit", "Patch") => {
            let res = call_unary::<
                _,
                proto::r#app::r#AuditPatchRequest,
                <proto::r#app::r#Audit as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.AuditService",
                    "Patch",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Cell", "Add") => {
            let res = call_unary::<
                _,
                proto::r#app::r#CellAddRequest,
                <proto::r#app::r#Cell as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.CellService",
                    "Add",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Cell", "Apply") => {
            let res = call_unary::<
                _,
                proto::r#app::r#CellApplyRequest,
                <proto::r#app::r#Cell as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.CellService",
                    "Apply",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Cell", "Erase") => {
            let res = call_unary::<
                _,
                proto::r#app::r#CellRef,
                <proto::r#app::r#CellEraseResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.CellService",
                    "Erase",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Cell", "Get") => {
            let res = call_unary::<
                _,
                proto::r#app::r#CellGetRequest,
                <proto::r#app::r#Cell as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.CellService",
                    "Get",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Cell", "Patch") => {
            let res = call_unary::<
                _,
                proto::r#app::r#CellPatchRequest,
                <proto::r#app::r#Cell as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.CellService",
                    "Patch",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Holder", "Add") => {
            let res = call_unary::<
                _,
                proto::r#app::r#HolderAddRequest,
                <proto::r#app::r#Holder as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.HolderService",
                    "Add",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Holder", "Apply") => {
            let res = call_unary::<
                _,
                proto::r#app::r#HolderApplyRequest,
                <proto::r#app::r#Holder as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.HolderService",
                    "Apply",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Holder", "Erase") => {
            let res = call_unary::<
                _,
                proto::r#app::r#HolderRef,
                <proto::r#app::r#HolderEraseResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.HolderService",
                    "Erase",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Holder", "Get") => {
            let res = call_unary::<
                _,
                proto::r#app::r#HolderGetRequest,
                <proto::r#app::r#Holder as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.HolderService",
                    "Get",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Holder", "List") => {
            let res = call_unary::<
                _,
                proto::r#app::r#HolderListRequest,
                <proto::r#app::r#HolderListResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.HolderService",
                    "List",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Holder", "Patch") => {
            let res = call_unary::<
                _,
                proto::r#app::r#HolderPatchRequest,
                <proto::r#app::r#Holder as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.HolderService",
                    "Patch",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Holder", "Watch") => {
            let mut stream = call_server_stream::<
                _,
                proto::r#app::r#HolderWatchRequest,
                <proto::r#app::r#HolderWatchResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.HolderService",
                    "Watch",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            while let Some(msg) = stream
                .message::<proto::r#app::r#HolderWatchResponse>()
                .await?
            {
                emit(serde_json::to_value(msg.to_owned_message())?)?;
            }
        }
        ("app.Robot", "Add") => {
            let res = call_unary::<
                _,
                proto::r#app::r#RobotAddRequest,
                <proto::r#app::r#Robot as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.RobotService",
                    "Add",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Robot", "AddMany") => {
            let res = call_unary::<
                _,
                proto::r#app::r#RobotAddManyRequest,
                <proto::r#app::r#RobotAddManyResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.RobotService",
                    "AddMany",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Robot", "Apply") => {
            let res = call_unary::<
                _,
                proto::r#app::r#RobotApplyRequest,
                <proto::r#app::r#Robot as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.RobotService",
                    "Apply",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Robot", "Erase") => {
            let res = call_unary::<
                _,
                proto::r#app::r#RobotRef,
                <proto::r#app::r#RobotEraseResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.RobotService",
                    "Erase",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Robot", "Get") => {
            let res = call_unary::<
                _,
                proto::r#app::r#RobotGetRequest,
                <proto::r#app::r#Robot as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.RobotService",
                    "Get",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Robot", "List") => {
            let res = call_unary::<
                _,
                proto::r#app::r#RobotListRequest,
                <proto::r#app::r#RobotListResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.RobotService",
                    "List",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Robot", "Move") => {
            let res = call_unary::<
                _,
                proto::r#app::r#RobotMoveRequest,
                <proto::r#app::r#Robot as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.RobotService",
                    "Move",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Robot", "Patch") => {
            let res = call_unary::<
                _,
                proto::r#app::r#RobotPatchRequest,
                <proto::r#app::r#Robot as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.RobotService",
                    "Patch",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Robot", "Watch") => {
            let mut stream = call_server_stream::<
                _,
                proto::r#app::r#RobotWatchRequest,
                <proto::r#app::r#RobotWatchResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.RobotService",
                    "Watch",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            while let Some(msg) = stream
                .message::<proto::r#app::r#RobotWatchResponse>()
                .await?
            {
                emit(serde_json::to_value(msg.to_owned_message())?)?;
            }
        }
        ("app.Tenant", "Add") => {
            let res = call_unary::<
                _,
                proto::r#app::r#TenantAddRequest,
                <proto::r#app::r#Tenant as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.TenantService",
                    "Add",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Tenant", "Apply") => {
            let res = call_unary::<
                _,
                proto::r#app::r#TenantApplyRequest,
                <proto::r#app::r#Tenant as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.TenantService",
                    "Apply",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Tenant", "Erase") => {
            let res = call_unary::<
                _,
                proto::r#app::r#TenantRef,
                <proto::r#app::r#TenantEraseResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.TenantService",
                    "Erase",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Tenant", "Get") => {
            let res = call_unary::<
                _,
                proto::r#app::r#TenantGetRequest,
                <proto::r#app::r#Tenant as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.TenantService",
                    "Get",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Tenant", "List") => {
            let res = call_unary::<
                _,
                proto::r#app::r#TenantListRequest,
                <proto::r#app::r#TenantListResponse as buffa::HasMessageView>::View<
                    'static,
                >,
            >(
                    client,
                    config,
                    "app.TenantService",
                    "List",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        ("app.Tenant", "Patch") => {
            let res = call_unary::<
                _,
                proto::r#app::r#TenantPatchRequest,
                <proto::r#app::r#Tenant as buffa::HasMessageView>::View<'static>,
            >(
                    client,
                    config,
                    "app.TenantService",
                    "Patch",
                    serde_json::from_value(request)?,
                    Default::default(),
                )
                .await?;
            emit(serde_json::to_value(res.into_owned())?)?;
        }
        _ => return Err("unsupported command".into()),
    }
    Ok(())
}
