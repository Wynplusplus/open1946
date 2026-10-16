//! Heads-up display: speed bar, messages and a simple gunsight, as screen-space overlay quads. Text
//! is rasterised from a TrueType font of the user's install (`lib/fonts/LucidaSansRegular.ttf`).

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use il2_render::{Image, OverlayVertex};
use std::collections::HashMap;

const ATLAS: u32 = 512;
const PX: f32 = 22.0;

struct Glyph {
    uv: [f32; 4],
    size: [f32; 2],
    offset: [f32; 2],
    advance: f32,
}

pub struct Hud {
    glyphs: HashMap<char, Glyph>,
    line: f32,
    /// Timed messages (text, seconds left).
    messages: Vec<(String, f64)>,
    pub atlas: Option<Image>,
    verts: Vec<OverlayVertex>,
}

impl Hud {
    pub fn new(font_path: &std::path::Path) -> Self {
        let mut hud = Hud {
            glyphs: HashMap::new(),
            line: PX,
            messages: Vec::new(),
            atlas: None,
            verts: Vec::new(),
        };
        let Some(font) = std::fs::read(font_path)
            .ok()
            .and_then(|d| FontVec::try_from_vec(d).ok())
        else {
            eprintln!("hud: no font at {}", font_path.display());
            return hud;
        };
        let scaled = font.as_scaled(PxScale::from(PX));
        hud.line = scaled.height() + scaled.line_gap();
        let mut rgba = vec![0u8; (ATLAS * ATLAS * 4) as usize];
        // A white block at the corner for solid shapes.
        for y in 0..4 {
            for x in 0..4 {
                rgba[((y * ATLAS + x) * 4) as usize..][..4].copy_from_slice(&[255; 4]);
            }
        }
        let (mut x, mut y, mut row) = (8u32, 0u32, 0u32);
        for c in (32u8..127).map(char::from).chain("°".chars()) {
            let g = scaled.scaled_glyph(c);
            let advance = scaled.h_advance(g.id);
            let Some(outlined) = font.outline_glyph(g) else {
                hud.glyphs.insert(
                    c,
                    Glyph {
                        uv: [0.0; 4],
                        size: [0.0; 2],
                        offset: [0.0; 2],
                        advance,
                    },
                );
                continue;
            };
            let b = outlined.px_bounds();
            let (w, h) = (b.width().ceil() as u32 + 1, b.height().ceil() as u32 + 1);
            if x + w >= ATLAS {
                x = 0;
                y += row + 1;
                row = 0;
            }
            outlined.draw(|gx, gy, cov| {
                let (px, py) = (x + gx, y + gy);
                if px < ATLAS && py < ATLAS {
                    let i = ((py * ATLAS + px) * 4) as usize;
                    rgba[i..i + 4].copy_from_slice(&[255, 255, 255, (cov * 255.0) as u8]);
                }
            });
            let a = ATLAS as f32;
            hud.glyphs.insert(
                c,
                Glyph {
                    uv: [
                        x as f32 / a,
                        y as f32 / a,
                        (x + w) as f32 / a,
                        (y + h) as f32 / a,
                    ],
                    size: [w as f32, h as f32],
                    offset: [b.min.x, b.min.y + scaled.ascent()],
                    advance,
                },
            );
            x += w + 1;
            row = row.max(h);
        }
        hud.atlas = Some(Image {
            width: ATLAS,
            height: ATLAS,
            rgba,
        });
        hud
    }

    /// Shows a message for a few seconds (newest last).
    pub fn message(&mut self, text: impl Into<String>) {
        self.messages.push((text.into(), 5.0));
        if self.messages.len() > 6 {
            self.messages.remove(0);
        }
    }

    fn quad(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, uv: [f32; 4], color: [f32; 4]) {
        let v = |x, y, u, w| OverlayVertex {
            pos: [x, y],
            uv: [u, w],
            color,
        };
        self.verts.extend([
            v(x0, y0, uv[0], uv[1]),
            v(x1, y0, uv[2], uv[1]),
            v(x1, y1, uv[2], uv[3]),
            v(x0, y0, uv[0], uv[1]),
            v(x1, y1, uv[2], uv[3]),
            v(x0, y1, uv[0], uv[3]),
        ]);
    }

    /// A solid rectangle.
    pub fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) {
        let w = 2.0 / ATLAS as f32;
        self.quad(
            x0,
            y0,
            x1,
            y1,
            [0.5 / ATLAS as f32, 0.5 / ATLAS as f32, w, w],
            color,
        );
    }

    /// Text with a dark shadow, top-left at (x, y).
    pub fn text(&mut self, x: f32, y: f32, s: &str, color: [f32; 4]) {
        for (dx, dy, c) in [
            (1.5, 1.5, [0.0, 0.0, 0.0, color[3] * 0.8]),
            (0.0, 0.0, color),
        ] {
            let mut pen = x + dx;
            for ch in s.chars() {
                let Some(g) = self.glyphs.get(&ch) else {
                    continue;
                };
                let (uv, size, off, adv) = (g.uv, g.size, g.offset, g.advance);
                if size[0] > 0.0 {
                    self.quad(
                        pen + off[0],
                        y + dy + off[1],
                        pen + off[0] + size[0],
                        y + dy + off[1] + size[1],
                        uv,
                        c,
                    );
                }
                pen += adv;
            }
        }
    }

    /// Builds this frame's overlay: `lines` in the bottom-left speed bar, messages above, and the
    /// gunsight when `sight` is set.
    pub fn build(
        &mut self,
        w: f32,
        h: f32,
        lines: &[String],
        sight: bool,
        dt: f64,
    ) -> &[OverlayVertex] {
        self.verts.clear();
        for m in &mut self.messages {
            m.1 -= dt;
        }
        self.messages.retain(|m| m.1 > 0.0);
        let white = [0.95, 0.95, 0.85, 1.0];
        let mut y = h - 12.0 - self.line * lines.len() as f32;
        for l in lines {
            self.text(12.0, y, l, white);
            y += self.line;
        }
        let mut y = h - 24.0 - self.line * (lines.len() + self.messages.len()) as f32;
        let msgs: Vec<(String, f64)> = self.messages.clone();
        for (m, t) in msgs {
            self.text(12.0, y, &m, [1.0, 0.85, 0.4, t.min(1.0) as f32]);
            y += self.line;
        }
        if sight {
            let (cx, cy) = (w * 0.5, h * 0.5);
            let c = [1.0, 0.55, 0.2, 0.85];
            self.rect(cx - 30.0, cy - 0.75, cx - 6.0, cy + 0.75, c);
            self.rect(cx + 6.0, cy - 0.75, cx + 30.0, cy + 0.75, c);
            self.rect(cx - 0.75, cy + 6.0, cx + 0.75, cy + 30.0, c);
            self.rect(cx - 1.5, cy - 1.5, cx + 1.5, cy + 1.5, c);
        }
        &self.verts
    }
}
