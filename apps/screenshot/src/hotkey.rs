use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed(String),
}

pub trait HotkeyController {
    fn register_hotkey(&mut self, hotkey: &str) -> Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct FakeHotkeyController {
    pub registered_hotkeys: Vec<String>,
}

impl HotkeyController for FakeHotkeyController {
    fn register_hotkey(&mut self, hotkey: &str) -> Result<()> {
        self.registered_hotkeys.push(hotkey.to_owned());
        Ok(())
    }
}
