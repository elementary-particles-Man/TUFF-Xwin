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

    #[test]
    fn test_parse_layer_shell_xml() {
        let xml = include_str!(
            "../../../protocols/unstable/wlr-layer-shell/wlr-layer-shell-unstable-v1.xml"
        );
        let spec = ProtocolSpec::parse(xml).expect("parse failed");
        let iface = spec.interfaces.get("zwlr_layer_shell_v1").expect("layer shell exists");
        assert_eq!(iface.version, 4);
        assert!(iface.requests.iter().any(|r| r.name == "get_layer_surface"));
        let layer_surface =
            spec.interfaces.get("zwlr_layer_surface_v1").expect("layer surface exists");
        assert!(layer_surface.events.iter().any(|e| e.name == "configure"));
        assert!(layer_surface.requests.iter().any(|r| r.name == "ack_configure"));
    }

    #[test]
    fn test_parse_idle_inhibit_xml() {
        let xml =
            include_str!("../../../protocols/unstable/idle-inhibit/idle-inhibit-unstable-v1.xml");
        let spec = ProtocolSpec::parse(xml).expect("parse failed");
        let iface = spec
            .interfaces
            .get("zwp_idle_inhibit_manager_v1")
            .expect("idle inhibit manager exists");
        assert!(iface.requests.iter().any(|r| r.name == "create_inhibitor"));
        assert!(spec.interfaces.contains_key("zwp_idle_inhibitor_v1"));
    }

    #[test]
    fn test_parse_relative_pointer_xml() {
        let xml = include_str!(
            "../../../protocols/unstable/relative-pointer/relative-pointer-unstable-v1.xml"
        );
        let spec = ProtocolSpec::parse(xml).expect("parse failed");
        let iface = spec
            .interfaces
            .get("zwp_relative_pointer_manager_v1")
            .expect("relative pointer manager exists");
        assert!(iface.requests.iter().any(|r| r.name == "get_relative_pointer"));
        let rel = spec.interfaces.get("zwp_relative_pointer_v1").expect("relative pointer exists");
        assert!(rel.events.iter().any(|e| e.name == "relative_motion"));
    }

    #[test]
    fn test_parse_pointer_constraints_xml() {
        let xml = include_str!(
            "../../../protocols/unstable/pointer-constraints/pointer-constraints-unstable-v1.xml"
        );
        let spec = ProtocolSpec::parse(xml).expect("parse failed");
        let iface =
            spec.interfaces.get("zwp_pointer_constraints_v1").expect("pointer constraints exists");
        assert!(iface.requests.iter().any(|r| r.name == "lock_pointer"));
        assert!(spec.interfaces.contains_key("zwp_locked_pointer_v1"));
        assert!(spec.interfaces.contains_key("zwp_confined_pointer_v1"));
    }
}
