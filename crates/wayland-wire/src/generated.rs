use crate::protocol::ProtocolSpec;
use std::sync::OnceLock;

const CORE_XML: &str = include_str!("../../../protocols/core/wayland-core.xml");
const XDG_SHELL_XML: &str = include_str!("../../../protocols/stable/xdg-shell/xdg-shell.xml");
const TEXT_INPUT_XML: &str =
    include_str!("../../../protocols/unstable/text-input/text-input-unstable-v3.xml");
const INPUT_METHOD_XML: &str =
    include_str!("../../../protocols/unstable/input-method/input-method-unstable-v2.xml");
const VIEWPORTER_XML: &str = include_str!("../../../protocols/stable/viewporter/viewporter.xml");
const PRESENTATION_XML: &str =
    include_str!("../../../protocols/stable/presentation-time/presentation-time.xml");
const FRACTIONAL_SCALE_XML: &str =
    include_str!("../../../protocols/staging/fractional-scale/fractional-scale-v1.xml");
const XDG_DECORATION_XML: &str =
    include_str!("../../../protocols/unstable/xdg-decoration/xdg-decoration-unstable-v1.xml");

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
        let viewporter =
            ProtocolSpec::parse(VIEWPORTER_XML).expect("failed to parse viewporter protocol XML");
        let presentation = ProtocolSpec::parse(PRESENTATION_XML)
            .expect("failed to parse presentation protocol XML");
        let fractional_scale = ProtocolSpec::parse(FRACTIONAL_SCALE_XML)
            .expect("failed to parse fractional-scale protocol XML");
        let xdg_decoration = ProtocolSpec::parse(XDG_DECORATION_XML)
            .expect("failed to parse xdg-decoration protocol XML");

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
        for (name, iface) in viewporter.interfaces {
            core.interfaces.insert(name, iface);
        }
        for (name, iface) in presentation.interfaces {
            core.interfaces.insert(name, iface);
        }
        for (name, iface) in fractional_scale.interfaces {
            core.interfaces.insert(name, iface);
        }
        for (name, iface) in xdg_decoration.interfaces {
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

        assert!(spec.interfaces.contains_key("wl_display"));
        assert!(spec.interfaces.contains_key("zwp_text_input_v3"));
        assert!(spec.interfaces.contains_key("wp_viewporter"));
        assert!(spec.interfaces.contains_key("wp_presentation"));
        assert!(spec.interfaces.contains_key("wp_fractional_scale_v1"));
        assert!(spec.interfaces.contains_key("zxdg_toplevel_decoration_v1"));
    }
}
