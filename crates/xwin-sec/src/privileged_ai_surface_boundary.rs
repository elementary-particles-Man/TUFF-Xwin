use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XwinAction {
    Allow,
    Observe,
    Constrain,
    Quarantine,
    FailClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XwinPrivilegedAiSurfaceContext {
    pub surface_markers: Vec<String>,
    pub pal_markers: Vec<String>,
    pub kairo_verdict: Option<String>,
}

pub fn evaluate_xwin_privileged_ai_surface_boundary(
    context: &XwinPrivilegedAiSurfaceContext,
) -> XwinAction {
    let markers = &context.surface_markers;
    let pal = &context.pal_markers;

    // 1. Propagated Fail Closed
    if context.kairo_verdict.as_deref() == Some("fail_closed")
        || markers.iter().any(|m| m == "xwin_kairo_fail_closed_propagated")
    {
        return XwinAction::FailClosed;
    }

    // 2. High-Risk Surface Requests with Sensitive Context
    let has_sensitive = pal.iter().any(|m| {
        m == "pal_sensitive_label_present"
            || m == "pal_dlp_policy_present"
            || m == "pal_mail_context"
            || m == "pal_file_context"
            || m == "pal_chat_context"
            || m == "pal_calendar_context"
            || m == "xwin_sensitive_context_marker"
    });

    let has_external_input = pal.iter().any(|m| {
        m == "pal_external_link_context"
            || m == "pal_external_image_context"
            || m == "pal_external_document_context"
            || m == "pal_prompt_injection_like_content"
            || m == "xwin_external_input_marker"
    });

    if has_sensitive || has_external_input {
        if markers.iter().any(|m| m == "xwin_native_messaging_surface_requested") {
            return XwinAction::FailClosed;
        }
        if markers.iter().any(|m| m == "xwin_external_protocol_surface_requested") {
            return XwinAction::FailClosed;
        }
        if markers.iter().any(|m| m == "xwin_browser_profile_surface_requested") {
            return XwinAction::FailClosed;
        }

        if markers.iter().any(|m| {
            m == "xwin_clipboard_surface_requested"
                || m == "xwin_file_picker_surface_requested"
                || m == "xwin_drag_drop_surface_requested"
                || m == "xwin_screen_capture_surface_requested"
                || m == "xwin_window_capture_surface_requested"
                || m == "xwin_download_upload_dialog_surface_requested"
        }) {
            return XwinAction::Quarantine;
        }
    }

    // 3. Propagated Quarantine
    if context.kairo_verdict.as_deref() == Some("quarantine")
        || markers.iter().any(|m| m == "xwin_kairo_quarantine_propagated")
    {
        return XwinAction::Quarantine;
    }

    // 4. Unknown risky surface
    if markers.iter().any(|m| m == "xwin_privileged_ai_surface_present")
        && markers.iter().any(|m| m.ends_with("_requested"))
        && !markers.iter().any(|m| {
            m == "xwin_clipboard_surface_requested"
                || m == "xwin_file_picker_surface_requested"
                || m == "xwin_drag_drop_surface_requested"
                || m == "xwin_screen_capture_surface_requested"
                || m == "xwin_window_capture_surface_requested"
                || m == "xwin_download_upload_dialog_surface_requested"
                || m == "xwin_native_messaging_surface_requested"
                || m == "xwin_external_protocol_surface_requested"
                || m == "xwin_browser_profile_surface_requested"
        })
    {
        return XwinAction::FailClosed;
    }

    // 5. Constrain/Observe/Allow
    if markers.iter().any(|m| m == "xwin_privileged_ai_surface_present") {
        if markers.iter().any(|m| m.ends_with("_requested")) {
            return XwinAction::Constrain;
        }
        return XwinAction::Observe;
    }

    XwinAction::Allow
}
