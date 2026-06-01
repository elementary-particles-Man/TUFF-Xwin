use crate::{Result, WireError};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolSpec {
    pub name: String,
    pub interfaces: HashMap<String, InterfaceSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceSpec {
    pub name: String,
    pub version: u32,
    pub requests: Vec<MessageSpec>,
    pub events: Vec<MessageSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageKindSpec {
    Request,
    Event,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSpec {
    pub name: String,
    pub kind: MessageKindSpec,
    pub opcode: u16,
    pub args: Vec<ArgSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgSpec {
    pub name: String,
    pub arg_type: String, // int, uint, fixed, string, object, new_id, array, fd
    pub interface: Option<String>,
    pub allow_null: bool,
}

impl ProtocolSpec {
    pub fn parse(xml: &str) -> Result<Self> {
        let mut reader = Reader::from_str(xml);
        reader.trim_text(true);

        let mut protocol_name = String::new();
        let mut interfaces = HashMap::new();
        let mut buf = Vec::new();

        let mut current_interface: Option<InterfaceSpec> = None;
        let mut current_message: Option<MessageSpec> = None;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(event) => match event {
                    Event::Start(ref e) | Event::Empty(ref e) => {
                        let is_empty = matches!(event, Event::Empty(_));
                        let tag_name = e.name();

                        match tag_name.as_ref() {
                            b"protocol" => {
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"name" {
                                        protocol_name =
                                            String::from_utf8_lossy(&attr.value).into_owned();
                                    }
                                }
                            }
                            b"interface" => {
                                let mut name = String::new();
                                let mut version = 1;
                                for attr in e.attributes().flatten() {
                                    match attr.key.as_ref() {
                                        b"name" => {
                                            name = String::from_utf8_lossy(&attr.value).into_owned()
                                        }
                                        b"version" => {
                                            version = String::from_utf8_lossy(&attr.value)
                                                .parse()
                                                .unwrap_or(1);
                                        }
                                        _ => {}
                                    }
                                }
                                current_interface = Some(InterfaceSpec {
                                    name,
                                    version,
                                    requests: Vec::new(),
                                    events: Vec::new(),
                                });
                            }
                            b"request" | b"event" => {
                                let kind = if tag_name.as_ref() == b"request" {
                                    MessageKindSpec::Request
                                } else {
                                    MessageKindSpec::Event
                                };
                                let mut name = String::new();
                                for attr in e.attributes().flatten() {
                                    if attr.key.as_ref() == b"name" {
                                        name = String::from_utf8_lossy(&attr.value).into_owned();
                                    }
                                }

                                let opcode = if let Some(ref iface) = current_interface {
                                    if kind == MessageKindSpec::Request {
                                        iface.requests.len() as u16
                                    } else {
                                        iface.events.len() as u16
                                    }
                                } else {
                                    0
                                };

                                let msg = MessageSpec {
                                    name,
                                    kind: kind.clone(),
                                    opcode,
                                    args: Vec::new(),
                                };
                                if is_empty {
                                    if let Some(ref mut iface) = current_interface {
                                        let kind = msg.kind.clone();
                                        if kind == MessageKindSpec::Request {
                                            iface.requests.push(msg);
                                        } else {
                                            iface.events.push(msg);
                                        }
                                    }
                                } else {
                                    current_message = Some(msg);
                                }
                            }
                            b"arg" => {
                                if let Some(ref mut msg) = current_message {
                                    let mut arg_name = String::new();
                                    let mut arg_type = String::new();
                                    let mut interface = None;
                                    let mut allow_null = false;

                                    for attr in e.attributes().flatten() {
                                        match attr.key.as_ref() {
                                            b"name" => {
                                                arg_name = String::from_utf8_lossy(&attr.value)
                                                    .into_owned()
                                            }
                                            b"type" => {
                                                arg_type = String::from_utf8_lossy(&attr.value)
                                                    .into_owned()
                                            }
                                            b"interface" => {
                                                interface = Some(
                                                    String::from_utf8_lossy(&attr.value)
                                                        .into_owned(),
                                                )
                                            }
                                            b"allow-null" => {
                                                allow_null =
                                                    String::from_utf8_lossy(&attr.value) == "true"
                                            }
                                            _ => {}
                                        }
                                    }
                                    msg.args.push(ArgSpec {
                                        name: arg_name,
                                        arg_type,
                                        interface,
                                        allow_null,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    Event::End(e) => match e.name().as_ref() {
                        b"interface" => {
                            if let Some(iface) = current_interface.take() {
                                interfaces.insert(iface.name.clone(), iface);
                            }
                        }
                        b"request" => {
                            if let Some(msg) = current_message.take() {
                                if let Some(ref mut iface) = current_interface {
                                    iface.requests.push(msg);
                                }
                            }
                        }
                        b"event" => {
                            if let Some(msg) = current_message.take() {
                                if let Some(ref mut iface) = current_interface {
                                    iface.events.push(msg);
                                }
                            }
                        }
                        _ => {}
                    },
                    Event::Eof => break,
                    _ => {}
                },
                Err(e) => {
                    return Err(WireError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))
                }
            }
            buf.clear();
        }

        Ok(ProtocolSpec { name: protocol_name, interfaces })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_xdg_shell_xml() {
        let xml = include_str!("../../../protocols/stable/xdg-shell/xdg-shell.xml");
        let spec = ProtocolSpec::parse(xml).expect("parse failed");
        assert_eq!(spec.name, "xdg_shell");

        let wm_base = spec.interfaces.get("xdg_wm_base").expect("xdg_wm_base exists");
        assert_eq!(wm_base.version, 6);
        assert!(wm_base.requests.iter().any(|r| r.name == "get_xdg_surface"));
        assert!(wm_base.events.iter().any(|e| e.name == "ping"));
    }
}
