//! Hierarchical meshes (`.him`): a tree of mesh parts with attachment transforms.
//! Spec: `docs/formats/him.md`.

use crate::sectfile::{atof, SectFile};

#[derive(Clone, Debug, PartialEq)]
pub struct HierNode {
    pub name: String,
    /// Mesh file name without extension, relative to the `.him` directory.
    pub mesh: Option<String>,
    pub parent: Option<usize>,
    /// Row-major 3x3 rotation followed by the translation, relative to the parent.
    pub attaching: [f32; 12],
    pub hidden: bool,
    pub separable: bool,
    pub collision_objects: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Hier {
    pub nodes: Vec<HierNode>,
    pub visibility_radius: Option<f32>,
}

pub const IDENTITY: [f32; 12] = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("hier: unknown parent '{parent}' of node '{node}'")]
    UnknownParent { node: String, parent: String },
    #[error("hier: bad Attaching in node '{0}'")]
    BadAttaching(String),
}

impl Hier {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let f = SectFile::parse(data);
        let mut h = Hier::default();
        let mut parents: Vec<Option<String>> = Vec::new();
        for s in &f.sections {
            if s.name.eq_ignore_ascii_case("_ROOT_") {
                h.visibility_radius = s.get_ci("VisibilitySphere").and_then(|v| atof(v.split_whitespace().next()?));
                continue;
            }
            let attaching = match s.get_ci("Attaching") {
                None => IDENTITY,
                Some(v) => {
                    let n: Vec<f32> = v.split_whitespace().filter_map(atof).collect();
                    n.try_into().map_err(|_| Error::BadAttaching(s.name.clone()))?
                }
            };
            parents.push(s.get_ci("Parent").map(str::to_owned));
            h.nodes.push(HierNode {
                name: s.name.clone(),
                mesh: s.get_ci("Mesh").filter(|m| !m.is_empty()).map(str::to_owned),
                parent: None,
                attaching,
                hidden: s.entries.iter().any(|(k, _)| k.eq_ignore_ascii_case("Hidden")),
                separable: s.entries.iter().any(|(k, _)| k.eq_ignore_ascii_case("Separable")),
                collision_objects: s
                    .entries
                    .iter()
                    .filter(|(k, _)| k.eq_ignore_ascii_case("CollisionObject"))
                    .map(|(_, v)| v.clone())
                    .collect(),
            });
        }
        for (i, p) in parents.into_iter().enumerate() {
            let Some(p) = p.filter(|p| !p.eq_ignore_ascii_case("_ROOT_")) else { continue };
            let idx = h
                .nodes
                .iter()
                .position(|n| n.name.eq_ignore_ascii_case(&p))
                .ok_or_else(|| Error::UnknownParent { node: h.nodes[i].name.clone(), parent: p })?;
            h.nodes[i].parent = Some(idx);
        }
        Ok(h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tree() {
        let h = Hier::parse(
            b"[_ROOT_]\nVisibilitySphere 9.9\n[CF_D0]\nMesh CF_D0\nParent _ROOT_\nAttaching 0 1 0 -1 0 0 0 0 1 -1.9 0 0.1\n\
              CollisionObject xcf1\n[CF_D1]\nMesh CF_D1\nParent CF_D0\nHidden\n",
        )
        .unwrap();
        assert_eq!(h.visibility_radius, Some(9.9));
        assert_eq!(h.nodes.len(), 2);
        assert_eq!(h.nodes[0].parent, None);
        assert_eq!(h.nodes[1].parent, Some(0));
        assert!(h.nodes[1].hidden);
        assert_eq!(h.nodes[1].attaching, IDENTITY);
        assert_eq!(h.nodes[0].attaching[9], -1.9);
    }
}
