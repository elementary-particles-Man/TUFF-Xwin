use crate::{Result, WaylandObjectId};

pub trait ImeBackend: Send + Sync {
    fn handle_surrounding_text(&mut self, text: &str, cursor: i32, anchor: i32);
    fn handle_commit(&mut self) -> Vec<ImeRequest>;
}

#[derive(Debug, Clone)]
pub enum ImeRequest {
    PreeditString { text: String, cursor_begin: i32, cursor_end: i32 },
    CommitString(String),
    DeleteSurroundingText { before: u32, after: u32 },
}

pub struct FakeImeBackend {
    pub last_text: String,
    pub requests: Vec<ImeRequest>,
}

impl FakeImeBackend {
    pub fn new() -> Self {
        Self { last_text: String::new(), requests: Vec::new() }
    }
}

impl ImeBackend for FakeImeBackend {
    fn handle_surrounding_text(&mut self, text: &str, _cursor: i32, _anchor: i32) {
        self.last_text = text.to_string();
    }

    fn handle_commit(&mut self) -> Vec<ImeRequest> {
        let mut reqs = Vec::new();
        // Simple logic for testing:
        // If text is "hello", suggest "こんにちは" as preedit
        if self.last_text == "hello" {
            reqs.push(ImeRequest::PreeditString {
                text: "こんにちは".into(),
                cursor_begin: 0,
                cursor_end: 15,
            });
        } else if self.last_text == "commit" {
            reqs.push(ImeRequest::CommitString("確定".into()));
        }
        reqs
    }
}
