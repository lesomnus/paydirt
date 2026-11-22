use crate::{
    proto::app::*,
    services::app::{SettingService, SettingServiceLayer},
};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};

pub struct Settings {
    pub name: String,
}

// The service setting has a stable domain-77 Payday ID across restarts.
const SERVICE_ID: [u8; 16] = [0, 0, 0, 0, 0, 0, 0x80, 0, 0x80, 77, 0, 0, 0, 0, 0, 1];

#[allow(refining_impl_trait)]
impl<N: SettingService> SettingServiceLayer<N> for Settings {
    async fn get(
        &self,
        _: &N,
        _: RequestContext,
        request: ServiceRequest<'_, SettingGetRequest>,
    ) -> ServiceResult<Setting> {
        let request = request.to_owned_message();
        let found = match &request.r#ref.key {
            Some(setting_ref::Key::Id(id)) => id == &SERVICE_ID,
            Some(setting_ref::Key::Alias(alias)) => alias == "service",
            None => return Err(ConnectError::invalid_argument("setting reference required")),
        };
        if !found {
            return Err(ConnectError::not_found("setting not found"));
        }
        let selection = request.select.as_option();
        let all = selection.is_none_or(|s| s.all.unwrap_or(false));
        Response::ok(Setting {
            id: SERVICE_ID.to_vec(),
            alias: if all || selection.is_some_and(|s| s.alias.unwrap_or(false)) {
                "service".into()
            } else {
                String::new()
            },
            value: if all || selection.is_some_and(|s| s.value.unwrap_or(false)) {
                self.name.clone()
            } else {
                String::new()
            },
            ..Default::default()
        })
    }
}
