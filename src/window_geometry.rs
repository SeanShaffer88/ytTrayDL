//! Normal window bounds persisted independently from user preferences.
//! Transient prompts are deliberately absent; native access stays in `platform`.
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
}

impl WindowGeometry {
    /// Validate untrusted persisted bounds, before any DPI scaling.
    pub fn valid(self) -> bool {
        self.x.abs_diff(0) <= 1_000_000
            && self.y.abs_diff(0) <= 1_000_000
            && (64..=16_384).contains(&self.width)
            && (64..=16_384).contains(&self.height)
            && (48..=768).contains(&self.dpi)
    }

    /// Preserve the physical origin; scale size only, then clamp against the
    /// current monitor topology. Old monitor origins need not still exist.
    pub fn at_dpi(self, dpi: u32) -> Self {
        if !self.valid() || !(48..=768).contains(&dpi) {
            return self;
        }
        let scale = |value: i32| {
            ((i64::from(value) * i64::from(dpi) + i64::from(self.dpi / 2)) / i64::from(self.dpi))
                as i32
        };
        Self {
            width: scale(self.width),
            height: scale(self.height),
            dpi,
            ..self
        }
    }

    /// Translate actual outer bounds into the native monitor work area.
    /// Oversized windows retain their size and align to the work-area origin.
    pub fn clamped(self, work: [i32; 4], inset: i32) -> Self {
        // Native/scaled sizes may legitimately cross the persisted-input
        // limits. Placement only requires positive sizes; i64 arithmetic keeps
        // every i32 input safe without skipping the clamp after DPI conversion.
        if self.width <= 0 || self.height <= 0 || work[2] <= work[0] || work[3] <= work[1] {
            return self;
        }
        let axis = |position: i32, size: i32, start: i32, end: i32| {
            let lower = i64::from(start) + i64::from(inset.max(0));
            let upper = i64::from(end) - i64::from(inset.max(0)) - i64::from(size);
            if upper < lower {
                start
            } else {
                i64::from(position).clamp(lower, upper) as i32
            }
        };
        Self {
            x: axis(self.x, self.width, work[0], work[2]),
            y: axis(self.y, self.height, work[1], work[3]),
            ..self
        }
    }
}

