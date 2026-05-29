use crate::protocol::ProtocolSpec;
use std::sync::OnceLock;

const CORE_XML: &str = include_str!("../../../protocols/core/wayland-core.xml");
const XDG_SHELL_XML: &str = include_str!("../../../protocols/stable/xdg-shell/xdg-shell.xml");

static CORE_SPEC: OnceLock<ProtocolSpec> = OnceLock::new();

pub fn core_protocol_spec() -> &'static ProtocolSpec {
    CORE_SPEC.get_or_init(|| {
        let mut core = ProtocolSpec::parse(CORE_XML).expect("failed to parse core wayland protocol XML");
        let xdg = ProtocolSpec::parse(XDG_SHELL_XML).expect("failed to parse xdg-shell protocol XML");
        
        // Merge xdg interfaces into core spec for simple lookup
        for (name, iface) in xdg.interfaces {
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
        assert_eq!(get_registry.args.len(), 1);
        assert_eq!(get_registry.args[0].arg_type, "new_id");
        assert_eq!(get_registry.args[0].interface.as_deref(), Some("wl_registry"));
    }
}
