use crate::client::{AppId, ClientId, ClientProfile};

pub fn browser_hostile_client(
    client_id: impl Into<ClientId>,
    app_id: impl Into<AppId>,
) -> ClientProfile {
    ClientProfile::browser(client_id, app_id)
}

pub fn browser_hostile_policy_note() -> &'static str {
    "Browser renderer is hostile by default; only explicitly granted scopes may cross boundaries."
}
