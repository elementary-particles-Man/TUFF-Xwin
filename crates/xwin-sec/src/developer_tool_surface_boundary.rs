use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XwinDeveloperToolSurfaceAction {
    Allow,
    Observe,
    Constrain,
    Quarantine,
    FailClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XwinDeveloperToolSurfaceContext {
    pub surface_markers: Vec<String>,
    pub pal_markers: Vec<String>,
    pub kairo_verdict: Option<String>,
}

pub fn evaluate_xwin_developer_tool_surface_boundary(
    context: &XwinDeveloperToolSurfaceContext,
) -> XwinDeveloperToolSurfaceAction {
    let markers = &context.surface_markers;
    let pal = &context.pal_markers;

    // 1. Propagated Fail Closed
    if context.kairo_verdict.as_deref() == Some("fail_closed")
        || markers.iter().any(|m| m == "xwin_kairo_fail_closed_propagated")
    {
        return XwinDeveloperToolSurfaceAction::FailClosed;
    }

    // 2. High-Risk Surface Requests with Untrusted Developer Context
    let is_untrusted = markers.iter().any(|m| {
        m == "xwin_untrusted_developer_source_context"
            || m == "xwin_interview_task_context"
            || m == "xwin_code_review_request_context"
            || m == "xwin_agent_generated_workspace_context"
    }) || pal.iter().any(|m| {
        m == "pal_repo_origin_untrusted"
            || m == "pal_interview_task_repo_marker"
            || m == "pal_code_review_request_repo_marker"
    });

    let has_secret = markers.iter().any(|m| m == "xwin_secret_namespace_context")
        || pal.iter().any(|m| {
            m == "pal_secret_path_access"
                || m == "pal_env_secret_access"
                || m == "pal_ssh_key_access"
                || m == "pal_npm_token_access"
                || m == "pal_github_token_access"
                || m == "pal_cloud_credential_access"
                || m == "pal_wallet_key_access"
                || m == "pal_browser_cookie_access"
        });

    let has_persistence = markers.iter().any(|m| m == "xwin_persistence_write_context")
        || pal.iter().any(|m| {
            m == "pal_git_hook_write"
                || m == "pal_shell_profile_write"
                || m == "pal_systemd_user_service_write"
                || m == "pal_cron_write"
        });

    if is_untrusted {
        if markers.iter().any(|m| m == "xwin_external_protocol_surface_requested") {
            return XwinDeveloperToolSurfaceAction::FailClosed;
        }

        if has_secret
            && markers.iter().any(|m| {
                m == "xwin_file_picker_surface_requested"
                    || m == "xwin_clipboard_surface_requested"
                    || m == "xwin_terminal_launch_surface_requested"
                    || m == "xwin_ide_extension_install_surface_requested"
            })
        {
            return XwinDeveloperToolSurfaceAction::FailClosed;
        }

        if has_persistence
            && markers.iter().any(|m| {
                m == "xwin_terminal_launch_surface_requested"
                    || m == "xwin_ide_extension_install_surface_requested"
                    || m == "xwin_browser_extension_install_surface_requested"
            })
        {
            return XwinDeveloperToolSurfaceAction::FailClosed;
        }

        if markers.iter().any(|m| {
            m == "xwin_clipboard_surface_requested"
                || m == "xwin_file_picker_surface_requested"
                || m == "xwin_drag_drop_surface_requested"
                || m == "xwin_terminal_launch_surface_requested"
                || m == "xwin_browser_download_upload_surface_requested"
                || m == "xwin_ide_extension_install_surface_requested"
                || m == "xwin_browser_extension_install_surface_requested"
                || m == "xwin_screen_capture_surface_requested"
                || m == "xwin_window_capture_surface_requested"
        }) {
            return XwinDeveloperToolSurfaceAction::Quarantine;
        }
    }

    // 3. Propagated Quarantine
    if context.kairo_verdict.as_deref() == Some("quarantine")
        || markers.iter().any(|m| m == "xwin_kairo_quarantine_propagated")
    {
        return XwinDeveloperToolSurfaceAction::Quarantine;
    }

    // 4. Unknown risky surface in untrusted context
    if is_untrusted && markers.iter().any(|m| m.ends_with("_requested")) {
        return XwinDeveloperToolSurfaceAction::FailClosed;
    }

    // 5. Constrain/Observe/Allow
    if markers.iter().any(|m| m == "xwin_developer_tool_surface_present") {
        if is_untrusted {
            return XwinDeveloperToolSurfaceAction::Observe;
        }
        if markers.iter().any(|m| m.ends_with("_requested")) {
            return XwinDeveloperToolSurfaceAction::Constrain;
        }
        return XwinDeveloperToolSurfaceAction::Observe;
    }

    XwinDeveloperToolSurfaceAction::Allow
}
