use crate::protocol::ProtocolSpec;
use std::sync::OnceLock;

const CORE_XML: &str = include_str!("../../../protocols/core/wayland-core.xml");
const XDG_SHELL_XML: &str = include_str!("../../../protocols/stable/xdg-shell/xdg-shell.xml");
const TEXT_INPUT_XML: &str =
    include_str!("../../../protocols/unstable/text-input/text-input-unstable-v3.xml");
const INPUT_METHOD_XML: &str =
    include_str!("../../../protocols/unstable/input-method/input-method-unstable-v2.xml");

static CORE_SPEC: OnceLock<ProtocolSpec> = OnceLock::new();

pub fn core_protocol_spec() -> &'static ProtocolSpec {
    CORE_SPEC.get_or_init(|| {
        let mut core =
            ProtocolSpec::parse(CORE_XML).expect("failed to parse core wayland protocol XML");
        let xdg =
            ProtocolSpec::parse(XDG_SHELL_XML).expect("failed to parse xdg-shell protocol XML");
        let text_input = ProtocolSpec::parse(TEXT_INPUT_XML)
            .expect("failed to parse text-input-v3 protocol XML");
        let input_method = ProtocolSpec::parse(INPUT_METHOD_XML)
            .expect("failed to parse input-method-v2 protocol XML");

        // Merge interfaces into core spec for simple lookup
        for (name, iface) in xdg.interfaces {
            core.interfaces.insert(name, iface);
        }
        for (name, iface) in text_input.interfaces {
            core.interfaces.insert(name, iface);
        }
        for (name, iface) in input_method.interfaces {
            core.interfaces.insert(name, iface);
        }
        core
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_protocol_lookup() {
        let spec = core_protocol_spec();
        assert_eq!(spec.name, "wayland");

        let wl_display = spec.interfaces.get("wl_display").expect("wl_display exists");
        let get_registry = wl_display
            .requests
            .iter()
            .find(|r| r.name == "get_registry")
            .expect("get_registry exists");
        assert_eq!(get_registry.opcode, 1);

        // Verify P10 interfaces are loaded
        assert!(spec.interfaces.contains_key("zwp_text_input_v3"));
        assert!(spec.interfaces.contains_key("zwp_input_method_v2"));
    }
}
