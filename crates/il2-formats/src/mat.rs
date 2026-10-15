//! Materials (`.mat`): `docs/formats/mat.md`.

use crate::sectfile::{SectFile, Section};

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Texture path relative to the material's directory; `None` for untextured layers.
    pub texture: Option<String>,
    pub alpha_test: Option<f32>,
    pub blend: bool,
    pub blend_add: bool,
    pub no_write_z: bool,
    pub depth_offset: bool,
    pub color_scale: [f32; 4],
    pub tex_coord_scale: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub double_side: bool,
    pub should_sort: bool,
    pub ambient: f32,
    pub diffuse: f32,
    pub specular: f32,
    pub specular_pow: f32,
    pub layers: Vec<Layer>,
}

/// Raw material sections, before `BasedOn` inheritance is resolved.
#[derive(Clone, Debug, Default)]
pub struct MatFile {
    pub based_on: Option<String>,
    pub file: SectFile,
}

impl MatFile {
    pub fn parse(data: &[u8]) -> Self {
        let file = SectFile::parse(data);
        let based_on = file.section_ci("ClassInfo").and_then(|s| s.get_ci("BasedOn")).filter(|b| !b.is_empty()).map(str::to_owned);
        MatFile { based_on, file }
    }
}

fn flag(chain: &[&MatFile], sect: &str, key: &str) -> Option<bool> {
    lookup(chain, sect, key).and_then(|v| v.split_whitespace().next()?.parse::<i32>().ok()).map(|v| v != 0)
}

fn num(chain: &[&MatFile], sect: &str, key: &str) -> Option<f32> {
    lookup(chain, sect, key).and_then(|v| v.split_whitespace().next()?.parse().ok())
}

fn vec4(chain: &[&MatFile], sect: &str, key: &str, default: [f32; 4]) -> [f32; 4] {
    lookup(chain, sect, key)
        .and_then(|v| v.split_whitespace().map(|t| t.parse().ok()).collect::<Option<Vec<f32>>>()?.try_into().ok())
        .unwrap_or(default)
}

/// Looks a key up in the most-derived material first, then in its bases.
fn lookup<'a>(chain: &[&'a MatFile], sect: &str, key: &str) -> Option<&'a str> {
    chain.iter().find_map(|m| m.file.section_ci(sect).and_then(|s: &Section| s.get_ci(key)))
}

impl Material {
    /// Resolves a material from its `BasedOn` chain, most-derived first.
    pub fn resolve(chain: &[&MatFile]) -> Self {
        let has_layer = |i: usize| chain.iter().any(|m| m.file.section_ci(&format!("Layer{i}")).is_some());
        let layers = (0..)
            .take_while(|&i| has_layer(i))
            .map(|i| {
                let s = format!("Layer{i}");
                let no_texture = flag(chain, &s, "tfNoTexture").unwrap_or(false);
                Layer {
                    texture: lookup(chain, &s, "TextureName")
                        .map(str::trim)
                        .filter(|t| !t.is_empty() && !no_texture)
                        .map(str::to_owned),
                    alpha_test: flag(chain, &s, "tfTestA")
                        .unwrap_or(false)
                        .then(|| num(chain, &s, "AlphaTestVal").unwrap_or(0.5)),
                    blend: flag(chain, &s, "tfBlend").unwrap_or(false),
                    blend_add: flag(chain, &s, "tfBlendAdd").unwrap_or(false),
                    no_write_z: flag(chain, &s, "tfNoWriteZ").unwrap_or(false),
                    depth_offset: flag(chain, &s, "tfDepthOffset").unwrap_or(false)
                        || flag(chain, "General", "tfDepthOffset").unwrap_or(false),
                    color_scale: vec4(chain, &s, "ColorScale", [1.0; 4]),
                    tex_coord_scale: vec4(chain, &s, "TextureCoordScale", [0.0, 0.0, 1.0, 1.0]),
                }
            })
            .collect();
        Material {
            double_side: flag(chain, "General", "tfDoubleSide").unwrap_or(false),
            should_sort: flag(chain, "General", "tfShouldSort").unwrap_or(false),
            ambient: num(chain, "LightParams", "Ambient").unwrap_or(1.0),
            diffuse: num(chain, "LightParams", "Diffuse").unwrap_or(1.0),
            specular: num(chain, "LightParams", "Specular").unwrap_or(0.0),
            specular_pow: num(chain, "LightParams", "SpecularPow").unwrap_or(1.0),
            layers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inheritance() {
        let base = MatFile::parse(
            b"[ClassInfo]\nClassName TMaterial\n[General]\ntfDoubleSide 1\n[Layer0]\nTextureName base.tga\ntfTestA 1\nAlphaTestVal 0.9\n",
        );
        let derived = MatFile::parse(b"[ClassInfo]\nBasedOn base.mat\n[Layer0]\nTextureName skin1o.tga\n[_Layer1]\nTextureName x.tga\n");
        assert_eq!(derived.based_on.as_deref(), Some("base.mat"));
        let m = Material::resolve(&[&derived, &base]);
        assert!(m.double_side);
        assert_eq!(m.layers.len(), 1);
        assert_eq!(m.layers[0].texture.as_deref(), Some("skin1o.tga"));
        assert_eq!(m.layers[0].alpha_test, Some(0.9));
    }
}
