#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageStatus {
    HarnessOnly,
    NotRealSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverageRow {
    pub area: &'static str,
    pub protocols: &'static str,
    pub status: CoverageStatus,
    pub note: &'static str,
}

static COVERAGE_ROWS: &[CoverageRow] = &[
    CoverageRow {
        area: "Core / SHM / Surface",
        protocols: "wl_display, wl_registry, wl_compositor, wl_surface, wl_shm",
        status: CoverageStatus::HarnessOnly,
        note: "tempdir isolated socket probe reaches the core handshake and buffer path.",
    },
    CoverageRow {
        area: "XDG Shell",
        protocols: "xdg_wm_base, xdg_surface, xdg_toplevel",
        status: CoverageStatus::HarnessOnly,
        note: "real-client isolated probe verifies xdg-shell lifecycle without wl_display_connect(NULL).",
    },
    CoverageRow {
        area: "Seat / Data Device / Viewporter / Presentation / Layer Shell",
        protocols: "wl_seat, wl_data_device_manager, wp_viewporter, wp_presentation, zwlr_layer_shell_v1",
        status: CoverageStatus::NotRealSession,
        note: "matrix only; no real Wayland session or input device is touched.",
    },
];

pub fn coverage_matrix() -> &'static [CoverageRow] {
    COVERAGE_ROWS
}
