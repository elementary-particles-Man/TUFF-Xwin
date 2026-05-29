use crate::protocol::MessageSpec;
use crate::WireArg;

pub fn generate_signature(msg: &MessageSpec) -> String {
    let mut sig = String::new();
    for arg in &msg.args {
        if arg.allow_null {
            sig.push('?');
        }
        let c = match arg.arg_type.as_str() {
            "int" => 'i',
            "uint" => 'u',
            "fixed" => 'f',
            "string" => 's',
            "object" => 'o',
            "new_id" => 'n',
            "array" => 'a',
            "fd" => 'h',
            _ => '?',
        };
        sig.push(c);
    }
    sig
}

pub fn validate_args(msg: &MessageSpec, args: &[WireArg]) -> bool {
    if msg.args.len() != args.len() {
        return false;
    }

    for (spec, arg) in msg.args.iter().zip(args.iter()) {
        let match_type = match (spec.arg_type.as_str(), arg) {
            ("int", WireArg::Int(_)) => true,
            ("uint", WireArg::Uint(_)) => true,
            ("fixed", WireArg::Fixed(_)) => true,
            ("string", WireArg::String(_)) => true,
            ("object", WireArg::Object(_)) => true,
            ("new_id", WireArg::NewId(_)) => true,
            ("array", WireArg::Array(_)) => true,
            ("fd", WireArg::Fd(_)) | ("fd", WireArg::AncillaryFd) => true,
            _ => false,
        };
        if !match_type {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ArgSpec, MessageKindSpec};

    #[test]
    fn test_signature_generation() {
        let msg = MessageSpec {
            name: "test".into(),
            kind: MessageKindSpec::Request,
            opcode: 0,
            args: vec![
                ArgSpec {
                    name: "a".into(),
                    arg_type: "int".into(),
                    interface: None,
                    allow_null: false,
                },
                ArgSpec {
                    name: "b".into(),
                    arg_type: "string".into(),
                    interface: None,
                    allow_null: true,
                },
                ArgSpec {
                    name: "c".into(),
                    arg_type: "new_id".into(),
                    interface: Some("wl_surface".into()),
                    allow_null: false,
                },
            ],
        };
        assert_eq!(generate_signature(&msg), "i?sn");
    }
}
