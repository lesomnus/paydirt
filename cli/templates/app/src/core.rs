use crate::services::app::{SettingService, SettingServiceLayer};

pub struct Core;

// Implement individual RPCs here to add application behavior.
// The generated defaults forward other methods to the next service.
impl<N: SettingService> SettingServiceLayer<N> for Core {}
