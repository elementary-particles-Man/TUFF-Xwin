use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClientId(String);

impl ClientId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ClientId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ClientId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SurfaceId(String);

impl SurfaceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SurfaceId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SurfaceId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppId(String);

impl AppId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for AppId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for AppId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientKind {
    Browser,
    NativeApp,
    SystemApp,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientTrust {
    TrustedSystem,
    UserApp,
    Hostile,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientProfile {
    pub client_id: ClientId,
    pub app_id: AppId,
    pub primary_surface: Option<SurfaceId>,
    pub kind: ClientKind,
    pub trust: ClientTrust,
}

impl ClientProfile {
    pub fn browser(client_id: impl Into<ClientId>, app_id: impl Into<AppId>) -> Self {
        Self {
            client_id: client_id.into(),
            app_id: app_id.into(),
            primary_surface: None,
            kind: ClientKind::Browser,
            trust: ClientTrust::Hostile,
        }
    }

    pub fn native_app(client_id: impl Into<ClientId>, app_id: impl Into<AppId>) -> Self {
        Self {
            client_id: client_id.into(),
            app_id: app_id.into(),
            primary_surface: None,
            kind: ClientKind::NativeApp,
            trust: ClientTrust::UserApp,
        }
    }

    pub fn system_app(client_id: impl Into<ClientId>, app_id: impl Into<AppId>) -> Self {
        Self {
            client_id: client_id.into(),
            app_id: app_id.into(),
            primary_surface: None,
            kind: ClientKind::SystemApp,
            trust: ClientTrust::TrustedSystem,
        }
    }

    pub fn unknown(client_id: impl Into<ClientId>, app_id: impl Into<AppId>) -> Self {
        Self {
            client_id: client_id.into(),
            app_id: app_id.into(),
            primary_surface: None,
            kind: ClientKind::Unknown,
            trust: ClientTrust::Unknown,
        }
    }

    pub fn with_primary_surface(mut self, surface_id: impl Into<SurfaceId>) -> Self {
        self.primary_surface = Some(surface_id.into());
        self
    }

    pub fn effective_trust(&self) -> ClientTrust {
        match (self.kind, self.trust) {
            (ClientKind::Browser, _) => ClientTrust::Hostile,
            (ClientKind::Unknown, _) => ClientTrust::Hostile,
            _ => self.trust,
        }
    }

    pub fn is_hostile_equivalent(&self) -> bool {
        matches!(self.effective_trust(), ClientTrust::Hostile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_client_defaults_to_hostile() {
        let profile = ClientProfile::browser("renderer-1", "org.browser.chrome");
        assert_eq!(profile.kind, ClientKind::Browser);
        assert_eq!(profile.effective_trust(), ClientTrust::Hostile);
    }

    #[test]
    fn unknown_client_defaults_to_hostile() {
        let profile = ClientProfile::unknown("mystery-1", "org.example.unknown");
        assert_eq!(profile.kind, ClientKind::Unknown);
        assert!(profile.is_hostile_equivalent());
    }
}
