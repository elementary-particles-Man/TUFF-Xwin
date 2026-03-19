#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceRole {
    Displayd,
    Waylandd,
    Compd,
    Lockd,
    Sessiond,
    Watchdog,
}

impl ServiceRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Displayd => "displayd",
            Self::Waylandd => "waylandd",
            Self::Compd => "compd",
            Self::Lockd => "lockd",
            Self::Sessiond => "sessiond",
            Self::Watchdog => "watchdog",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceBanner {
    pub role: ServiceRole,
    pub responsibility: &'static str,
}

impl ServiceBanner {
    pub const fn new(role: ServiceRole, responsibility: &'static str) -> Self {
        Self { role, responsibility }
    }

    pub fn render(self) -> String {
        format!("waybroker service={} responsibility={}", self.role.as_str(), self.responsibility)
    }
}

#[cfg(test)]
mod tests {
    use super::{ServiceBanner, ServiceRole};

    #[test]
    fn renders_service_banner() {
        let banner = ServiceBanner::new(ServiceRole::Compd, "scene and focus policy");
        assert_eq!(
            banner.render(),
            "waybroker service=compd responsibility=scene and focus policy"
        );
    }
}
