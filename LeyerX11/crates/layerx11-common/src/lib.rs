use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use waybroker_common::{FocusTarget, SurfacePlacement, SurfaceSnapshot, WaylandSelectionState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct X11RootlessScene {
    pub output: X11OutputTarget,
    pub focus_window: Option<String>,
    #[serde(default)]
    pub selection: X11SelectionState,
    pub windows: Vec<X11Window>,
}

impl X11RootlessScene {
    pub fn validate(&self) -> Result<(), String> {
        if self.output.name.is_empty() {
            return Err("output name must not be empty".into());
        }
        if !(1..=16_384).contains(&self.output.width) || !(1..=16_384).contains(&self.output.height)
        {
            return Err("output dimensions must be between 1 and 16384".into());
        }
        let pixels = u64::from(self.output.width) * u64::from(self.output.height);
        if pixels > 67_108_864 {
            return Err("output pixel budget exceeded".into());
        }
        if self.windows.len() > 4096 {
            return Err("window budget exceeded".into());
        }

        let mut ids = BTreeSet::new();
        for window in &self.windows {
            if window.id.is_empty() || !ids.insert(window.id.as_str()) {
                return Err("window ids must be non-empty and unique".into());
            }
            if window.mapped {
                if window.width == 0 || window.height == 0 {
                    return Err(format!(
                        "mapped window {} must have non-zero dimensions",
                        window.id
                    ));
                }
                let width = i32::try_from(window.width)
                    .map_err(|_| format!("window {} width overflows i32", window.id))?;
                let height = i32::try_from(window.height)
                    .map_err(|_| format!("window {} height overflows i32", window.id))?;
                window
                    .x
                    .checked_add(width)
                    .ok_or_else(|| format!("window {} x coordinate overflows i32", window.id))?;
                window
                    .y
                    .checked_add(height)
                    .ok_or_else(|| format!("window {} y coordinate overflows i32", window.id))?;
            }
        }

        for (label, owner, payload, serial) in [
            (
                "clipboard",
                self.selection.clipboard_owner.as_deref(),
                self.selection.clipboard_payload_id.as_ref(),
                self.selection.clipboard_source_serial,
            ),
            (
                "primary selection",
                self.selection.primary_selection_owner.as_deref(),
                self.selection.primary_selection_payload_id.as_ref(),
                self.selection.primary_selection_source_serial,
            ),
        ] {
            match owner {
                Some(owner) => {
                    let mapped =
                        self.windows.iter().any(|window| window.mapped && window.id == owner);
                    if !mapped {
                        return Err(format!("{label} owner {owner} is not a mapped window"));
                    }
                }
                None if payload.is_some() || serial.is_some() => {
                    return Err(format!("{label} metadata requires an owner"));
                }
                None => {}
            }
        }
        Ok(())
    }

    pub fn mapped_windows(&self) -> impl Iterator<Item = &X11Window> {
        self.windows.iter().filter(|window| window.mapped)
    }

    pub fn to_surface_snapshots(&self) -> Vec<SurfaceSnapshot> {
        self.mapped_windows()
            .map(|window| SurfaceSnapshot {
                id: window.id.clone(),
                app_id: window.app_id.clone(),
                placement: SurfacePlacement {
                    x: window.x,
                    y: window.y,
                    width: window.width,
                    height: window.height,
                    z: window.z,
                    visible: window.mapped,
                },
                ..Default::default()
            })
            .collect()
    }

    pub fn focus_target(&self) -> FocusTarget {
        match self.focus_window.as_deref() {
            Some(focus_id) if self.mapped_windows().any(|window| window.id == focus_id) => {
                FocusTarget::Surface { id: focus_id.into() }
            }
            _ => FocusTarget::None,
        }
    }

    pub fn selection_state(&self) -> WaylandSelectionState {
        WaylandSelectionState {
            clipboard_owner: self.selection.clipboard_owner.clone(),
            clipboard_payload_id: self.selection.clipboard_payload_id.clone(),
            clipboard_source_serial: self.selection.clipboard_source_serial,
            clipboard_offer: None,
            primary_selection_owner: self.selection.primary_selection_owner.clone(),
            primary_selection_payload_id: self.selection.primary_selection_payload_id.clone(),
            primary_selection_source_serial: self.selection.primary_selection_source_serial,
            primary_offer: None,
        }
    }

    pub fn mapped_window_count(&self) -> usize {
        self.mapped_windows().count()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct X11SelectionState {
    pub clipboard_owner: Option<String>,
    pub clipboard_payload_id: Option<String>,
    pub clipboard_source_serial: Option<u64>,
    pub primary_selection_owner: Option<String>,
    pub primary_selection_payload_id: Option<String>,
    pub primary_selection_source_serial: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct X11OutputTarget {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct X11Window {
    pub id: String,
    pub app_id: String,
    pub title: String,
    pub kind: X11WindowKind,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub z: i32,
    pub mapped: bool,
    pub override_redirect: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum X11WindowKind {
    Normal,
    Dialog,
    Utility,
    Dock,
    Menu,
    Tooltip,
}

pub fn sample_rootless_scene() -> X11RootlessScene {
    X11RootlessScene {
        output: X11OutputTarget { name: "eDP-1".into(), width: 1920, height: 1080 },
        focus_window: Some("xterm-1".into()),
        selection: X11SelectionState {
            clipboard_owner: Some("xterm-1".into()),
            clipboard_payload_id: Some("x11-clipboard-v1".into()),
            clipboard_source_serial: Some(101),
            primary_selection_owner: Some("xterm-1".into()),
            primary_selection_payload_id: Some("x11-primary-v1".into()),
            primary_selection_source_serial: Some(102),
        },
        windows: vec![
            X11Window {
                id: "xterm-1".into(),
                app_id: "org.x.term".into(),
                title: "xterm".into(),
                kind: X11WindowKind::Normal,
                x: 96,
                y: 72,
                width: 1120,
                height: 720,
                z: 10,
                mapped: true,
                override_redirect: false,
            },
            X11Window {
                id: "xclock-1".into(),
                app_id: "org.x.clock".into(),
                title: "xclock".into(),
                kind: X11WindowKind::Utility,
                x: 1320,
                y: 88,
                width: 240,
                height: 240,
                z: 20,
                mapped: true,
                override_redirect: false,
            },
            X11Window {
                id: "dock-1".into(),
                app_id: "org.x.panel".into(),
                title: "legacy dock".into(),
                kind: X11WindowKind::Dock,
                x: 0,
                y: 0,
                width: 1920,
                height: 32,
                z: 100,
                mapped: true,
                override_redirect: true,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::{X11RootlessScene, X11Window, X11WindowKind, sample_rootless_scene};
    use waybroker_common::FocusTarget;

    #[test]
    fn maps_only_visible_windows_into_surfaces() {
        let mut scene = sample_rootless_scene();
        scene.windows.push(X11Window {
            id: "hidden-1".into(),
            app_id: "org.x.hidden".into(),
            title: "hidden".into(),
            kind: X11WindowKind::Dialog,
            x: 400,
            y: 240,
            width: 320,
            height: 240,
            z: 30,
            mapped: false,
            override_redirect: false,
        });

        let surfaces = scene.to_surface_snapshots();

        assert_eq!(surfaces.len(), 3);
        assert!(surfaces.iter().all(|surface| surface.placement.visible));
        assert!(surfaces.iter().all(|surface| surface.id != "hidden-1"));
    }

    #[test]
    fn derives_focus_from_mapped_window() {
        let scene = sample_rootless_scene();

        assert_eq!(scene.focus_target(), FocusTarget::Surface { id: "xterm-1".into() });
    }

    #[test]
    fn falls_back_to_none_for_missing_focus() {
        let scene = X11RootlessScene {
            output: super::X11OutputTarget { name: "eDP-1".into(), width: 1920, height: 1080 },
            focus_window: Some("ghost".into()),
            selection: super::X11SelectionState::default(),
            windows: vec![],
        };
        assert_eq!(scene.focus_target(), FocusTarget::None);
    }

    #[test]
    fn test_phase4_x11_clipboard_and_selection_mapping() {
        let scene = sample_rootless_scene();
        let selection = scene.selection_state();

        assert_eq!(selection.clipboard_owner.as_deref(), Some("xterm-1"));
        assert_eq!(selection.clipboard_payload_id.as_deref(), Some("x11-clipboard-v1"));
        assert_eq!(selection.clipboard_source_serial, Some(101));
        assert_eq!(selection.primary_selection_owner.as_deref(), Some("xterm-1"));
        assert_eq!(selection.primary_selection_payload_id.as_deref(), Some("x11-primary-v1"));
        assert_eq!(selection.primary_selection_source_serial, Some(102));
    }

    #[test]
    fn test_phase4_x11_window_properties_and_ewmh() {
        let scene = sample_rootless_scene();
        let snapshots = scene.to_surface_snapshots();

        let dock_snap = snapshots.iter().find(|s| s.id == "dock-1").expect("dock window exists");
        assert!(dock_snap.placement.visible);
        assert_eq!(dock_snap.placement.width, 1920);
        assert_eq!(dock_snap.placement.height, 32);
    }

    #[test]
    fn validates_sample_and_rejects_invalid_rootless_scenes() {
        assert!(sample_rootless_scene().validate().is_ok());
        let mut scene = sample_rootless_scene();
        scene.output.name.clear();
        assert!(scene.validate().is_err());
        let mut scene = sample_rootless_scene();
        scene.output.width = 16_385;
        assert!(scene.validate().is_err());
        let mut scene = sample_rootless_scene();
        scene.windows[1].id = scene.windows[0].id.clone();
        assert!(scene.validate().is_err());
        let mut scene = sample_rootless_scene();
        scene.windows[0].width = 0;
        assert!(scene.validate().is_err());
        let mut scene = sample_rootless_scene();
        scene.windows[0].x = i32::MAX;
        assert!(scene.validate().is_err());
    }

    #[test]
    fn rejects_dangling_and_orphaned_selection_metadata() {
        let mut scene = sample_rootless_scene();
        scene.selection.clipboard_owner = Some("ghost".into());
        assert!(scene.validate().is_err());
        let mut scene = sample_rootless_scene();
        scene.selection.clipboard_owner = None;
        assert!(scene.validate().is_err());
        let mut scene = sample_rootless_scene();
        scene.selection.primary_selection_owner = None;
        scene.selection.primary_selection_payload_id = None;
        scene.selection.primary_selection_source_serial = None;
        assert!(scene.validate().is_ok());
    }
}
