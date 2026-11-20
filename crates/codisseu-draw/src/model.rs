use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Result;

pub const MAX_SCENE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ELEMENTS: usize = 20_000;
const MAX_POINTS: usize = 200_000;
const MAX_COORDINATE: f64 = 10_000_000.0;
pub type Point = [f64; 2];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElementKind {
    Rectangle,
    Ellipse,
    Diamond,
    Arrow,
    Line,
    Freedraw,
    Text,
    Other(String),
}

impl ElementKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Rectangle => "rectangle",
            Self::Ellipse => "ellipse",
            Self::Diamond => "diamond",
            Self::Arrow => "arrow",
            Self::Line => "line",
            Self::Freedraw => "freedraw",
            Self::Text => "text",
            Self::Other(kind) => kind,
        }
    }

    pub fn is_linear(&self) -> bool {
        matches!(self, Self::Arrow | Self::Line | Self::Freedraw)
    }

    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl Serialize for ElementKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ElementKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "rectangle" => Self::Rectangle,
            "ellipse" => Self::Ellipse,
            "diamond" => Self::Diamond,
            "arrow" => Self::Arrow,
            "line" => Self::Line,
            "freedraw" => Self::Freedraw,
            "text" => Self::Text,
            _ => Self::Other(value),
        })
    }
}

/// Fields without a native editing equivalent are preserved without interpretation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ElementKind,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    #[serde(default)]
    pub angle: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<Point>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default = "default_stroke")]
    pub stroke_color: String,
    #[serde(default = "default_background")]
    pub background_color: String,
    #[serde(default = "default_stroke_width")]
    pub stroke_width: f64,
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    #[serde(default = "default_font_size")]
    pub font_size: f64,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_stroke() -> String {
    "#1b1b1f".into()
}
fn default_background() -> String {
    "transparent".into()
}
fn default_stroke_width() -> f64 {
    2.0
}
fn default_opacity() -> f64 {
    100.0
}
fn default_font_size() -> f64 {
    20.0
}

pub(crate) fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Element {
    pub fn new(kind: ElementKind, x: f64, y: f64, width: f64, height: f64) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let nonce = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let now = timestamp();
        let linear = kind.is_linear();
        let is_arrow = kind == ElementKind::Arrow;
        let mut extra = json!({
            "version": 1, "versionNonce": nonce, "updated": now, "created": now,
            "seed": nonce % 2_147_483_647, "fillStyle": "solid", "strokeStyle": "solid",
            "roughness": 0, "groupIds": [], "frameId": null, "roundness": null,
            "boundElements": null, "link": null, "locked": false,
        })
        .as_object()
        .expect("object literal")
        .clone();
        if linear {
            extra.insert("startBinding".into(), Value::Null);
            extra.insert("endBinding".into(), Value::Null);
            extra.insert("startArrowhead".into(), Value::Null);
            extra.insert(
                "endArrowhead".into(),
                if is_arrow {
                    json!("arrow")
                } else {
                    Value::Null
                },
            );
        }
        if kind == ElementKind::Freedraw {
            extra.insert("pressures".into(), json!([]));
            extra.insert("simulatePressure".into(), json!(true));
        }
        if kind == ElementKind::Text {
            extra.extend(json!({"fontFamily": 2, "textAlign": "left", "verticalAlign": "top", "containerId": null, "originalText": "", "autoResize": true, "lineHeight": 1.25}).as_object().expect("object literal").clone());
        }
        Self {
            id: format!("codisseu-{now:x}-{nonce:x}"),
            kind,
            x,
            y,
            width,
            height,
            angle: 0.0,
            points: if linear {
                vec![[0.0, 0.0], [width, height]]
            } else {
                vec![]
            },
            text: String::new(),
            stroke_color: default_stroke(),
            background_color: default_background(),
            stroke_width: default_stroke_width(),
            opacity: default_opacity(),
            font_size: default_font_size(),
            is_deleted: false,
            extra,
        }
    }

    /// Keep Excalidraw's reconciliation metadata current after a committed edit.
    pub fn touch(&mut self) {
        let version = self
            .extra
            .get("version")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let nonce = self
            .extra
            .get("versionNonce")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.extra
            .insert("version".into(), json!(version.saturating_add(1)));
        self.extra.insert(
            "versionNonce".into(),
            json!((nonce.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)) % 2_147_483_647),
        );
        self.extra.insert("updated".into(), json!(timestamp()));
        if self.kind == ElementKind::Text {
            self.extra.insert("originalText".into(), json!(self.text));
        }
    }

    pub fn bounds(&self) -> [f64; 4] {
        if self.kind.is_linear() && !self.points.is_empty() {
            let mut bounds = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for [x, y] in &self.points {
                bounds[0] = bounds[0].min(self.x + x);
                bounds[1] = bounds[1].min(self.y + y);
                bounds[2] = bounds[2].max(self.x + x);
                bounds[3] = bounds[3].max(self.y + y);
            }
            bounds
        } else {
            [
                self.x.min(self.x + self.width),
                self.y.min(self.y + self.height),
                self.x.max(self.x + self.width),
                self.y.max(self.y + self.height),
            ]
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    #[serde(rename = "type")]
    pub scene_type: String,
    pub version: u32,
    #[serde(default = "default_source")]
    pub source: String,
    pub elements: Vec<Element>,
    #[serde(default)]
    pub app_state: Map<String, Value>,
    #[serde(default)]
    pub files: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_source() -> String {
    "https://github.com/lucstrik/codisseu".into()
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            scene_type: "excalidraw".into(),
            version: 2,
            source: default_source(),
            elements: vec![],
            app_state: json!({"viewBackgroundColor":"#ffffff", "gridSize":20})
                .as_object()
                .expect("object literal")
                .clone(),
            files: Map::new(),
            extra: Map::new(),
        }
    }
}

