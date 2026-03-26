use serde::{Deserialize, Serialize};
use waybroker_common::{FocusTarget, SurfacePlacement, SurfaceSnapshot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct X11RootlessScene {
    pub output: X11OutputTarget,
    pub focus_window: Option<String>,
    pub windows: Vec<X11Window>,
}

impl X11RootlessScene {
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

    pub fn mapped_window_count(&self) -> usize {
        self.mapped_windows().count()
    }
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
            windows: vec![],
        };

        assert_eq!(scene.focus_target(), FocusTarget::None);
    }
}
