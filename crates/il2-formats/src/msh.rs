//! Meshes: `docs/formats/msh.md`.

use crate::binsect::{RowFile, RowSection, Value};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    BinSect(#[from] crate::binsect::Error),
    #[error("mesh: missing section [{0}]")]
    MissingSection(String),
    #[error("mesh: bad data in [{0}]")]
    BadData(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FaceGroup {
    pub material: usize,
    pub start_vertex: u32,
    pub vertex_count: u32,
    pub start_face: u32,
    pub face_count: u32,
}

/// One level of detail, frame 0.
#[derive(Clone, Debug, Default)]
pub struct Lod {
    pub materials: Vec<String>,
    pub groups: Vec<FaceGroup>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Triangles with absolute vertex indices.
    pub faces: Vec<[u32; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hook {
    pub name: String,
    pub parent: String,
    /// Row-major 3x3 rotation followed by the translation.
    pub matrix: [f32; 12],
}

/// A collision part (`Co*` sections, `docs/formats/msh.md`): a closed hull in mesh coordinates.
#[derive(Clone, Debug, Default)]
pub struct CoPart {
    pub name: String,
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    /// Switch-out distance of each LOD; empty for single-LOD meshes.
    pub lod_distances: Vec<f32>,
    pub lods: Vec<Lod>,
    pub hooks: Vec<Hook>,
    pub collision: Vec<CoPart>,
}

/// Reads `rows x width` numbers. Like the engine, numeric sections are treated as one contiguous
/// stream, so a malformed row shifts the following values instead of failing; a short stream is
/// padded with zeros.
fn floats(sect: &RowSection, width: usize) -> Result<Vec<Vec<f32>>, Error> {
    let flat: Vec<f32> = sect
        .rows
        .iter()
        .flatten()
        .map(|v| v.as_f32().ok_or_else(|| Error::BadData(sect.name.clone())))
        .collect::<Result<_, _>>()?;
    Ok((0..sect.rows.len())
        .map(|r| {
            (0..width)
                .map(|c| flat.get(r * width + c).copied().unwrap_or(0.0))
                .collect()
        })
        .collect())
}

fn ints(row: &[Value]) -> Option<Vec<i64>> {
    row.iter().map(|v| v.as_i32().map(|i| i as i64)).collect()
}

impl Mesh {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        Self::from_rows(&RowFile::parse(data)?)
    }

    pub fn from_rows(f: &RowFile) -> Result<Self, Error> {
        let lod_distances: Vec<f32> = f
            .section("LOD")
            .map(|s| s.rows.iter().filter_map(|r| r.first()?.as_f32()).collect())
            .unwrap_or_default();
        let mut lods = vec![parse_lod(f, "")?];
        for i in 1..lod_distances.len() {
            match parse_lod(f, &format!("LOD{i}_")) {
                Ok(l) => lods.push(l),
                Err(Error::MissingSection(_)) => break,
                Err(e) => return Err(e),
            }
        }
        let hooks = match (f.section("Hooks"), f.section("HookLoc")) {
            (Some(h), Some(loc)) => {
                let m = floats(loc, 12)?;
                h.rows
                    .iter()
                    .zip(m)
                    .map(|(r, m)| Hook {
                        name: r
                            .first()
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        parent: r
                            .get(1)
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        matrix: m.try_into().unwrap(),
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        Ok(Mesh {
            lod_distances,
            lods,
            hooks,
            collision: collision(f),
        })
    }
}

/// The collision parts of every block (first frame); unreadable parts are skipped.
fn collision(f: &RowFile) -> Vec<CoPart> {
    let count = |name: &str, key: &str| -> usize {
        f.section(name)
            .and_then(|s| {
                s.rows
                    .iter()
                    .find(|r| r.first().and_then(Value::as_str) == Some(key))
            })
            .and_then(|r| r.get(1)?.as_i32())
            .map_or(0, |n| n.max(0) as usize)
    };
    let mut out = Vec::new();
    for b in 0..count("CoCommon", "NBlocks") {
        for p in 0..count(&format!("CoCommon_b{b}"), "NParts") {
            let Some(head) = f.section(&format!("CoCommon_b{b}p{p}")) else {
                continue;
            };
            let name = head
                .rows
                .iter()
                .find(|r| r.first().and_then(Value::as_str) == Some("Name"))
                .and_then(|r| r.get(1)?.as_str())
                .unwrap_or_default()
                .to_owned();
            let (Some(v), Some(fc)) = (
                f.section(&format!("CoVer0_b{b}p{p}")),
                f.section(&format!("CoFac_b{b}p{p}")),
            ) else {
                continue;
            };
            let Ok(vertices) = floats(v, 3) else { continue };
            let vertices: Vec<[f32; 3]> = vertices.iter().map(|r| [r[0], r[1], r[2]]).collect();
            let faces: Vec<[u32; 3]> = fc
                .rows
                .iter()
                .filter_map(|r| {
                    let i = ints(r)?;
                    (i.len() >= 3
                        && i[..3]
                            .iter()
                            .all(|&k| k >= 0 && (k as usize) < vertices.len()))
                    .then(|| [i[0] as u32, i[1] as u32, i[2] as u32])
                })
                .collect();
            out.push(CoPart {
                name,
                vertices,
                faces,
            });
        }
    }
    out
}

fn parse_lod(f: &RowFile, prefix: &str) -> Result<Lod, Error> {
    let get = |n: &str| {
        let name = format!("{prefix}{n}");
        f.section(&name).ok_or(Error::MissingSection(name))
    };
    let materials = get("Materials")?
        .rows
        .iter()
        .map(|r| {
            r.first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    let fg = get("FaceGroups")?;
    let bad = || Error::BadData(fg.name.clone());
    let header = fg.rows.first().and_then(|r| ints(r)).ok_or_else(bad)?;
    let (nverts, nfaces) = (
        *header.first().ok_or_else(bad)? as usize,
        *header.get(1).ok_or_else(bad)? as usize,
    );
    let groups = fg.rows[1..]
        .iter()
        .map(|r| {
            let v = ints(r)
                .filter(|v| v.len() >= 5 && v.iter().take(5).all(|&x| x >= 0))
                .ok_or_else(bad)?;
            Ok(FaceGroup {
                material: v[0] as usize,
                start_vertex: v[1] as u32,
                vertex_count: v[2] as u32,
                start_face: v[3] as u32,
                face_count: v[4] as u32,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let verts = floats(get("Vertices_Frame0")?, 6)?;
    let uvs = floats(get("MaterialMapping")?, 2)?;
    let faces_sect = get("Faces")?;
    if verts.len() != nverts || uvs.len() != nverts || faces_sect.rows.len() != nfaces {
        return Err(bad());
    }
    let mut faces = vec![[0u32; 3]; nfaces];
    for g in &groups {
        let (fs, fc) = (g.start_face as usize, g.face_count as usize);
        let rows = faces_sect.rows.get(fs..fs + fc).ok_or_else(bad)?;
        for (dst, r) in faces[fs..fs + fc].iter_mut().zip(rows) {
            let v = ints(r)
                .filter(|v| v.len() == 3)
                .ok_or_else(|| Error::BadData(faces_sect.name.clone()))?;
            for k in 0..3 {
                if v[k] < 0 || v[k] as u32 >= g.vertex_count {
                    return Err(Error::BadData(faces_sect.name.clone()));
                }
                dst[k] = g.start_vertex + v[k] as u32;
            }
        }
    }
    Ok(Lod {
        materials,
        groups,
        positions: verts.iter().map(|v| [v[0], v[1], v[2]]).collect(),
        normals: verts.iter().map(|v| [v[3], v[4], v[5]]).collect(),
        uvs: uvs.iter().map(|v| [v[0], v[1]]).collect(),
        faces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_mesh() {
        let text =
            b"[Common]\nNumBones 0\n[LOD]\n50\n[Materials]\nskin\n[FaceGroups]\n4 2\n0 0 4 0 2 0\n\
            [Vertices_Frame0]\n0 0 0 0 0 1\n1 0 0 0 0 1\n1 1 0 0 0 1\n0 1 0 0 0 1\n\
            [MaterialMapping]\n0 0\n1 0\n1 1\n0 1\n[Faces]\n0 1 2\n0 2 3\n";
        let m = Mesh::parse(text).unwrap();
        assert_eq!(m.lods.len(), 1);
        assert_eq!(m.lods[0].faces, [[0, 1, 2], [0, 2, 3]]);
        assert_eq!(m.lods[0].materials, ["skin"]);
    }
}
