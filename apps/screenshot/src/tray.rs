use anyhow::Result;

pub trait TrayController {
    fn request_popup(&mut self) -> Result<()>;
    fn dismiss_popup(&mut self) -> Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct FakeTrayController {
    pub popup_requests: usize,
    pub dismiss_requests: usize,
}

impl TrayController for FakeTrayController {
    fn request_popup(&mut self) -> Result<()> {
        self.popup_requests += 1;
        Ok(())
    }

    fn dismiss_popup(&mut self) -> Result<()> {
        self.dismiss_requests += 1;
        Ok(())
    }
}