/// Transient windows keep current intent anchors rather than persisted bounds.
#[derive(Debug, Clone, Copy)]
pub enum TransientWindow {
    DownloadPrompt,
    HideIconConfirmation,
    CloseConfirmation,
}
impl TransientWindow {
    pub fn title(self) -> &'static str {
        match self {
            Self::DownloadPrompt => "ytTrayDL — download?",
            Self::HideIconConfirmation => "Hide the tray icon?",
            Self::CloseConfirmation => "Close ytTrayDL?",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum WindowKind {
    Main,
    Settings,
    Downloads,
    About,
}

impl WindowKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Main => "ytTrayDL",
            Self::Settings => "ytTrayDL — Settings",
            Self::Downloads => "ytTrayDL — Downloads",
            Self::About => "About ytTrayDL",
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::Main => 0,
            Self::Settings => 1,
            Self::Downloads => 2,
            Self::About => 3,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowGeometries {
    pub schema_version: u32,
    pub main: Option<WindowGeometry>,
    pub settings: Option<WindowGeometry>,
    pub downloads: Option<WindowGeometry>,
    pub about: Option<WindowGeometry>,
}
impl Default for WindowGeometries {
    fn default() -> Self {
        Self {
            schema_version: 1,
            main: None,
            settings: None,
            downloads: None,
            about: None,
        }
    }
}

impl WindowGeometries {
    pub fn get(&self, kind: WindowKind) -> Option<WindowGeometry> {
        match kind {
            WindowKind::Main => self.main,
            WindowKind::Settings => self.settings,
            WindowKind::Downloads => self.downloads,
            WindowKind::About => self.about,
        }
        .filter(|value| value.valid())
    }
    pub fn set(&mut self, kind: WindowKind, geometry: WindowGeometry) -> bool {
        if !geometry.valid() {
            return false;
        }
        let slot = match kind {
            WindowKind::Main => &mut self.main,
            WindowKind::Settings => &mut self.settings,
            WindowKind::Downloads => &mut self.downloads,
            WindowKind::About => &mut self.about,
        };
        if *slot == Some(geometry) {
            return false;
        }
        *slot = Some(geometry);
        true
    }
    pub fn load() -> Self {
        let path = crate::settings::config_dir().join("window-state.json");
        Self::read_from(&path).unwrap_or_else(|| {
            if path.exists() {
                crate::diag_log("window state invalid or unsupported; using default geometry");
            }
            Self::default()
        })
    }
    #[cfg(test)]
    fn load_from(path: &Path) -> Self {
        Self::read_from(path).unwrap_or_default()
    }
    fn read_from(path: &Path) -> Option<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(16_385)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > 16_384 {
            return None;
        }
        serde_json::from_slice::<Self>(&bytes)
            .ok()
            .filter(|state| state.schema_version == 1)
    }
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&crate::settings::config_dir().join("window-state.json"))
    }
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        let file = std::fs::File::create(&temporary)?;
        serde_json::to_writer_pretty(&file, self)?;
        file.sync_all()?;
        std::fs::rename(temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn popup() -> WindowGeometry {
        WindowGeometry {
            x: 1478,
            y: 516,
            width: 446,
            height: 591,
            dpi: 96,
        }
    }
    #[test]
    fn actual_popup_overflow_translates_without_resizing() {
        assert_eq!(
            popup().clamped([0, 0, 1920, 1040], 12),
            WindowGeometry {
                x: 1462,
                y: 437,
                ..popup()
            }
        );
    }
    #[test]
    fn negative_monitor_and_oversized_bounds_remain_reachable() {
        let bounds = WindowGeometry {
            x: -2500,
            y: -500,
            ..popup()
        };
        assert_eq!(
            bounds.clamped([-1920, 0, 0, 1040], 12),
            WindowGeometry {
                x: -1908,
                y: 12,
                ..bounds
            }
        );
        assert_eq!(
            bounds.clamped([-1920, 0, -1800, 100], 12),
            WindowGeometry {
                x: -1920,
                y: 0,
                ..bounds
            }
        );
    }
    #[test]
    fn dpi_scaling_preserves_origin_and_invalid_state_is_ignored() {
        assert_eq!(
            popup().at_dpi(144),
            WindowGeometry {
                width: 669,
                height: 887,
                dpi: 144,
                ..popup()
            }
        );
        let state: WindowGeometries =
            serde_json::from_str(r#"{"main":{"x":0,"y":0,"width":-1,"height":500,"dpi":0}}"#)
                .unwrap();
        assert!(state.get(WindowKind::Main).is_none());
        assert!(serde_json::from_str::<WindowGeometries>("{}")
            .unwrap()
            .main
            .is_none());
        assert!(serde_json::from_str::<WindowGeometry>(
            r#"{"x":0,"y":0,"width":NaN,"height":5,"dpi":96}"#
        )
        .is_err());
    }
    #[test]
    fn normal_bounds_round_trip_in_a_private_file() {
        let root = std::env::temp_dir().join(format!("yttraydl-geometry-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("window-state.json");
        let mut state = WindowGeometries::default();
        assert!(state.set(WindowKind::Main, popup()));
        state.save_to(&path).unwrap();
        state.save_to(&path).unwrap();
        assert_eq!(
            WindowGeometries::load_from(&path).get(WindowKind::Main),
            Some(popup())
        );
        std::fs::write(&path, "broken").unwrap();
        assert!(WindowGeometries::load_from(&path)
            .get(WindowKind::Main)
            .is_none());
        std::fs::write(&path, r#"{"schema_version":999}"#).unwrap();
        assert!(WindowGeometries::load_from(&path)
            .get(WindowKind::Main)
            .is_none());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn extreme_public_inputs_never_overflow() {
        let invalid = WindowGeometry {
            x: i32::MIN,
            y: i32::MAX,
            width: i32::MAX,
            height: -1,
            dpi: 0,
        };
        assert_eq!(
            invalid.clamped([i32::MIN, i32::MIN, i32::MAX, i32::MAX], i32::MAX),
            invalid
        );
        let result = popup().clamped([i32::MIN, i32::MIN, i32::MAX, i32::MAX], i32::MIN);
        assert_eq!(result, popup());
    }

    #[test]
    fn scaled_dimensions_crossing_saved_limits_still_clamp() {
        let large = WindowGeometry {
            x: -9000,
            y: 9000,
            width: 10_000,
            height: 1000,
            dpi: 96,
        }
        .at_dpi(192);
        assert_eq!(large.width, 20_000);
        assert_eq!(
            large.clamped([-1920, 0, 0, 1040], 24),
            WindowGeometry {
                x: -1920,
                y: 0,
                ..large
            }
        );
        let small = WindowGeometry {
            x: -9000,
            y: 9000,
            width: 64,
            height: 64,
            dpi: 768,
        }
        .at_dpi(96);
        assert_eq!((small.width, small.height), (8, 8));
        assert_eq!(
            small.clamped([-1920, 0, 0, 1040], 12),
            WindowGeometry {
                x: -1908,
                y: 1020,
                ..small
            }
        );
    }

    #[test]
    fn confirmation_bounds_use_same_negative_origin_clamp() {
        for (width, height) in [(476, 309), (486, 369)] {
            let bounds = WindowGeometry {
                x: -2000,
                y: 1100,
                width,
                height,
                dpi: 96,
            };
            let result = bounds.clamped([-1920, 0, 0, 1040], 12);
            assert_eq!(
                result,
                WindowGeometry {
                    x: -1908,
                    y: 1028 - height,
                    ..bounds
                }
            );
        }
    }
}