impl Scene {
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > MAX_SCENE_BYTES {
            return Err("Desenho excede o limite de 16 MiB".into());
        }
        let scene: Self =
            serde_json::from_str(text).map_err(|e| format!("Arquivo Excalidraw inválido: {e}"))?;
        scene.validate()?;
        Ok(scene)
    }

    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Falha ao serializar desenho: {e}"))?;
        if text.len() > MAX_SCENE_BYTES {
            return Err("Desenho excede o limite de 16 MiB".into());
        }
        Ok(text)
    }

    pub fn validate(&self) -> Result<()> {
        if self.scene_type != "excalidraw" || self.version != 2 {
            return Err(
                "Formato Excalidraw não suportado; esperado type=excalidraw e version=2".into(),
            );
        }
        if self.elements.len() > MAX_ELEMENTS {
            return Err(format!("Desenho excede {MAX_ELEMENTS} elementos"));
        }
        let mut ids = HashSet::with_capacity(self.elements.len());
        let mut points = 0usize;
        for el in &self.elements {
            if el.id.is_empty() || el.id.len() > 1024 || !ids.insert(&el.id) {
                return Err("Desenho contém ID de elemento inválido ou repetido".into());
            }
            for number in [
                el.x,
                el.y,
                el.width,
                el.height,
                el.angle,
                el.stroke_width,
                el.opacity,
                el.font_size,
            ]
            .into_iter()
            .chain(el.points.iter().flatten().copied())
            {
                if !number.is_finite() || number.abs() > MAX_COORDINATE {
                    return Err(format!("Geometria inválida no elemento {}", el.id));
                }
            }
            if el.stroke_width < 0.0 || !(0.0..=100.0).contains(&el.opacity) || el.font_size <= 0.0
            {
                return Err(format!("Estilo inválido no elemento {}", el.id));
            }
            points = points.saturating_add(el.points.len());
            if points > MAX_POINTS {
                return Err(format!("Desenho excede {MAX_POINTS} pontos"));
            }
        }
        Ok(())
    }

    pub fn unsupported_count(&self) -> usize {
        self.elements
            .iter()
            .filter(|el| !el.is_deleted && !el.kind.is_supported())
            .count()
    }

    pub fn export_svg(&self) -> Result<String> {
        crate::svg::export(self)
    }
}
