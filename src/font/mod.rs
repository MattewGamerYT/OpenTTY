use std::collections::HashMap;
use std::sync::Arc;

use fontdb::{Database, Family, Query, Style, Weight};
use fontdue::{Font, FontSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub ch: char,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone)]
pub struct RasterizedGlyph {
    pub width: u32,
    pub height: u32,
    pub xmin: i32,
    pub ymin: i32,
    pub bitmap: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct CellMetrics {
    pub cell_width: f32,
    pub cell_height: f32,
    pub baseline: f32,
}

pub struct FontEngine {
    regular: Font,
    bold: Option<Font>,
    italic: Option<Font>,
    fallback_fonts: Vec<Font>,
    px_size: f32,
    metrics: CellMetrics,
    cache: HashMap<GlyphKey, RasterizedGlyph>,
}

impl FontEngine {
    pub fn load(preferences: &[String], font_size_pt: f32, scale_factor: f32) -> Result<Self, String> {
        let mut db = Database::new();
        db.load_system_fonts();

        let regular = load_best_font(&db, preferences, Weight::NORMAL, Style::Normal)
            .ok_or_else(|| "no suitable system monospace font found".to_string())?;
        let bold = load_best_font(&db, preferences, Weight::BOLD, Style::Normal);
        let italic = load_best_font(&db, preferences, Weight::NORMAL, Style::Italic);

        // Collect additional system monospace/symbol fallback fonts for box-drawing & nerd glyphs.
        let mut fallback_fonts = Vec::new();
        for family_name in [
            "Symbols Nerd Font Mono",
            "Noto Sans Symbols 2",
            "DejaVu Sans Mono",
            "Noto Color Emoji",
        ] {
            if let Some(font) = load_family_font(&db, family_name, Weight::NORMAL, Style::Normal) {
                fallback_fonts.push(font);
            }
        }

        let px_size = (font_size_pt * (96.0 / 72.0) * scale_factor).max(8.0);
        let metrics = compute_cell_metrics(&regular, px_size);

        Ok(Self {
            regular,
            bold,
            italic,
            fallback_fonts,
            px_size,
            metrics,
            cache: HashMap::new(),
        })
    }

    pub fn set_scale(&mut self, font_size_pt: f32, scale_factor: f32) {
        let new_px = (font_size_pt * (96.0 / 72.0) * scale_factor).max(8.0);
        if (new_px - self.px_size).abs() > 0.25 {
            self.px_size = new_px;
            self.metrics = compute_cell_metrics(&self.regular, new_px);
            self.cache.clear();
        }
    }

    pub fn metrics(&self) -> CellMetrics {
        self.metrics
    }

    pub fn rasterize(&mut self, key: GlyphKey) -> &RasterizedGlyph {
        if !self.cache.contains_key(&key) {
            let font = self.select_font(key);
            let (m, bitmap) = font.rasterize(key.ch, self.px_size);
            let glyph = RasterizedGlyph {
                width: m.width as u32,
                height: m.height as u32,
                xmin: m.xmin,
                ymin: m.ymin,
                bitmap,
            };
            self.cache.insert(key, glyph);
        }
        &self.cache[&key]
    }

    fn select_font(&self, key: GlyphKey) -> &Font {
        let primary = if key.bold {
            self.bold.as_ref().unwrap_or(&self.regular)
        } else if key.italic {
            self.italic.as_ref().unwrap_or(&self.regular)
        } else {
            &self.regular
        };

        if primary.lookup_glyph_index(key.ch) != 0 {
            return primary;
        }
        if self.regular.lookup_glyph_index(key.ch) != 0 {
            return &self.regular;
        }
        for fallback in &self.fallback_fonts {
            if fallback.lookup_glyph_index(key.ch) != 0 {
                return fallback;
            }
        }
        primary
    }
}

fn compute_cell_metrics(font: &Font, px_size: f32) -> CellMetrics {
    let (m_metrics, _) = font.rasterize('M', px_size);
    let line_metrics = font.horizontal_line_metrics(px_size);

    let cell_width = m_metrics.advance_width.max(px_size * 0.48).round().max(1.0);
    let (ascent, descent, line_gap) = match line_metrics {
        Some(lm) => (lm.ascent, lm.descent, lm.line_gap),
        None => (px_size * 0.8, -px_size * 0.2, 0.0),
    };
    let baseline = ascent.round().max(1.0);
    let min_span = baseline + (-descent).round();
    let cell_height = (ascent - descent + line_gap * 0.25)
        .round()
        .max(min_span)
        .max((px_size * 1.1).round())
        .max(1.0);

    CellMetrics {
        cell_width,
        cell_height,
        baseline,
    }
}

fn load_best_font(
    db: &Database,
    preferences: &[String],
    weight: Weight,
    style: Style,
) -> Option<Font> {
    for family_name in preferences {
        if let Some(font) = load_family_font(db, family_name, weight, style) {
            return Some(font);
        }
    }

    let query = Query {
        families: &[Family::Monospace],
        weight,
        style,
        ..Default::default()
    };
    let id = db.query(&query)?;
    load_face_by_id(db, id)
}

fn load_family_font(
    db: &Database,
    family_name: &str,
    weight: Weight,
    style: Style,
) -> Option<Font> {
    let families = [Family::Name(family_name)];
    let query = Query {
        families: &families,
        weight,
        style,
        ..Default::default()
    };
    let id = db.query(&query)?;
    load_face_by_id(db, id)
}

fn load_face_by_id(db: &Database, id: fontdb::ID) -> Option<Font> {
    let mut loaded: Option<Font> = None;
    db.with_face_data(id, |data, face_index| {
        let settings = FontSettings {
            collection_index: face_index,
            scale: 40.0,
            ..Default::default()
        };
        if let Ok(font) = Font::from_bytes(Arc::from(data), settings) {
            loaded = Some(font);
        }
    });
    loaded
}
