use serde::Deserialize;
use std::path::PathBuf;

/// TOML config at `~/.config/mdrv-em/config.toml`. Everything optional;
/// unknown keys ignored (forward compatible).
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Conf {
    pub panel: Panel,
    pub font: Font,
    pub search: Search,
    pub bundle: Bundle,
}

impl Conf {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(conf) => conf,
                Err(e) => {
                    eprintln!("mdrv-em: bad config {}: {e}", path.display());
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    Some(
        dirs_home()?
            .join(".config")
            .join("mdrv-em")
            .join("config.toml"),
    )
}

pub fn state_dir() -> Option<PathBuf> {
    if let Ok(x) = std::env::var("XDG_STATE_HOME") {
        if !x.is_empty() {
            return Some(PathBuf::from(x).join("mdrv-em"));
        }
    }
    Some(dirs_home()?.join(".local").join("state").join("mdrv-em"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Panel position: one of nine `edge-edge` / `edge` / `center` combos.
#[derive(Deserialize, Clone, Copy, PartialEq)]
pub enum PanelAnchor {
    #[serde(rename = "top-left")]
    TopLeft,
    #[serde(rename = "top")]
    Top,
    #[serde(rename = "top-right")]
    TopRight,
    #[serde(rename = "left")]
    Left,
    #[serde(rename = "center")]
    Center,
    #[serde(rename = "right")]
    Right,
    #[serde(rename = "bottom-left")]
    BottomLeft,
    #[serde(rename = "bottom")]
    Bottom,
    #[serde(rename = "bottom-right")]
    BottomRight,
}

impl Default for PanelAnchor {
    fn default() -> Self {
        Self::BottomLeft
    }
}

impl PanelAnchor {
    // §16.3: the layer surface fills the output (all anchors), so the
    // 9-way position is applied by flex alignment + margins inside it.
    pub fn is_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::Top | Self::TopRight)
    }
    pub fn is_bottom(self) -> bool {
        matches!(self, Self::BottomLeft | Self::Bottom | Self::BottomRight)
    }
    pub fn is_left(self) -> bool {
        matches!(self, Self::TopLeft | Self::Left | Self::BottomLeft)
    }
    pub fn is_right(self) -> bool {
        matches!(self, Self::TopRight | Self::Right | Self::BottomRight)
    }
    pub fn v_center(self) -> bool {
        !(self.is_top() || self.is_bottom())
    }
    pub fn h_center(self) -> bool {
        !(self.is_left() || self.is_right())
    }
}

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Panel {
    pub anchor: PanelAnchor,
    /// Distance from the anchored edges, px (all four sides).
    pub margin: u32,
    pub width: u32,
    pub height: u32,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            anchor: PanelAnchor::default(),
            margin: 12,
            width: 640,
            height: 420,
        }
    }
}

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Font {
    /// Emoji glyph font: family name or absolute .ttf path.
    pub emoji: String,
    /// UI (chrome) font: family name or absolute .ttf path.
    pub ui: String,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            emoji: "Twemoji".into(),
            ui: String::new(),
        }
    }
}

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Search {
    /// CLDR keyword locales consulted after `en` names/keywords
    /// (lowercased substring fallback). Add "ja" here once the locale
    /// lands in the generated catalog.
    pub fallback_locales: Vec<String>,
    /// Max grid results per query.
    pub max_results: usize,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            fallback_locales: vec!["id".into()],
            max_results: 60,
        }
    }
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Bundle {
    /// Alternate engine bundle dir (model.onnx + vectors.bin + meta.json).
    /// Empty = the embedded default bundle.
    pub path: String,
}
