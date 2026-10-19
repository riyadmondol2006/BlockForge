// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.

//! Biomes (`intermediate.column.biome`): the biome type and the four storage layouts a column can carry, with the
//! lossy conversions between them.

use std::sync::Arc;

use super::generated::{BIOME_IDS, BIOME_INFO, VanillaBiome};
use super::palette::Palette;

impl VanillaBiome {
    /// The Java identifier (`minecraft:plains`), if the biome exists on Java.
    pub fn java_identifier(self) -> Option<&'static str> {
        BIOME_INFO[self as usize].java
    }

    /// The Bedrock identifier (`minecraft:ice_plains` for snowy plains), if the biome exists on Bedrock.
    pub fn bedrock_identifier(self) -> Option<&'static str> {
        BIOME_INFO[self as usize].bedrock
    }

    /// The biome used instead when this one has been removed (the deprecated hills, mountains and so on).
    pub fn fallback(self) -> Option<VanillaBiome> {
        BIOME_INFO[self as usize].fallback
    }

    /// Look up a biome by identifier; the Java name wins, the Bedrock name is an alias (`find`).
    pub fn find(identifier: &str) -> Option<VanillaBiome> {
        BIOME_IDS.binary_search_by(|(id, _)| (*id).cmp(identifier)).ok().map(|i| BIOME_IDS[i].1)
    }
}

/// A biome: a vanilla one, or a custom one known by its identifier (`ChunkerCustomBiome`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Biome {
    Vanilla(VanillaBiome),
    Custom(Arc<str>),
}

impl Biome {
    /// The biome for an identifier: vanilla when known, else custom.
    pub fn from_identifier(identifier: &str) -> Biome {
        match VanillaBiome::find(identifier) {
            Some(v) => Biome::Vanilla(v),
            None => Biome::Custom(Arc::from(identifier)),
        }
    }

    /// The replacement biome (`getFallback`); custom biomes have none.
    pub fn fallback(&self) -> Option<VanillaBiome> {
        match self {
            Biome::Vanilla(v) => v.fallback(),
            Biome::Custom(_) => None,
        }
    }
}

impl From<VanillaBiome> for Biome {
    fn from(value: VanillaBiome) -> Self {
        Biome::Vanilla(value)
    }
}

/// Palette of per-block biomes (16 cube) in a section.
pub type BiomePalette = Palette<Biome, 16>;
/// Palette of biome cells (4 cube) in a section.
pub type CellPalette = Palette<Biome, 4>;

/// The biomes of a column in one of Chunker's four layouts.
#[derive(Clone, Debug, PartialEq)]
pub enum Biomes {
    /// `ChunkerColumnBasedBiomes`: 256 entries, index `z << 4 | x` (legacy Java).
    Column(Vec<Biome>),
    /// `ChunkerClusterBasedBiomes`: 1024 cells of 4x4x4, index `chunk_y << 6 | y4 << 4 | z4 << 2 | x4`.
    Cluster(Vec<Biome>),
    /// `ChunkerPaletteBasedBiomes`: one 16 cube palette per section (Bedrock 3D).
    Palette(Vec<BiomePalette>),
    /// `ChunkerClusterPaletteBasedBiomes`: one 4 cube palette per section (Java 1.18+).
    ClusterPalette(Vec<CellPalette>),
}

impl Biomes {
    /// Per column biomes (256 entries, `z << 4 | x`), using `fallback` where nothing is known.
    pub fn as_column(&self, fallback: &Biome) -> Vec<Biome> {
        match self {
            Biomes::Column(columns) => columns.clone(),
            Biomes::Cluster(clusters) => {
                let mut out = vec![fallback.clone(); 256];
                for i in 0..4 {
                    for j in 0..4 {
                        let new_index = (i << 2) << 4 | (j << 2);
                        let biome = &clusters[i << 2 | j];
                        for k in 0..4 {
                            let at = new_index + (k << 4);
                            out[at..at + 4].fill(biome.clone());
                        }
                    }
                }
                out
            }
            Biomes::Palette(chunks) => {
                let mut out = vec![fallback.clone(); 256];
                if let Some(palette) = chunks.first() {
                    for (i, slot) in out.iter_mut().enumerate() {
                        *slot = palette.get_or(i & 0xf, 0, (i >> 4) & 0xf, fallback).clone();
                    }
                }
                out
            }
            Biomes::ClusterPalette(chunks) => {
                let mut out = vec![fallback.clone(); 256];
                if let Some(palette) = chunks.first() {
                    for i in 0..4 {
                        for j in 0..4 {
                            let new_index = (i << 2) << 4 | (j << 2);
                            let biome = palette.get_or(j, 0, i, fallback);
                            for k in 0..4 {
                                let at = new_index + (k << 4);
                                out[at..at + 4].fill(biome.clone());
                            }
                        }
                    }
                }
                out
            }
        }
    }

    /// Biome cells (1024 entries, `chunk_y << 6 | y4 << 4 | z4 << 2 | x4`) for the first 16 sections.
    pub fn as_4x4(&self, fallback: &Biome) -> Vec<Biome> {
        match self {
            Biomes::Column(columns) => {
                let mut out = vec![fallback.clone(); 1024];
                for i in 0..4 {
                    for j in 0..4 {
                        let old_index = ((i << 2) + 2) << 4 | ((j << 2) + 2);
                        out[i << 2 | j] = columns[old_index].clone();
                    }
                }
                // The 16 cells repeat for every vertical layer.
                for layer in 1..64 {
                    for k in 0..16 {
                        out[(layer << 4) + k] = out[k].clone();
                    }
                }
                out
            }
            Biomes::Cluster(clusters) => clusters.clone(),
            Biomes::Palette(chunks) => {
                let mut out = vec![fallback.clone(); 1024];
                if let Some(last) = chunks.last() {
                    for chunk_y in 0..16 {
                        let palette = chunks.get(chunk_y).unwrap_or(last);
                        for local_x in 0..4 {
                            for local_y in 0..4 {
                                for local_z in 0..4 {
                                    // Centre point of each cell.
                                    let value = palette.get_or(
                                        (local_x << 2) + 2,
                                        (local_y << 2) + 2,
                                        (local_z << 2) + 2,
                                        fallback,
                                    );
                                    out[chunk_y << 6 | local_y << 4 | local_z << 2 | local_x] = value.clone();
                                }
                            }
                        }
                    }
                }
                out
            }
            Biomes::ClusterPalette(chunks) => {
                let mut out = vec![fallback.clone(); 1024];
                if let Some(last) = chunks.last() {
                    for chunk_y in 0..16 {
                        let palette = chunks.get(chunk_y).unwrap_or(last);
                        for local_x in 0..4 {
                            for local_y in 0..4 {
                                for local_z in 0..4 {
                                    let value = palette.get_or(local_x, local_y, local_z, fallback);
                                    out[chunk_y << 6 | local_y << 4 | local_z << 2 | local_x] = value.clone();
                                }
                            }
                        }
                    }
                }
                out
            }
        }
    }

    /// Per section 16 cube palettes (`asPalette`).
    pub fn as_palettes(&self) -> Vec<BiomePalette> {
        match self {
            Biomes::Column(columns) => {
                let mut palette = BiomePalette::Empty;
                palette.make_writeable();
                for (i, entry) in columns.iter().enumerate() {
                    let (x, z) = (i & 0xf, (i >> 4) & 0xf);
                    let key = palette.get_or_create_key(entry.clone());
                    for y in 0..16 {
                        palette.set_index(x, y, z, key);
                    }
                }
                vec![palette]
            }
            Biomes::Cluster(clusters) => (0..16)
                .map(|chunk_y| {
                    let mut palette = BiomePalette::Empty;
                    palette.make_writeable();
                    for cluster_x in 0..4 {
                        for cluster_y in 0..4 {
                            for cluster_z in 0..4 {
                                let old_index = chunk_y << 6 | cluster_y << 4 | cluster_z << 2 | cluster_x;
                                // A missing cluster leaves the voxels at index 0.
                                let Some(biome) = clusters.get(old_index) else { continue };
                                let key = palette.get_or_create_key(biome.clone());
                                for lx in 0..4 {
                                    for ly in 0..4 {
                                        for lz in 0..4 {
                                            palette.set_index(
                                                (cluster_x << 2) + lx,
                                                (cluster_y << 2) + ly,
                                                (cluster_z << 2) + lz,
                                                key,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    palette
                })
                .collect(),
            Biomes::Palette(chunks) => chunks.clone(),
            Biomes::ClusterPalette(chunks) => chunks.iter().map(upsample_cells).collect(),
        }
    }

    /// The 4 cube palette of one section (`as4X4Palette`); sections above or below the stored ones use the last
    /// one (`Cluster` uses sections 0 to 15).
    pub fn as_4x4_palette(&self, chunk_y: i32) -> CellPalette {
        match self {
            Biomes::Column(columns) => {
                let mut palette = CellPalette::Empty;
                palette.make_writeable();
                for i in 0..4 {
                    for j in 0..4 {
                        let old_index = ((i << 2) + 2) << 4 | ((j << 2) + 2);
                        let key = palette.get_or_create_key(columns[old_index].clone());
                        for k in 0..4 {
                            palette.set_index(j, k, i, key);
                        }
                    }
                }
                palette
            }
            Biomes::Cluster(clusters) => {
                let chunk_y = chunk_y.clamp(0, 15) as usize;
                let mut palette = CellPalette::Empty;
                for cluster_x in 0..4 {
                    for cluster_y in 0..4 {
                        for cluster_z in 0..4 {
                            let old_index = chunk_y << 6 | cluster_y << 4 | cluster_z << 2 | cluster_x;
                            palette.set(cluster_x, cluster_y, cluster_z, clusters[old_index].clone());
                        }
                    }
                }
                palette
            }
            Biomes::Palette(chunks) => {
                let Some(last) = chunks.last() else { return CellPalette::Empty };
                let old = usize::try_from(chunk_y).ok().and_then(|i| chunks.get(i)).unwrap_or(last);
                if old.is_empty() {
                    CellPalette::Empty
                } else if old.key_count() == 1 {
                    CellPalette::Single(old.keys()[0].clone())
                } else {
                    let mut palette = CellPalette::Empty;
                    for cluster_x in 0..4 {
                        for cluster_y in 0..4 {
                            for cluster_z in 0..4 {
                                if let Some(biome) =
                                    old.get((cluster_x << 2) + 2, (cluster_y << 2) + 2, (cluster_z << 2) + 2)
                                {
                                    palette.set(cluster_x, cluster_y, cluster_z, biome.clone());
                                }
                            }
                        }
                    }
                    palette
                }
            }
            Biomes::ClusterPalette(chunks) => {
                let Some(last) = chunks.last() else { return CellPalette::Empty };
                usize::try_from(chunk_y).ok().and_then(|i| chunks.get(i)).unwrap_or(last).clone()
            }
        }
    }

    /// Replace every biome (`remap`), e.g. with the writer's supported fallbacks.
    pub fn remap(&mut self, mut f: impl FnMut(&Biome) -> Biome) {
        match self {
            Biomes::Column(v) | Biomes::Cluster(v) => v.iter_mut().for_each(|b| *b = f(b)),
            Biomes::Palette(v) => {
                *v = std::mem::take(v).into_iter().map(|p| p.map(|b| f(&b))).collect();
            }
            Biomes::ClusterPalette(v) => {
                *v = std::mem::take(v).into_iter().map(|p| p.map(|b| f(&b))).collect();
            }
        }
    }
}

/// Expand a 4 cube palette to a 16 cube palette by replicating each cell (`ClusterPalette.asPalette`).
fn upsample_cells(old: &CellPalette) -> BiomePalette {
    if old.is_empty() {
        return BiomePalette::Empty;
    }
    if old.key_count() == 1 {
        return BiomePalette::Single(old.keys()[0].clone());
    }
    let mut palette = BiomePalette::Empty;
    palette.make_writeable();
    for cluster_x in 0..4 {
        for cluster_y in 0..4 {
            for cluster_z in 0..4 {
                // A cell outside the keys has no biome; skip it like a null would be.
                let Some(biome) = old.get(cluster_x, cluster_y, cluster_z) else { continue };
                let key = palette.get_or_create_key(biome.clone());
                for lx in 0..4 {
                    for ly in 0..4 {
                        for lz in 0..4 {
                            palette.set_index(
                                (cluster_x << 2) + lx,
                                (cluster_y << 2) + ly,
                                (cluster_z << 2) + lz,
                                key,
                            );
                        }
                    }
                }
            }
        }
    }
    palette
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: VanillaBiome) -> Biome {
        Biome::Vanilla(v)
    }

    fn fallback() -> Biome {
        b(VanillaBiome::PLAINS)
    }

    #[test]
    fn vanilla_biome_lookup() {
        assert_eq!(VanillaBiome::find("minecraft:plains"), Some(VanillaBiome::PLAINS));
        // Bedrock alias.
        assert_eq!(VanillaBiome::find("minecraft:ice_plains"), Some(VanillaBiome::SNOWY_PLAINS));
        assert_eq!(VanillaBiome::find("minecraft:snowy_plains"), Some(VanillaBiome::SNOWY_PLAINS));
        assert_eq!(VanillaBiome::find("minecraft:nope"), None);
        assert_eq!(VanillaBiome::SNOWY_MOUNTAINS.fallback(), Some(VanillaBiome::SNOWY_PLAINS));
        assert_eq!(VanillaBiome::PLAINS.fallback(), None);
        // Deprecated biomes without a Java name.
        assert_eq!(VanillaBiome::LEGACY_FROZEN_OCEAN.java_identifier(), None);
        assert_eq!(VanillaBiome::LEGACY_FROZEN_OCEAN.bedrock_identifier(), Some("minecraft:legacy_frozen_ocean"));
        assert_eq!(Biome::from_identifier("mod:thing"), Biome::Custom(Arc::from("mod:thing")));
        assert_eq!(Biome::from_identifier("minecraft:river"), b(VanillaBiome::RIVER));
        assert_eq!(Biome::Custom(Arc::from("x")).fallback(), None);
    }

    #[test]
    fn identifiers_resolve_to_a_biome_with_that_identifier() {
        for (id, biome) in BIOME_IDS.iter() {
            let info = &BIOME_INFO[*biome as usize];
            assert!(info.java == Some(id) || info.bedrock == Some(id), "{id}");
        }
        assert!(BIOME_IDS.windows(2).all(|w| w[0].0 < w[1].0));
    }

    fn column_biomes() -> Vec<Biome> {
        // Column index z << 4 | x: left half (x < 8) plains, right half desert, one swamp cell at (2, 2).
        let mut v = Vec::new();
        for _z in 0..16 {
            for x in 0..16 {
                v.push(b(if x < 8 { VanillaBiome::PLAINS } else { VanillaBiome::DESERT }));
            }
        }
        v[2 << 4 | 2] = b(VanillaBiome::SWAMP);
        v
    }

    #[test]
    fn column_layout() {
        let biomes = Biomes::Column(column_biomes());
        assert_eq!(biomes.as_column(&fallback()), column_biomes());
        let cells = biomes.as_4x4(&fallback());
        assert_eq!(cells.len(), 1024);
        // Cell (z4 0, x4 0) samples (2, 2) = swamp; (0, 3) samples x = 14 = desert; replicated on every layer.
        assert_eq!(cells[0], b(VanillaBiome::SWAMP));
        assert_eq!(cells[3], b(VanillaBiome::DESERT));
        assert_eq!(cells[63 << 4], b(VanillaBiome::SWAMP));
        assert_eq!(cells[1023], b(VanillaBiome::DESERT));
        let palettes = biomes.as_palettes();
        assert_eq!(palettes.len(), 1);
        assert_eq!(palettes[0].get(2, 9, 2), Some(&b(VanillaBiome::SWAMP)));
        assert_eq!(palettes[0].get(12, 0, 5), Some(&b(VanillaBiome::DESERT)));
        let p4 = biomes.as_4x4_palette(0);
        assert_eq!(p4.get(0, 3, 0), Some(&b(VanillaBiome::SWAMP)));
        assert_eq!(p4.get(3, 0, 3), Some(&b(VanillaBiome::DESERT)));
        assert_eq!(p4.get(0, 0, 3), Some(&b(VanillaBiome::PLAINS)));
    }

    #[test]
    fn cluster_layout() {
        let mut cells = vec![b(VanillaBiome::PLAINS); 1024];
        cells[0] = b(VanillaBiome::DESERT); // chunk 0, y4 0, z4 0, x4 0
        cells[1 << 6 | 2 << 4 | 3 << 2 | 1] = b(VanillaBiome::SWAMP); // chunk 1, y4 2, z4 3, x4 1
        let biomes = Biomes::Cluster(cells.clone());
        let column = biomes.as_column(&fallback());
        // Cell (z4 0, x4 0) covers x 0..4, z 0..4.
        assert_eq!(column[0], b(VanillaBiome::DESERT));
        assert_eq!(column[3 << 4 | 3], b(VanillaBiome::DESERT));
        assert_eq!(column[4], b(VanillaBiome::PLAINS));
        assert_eq!(biomes.as_4x4(&fallback()), cells);
        let palettes = biomes.as_palettes();
        assert_eq!(palettes.len(), 16);
        assert_eq!(palettes[0].get(0, 0, 0), Some(&b(VanillaBiome::DESERT)));
        assert_eq!(palettes[0].get(3, 3, 3), Some(&b(VanillaBiome::DESERT)));
        assert_eq!(palettes[1].get(4, 8, 12), Some(&b(VanillaBiome::SWAMP)));
        assert_eq!(palettes[1].get(7, 11, 15), Some(&b(VanillaBiome::SWAMP)));
        assert_eq!(palettes[1].get(8, 8, 12), Some(&b(VanillaBiome::PLAINS)));
        let p4 = biomes.as_4x4_palette(1);
        assert_eq!(p4.get(1, 2, 3), Some(&b(VanillaBiome::SWAMP)));
        // Out of range sections clamp.
        assert_eq!(biomes.as_4x4_palette(-5), biomes.as_4x4_palette(0));
        assert_eq!(biomes.as_4x4_palette(99), biomes.as_4x4_palette(15));
    }

    #[test]
    fn palette_layout() {
        let mut p0 = BiomePalette::single(b(VanillaBiome::PLAINS));
        p0.set(2, 2, 2, b(VanillaBiome::SWAMP)); // centre of cell (0, 0, 0)
        let p1 = BiomePalette::single(b(VanillaBiome::DESERT));
        let biomes = Biomes::Palette(vec![p0.clone(), p1.clone()]);
        let column = biomes.as_column(&fallback());
        assert_eq!(column[0], b(VanillaBiome::PLAINS)); // (0, 0, 0)
        assert_eq!(column[2 << 4 | 2], b(VanillaBiome::PLAINS)); // y = 0 only: the swamp voxel is at y 2
        let cells = biomes.as_4x4(&fallback());
        assert_eq!(cells[0], b(VanillaBiome::SWAMP)); // centre sampling hits it
        assert_eq!(cells[1], b(VanillaBiome::PLAINS));
        assert_eq!(cells[1 << 6], b(VanillaBiome::DESERT));
        // Sections past the list repeat the last palette.
        assert_eq!(cells[5 << 6 | 7], b(VanillaBiome::DESERT));
        assert_eq!(cells[15 << 6 | 63], b(VanillaBiome::DESERT));
        assert_eq!(biomes.as_palettes(), vec![p0.clone(), p1]);
        let p4 = biomes.as_4x4_palette(0);
        assert_eq!(p4.get(0, 0, 0), Some(&b(VanillaBiome::SWAMP)));
        assert_eq!(p4.get(1, 0, 0), Some(&b(VanillaBiome::PLAINS)));
        // One key: stays single.
        assert_eq!(biomes.as_4x4_palette(1), CellPalette::Single(b(VanillaBiome::DESERT)));
        assert_eq!(biomes.as_4x4_palette(-1), CellPalette::Single(b(VanillaBiome::DESERT)));
        // No sections: fallback everywhere and an empty palette.
        let none = Biomes::Palette(Vec::new());
        assert!(none.as_column(&fallback()).iter().all(|x| *x == fallback()));
        assert!(none.as_4x4(&b(VanillaBiome::RIVER)).iter().all(|x| *x == b(VanillaBiome::RIVER)));
        assert_eq!(none.as_4x4_palette(0), CellPalette::Empty);
    }

    #[test]
    fn cluster_palette_layout() {
        let mut cells = CellPalette::single(b(VanillaBiome::PLAINS));
        cells.set(1, 2, 3, b(VanillaBiome::SWAMP));
        let biomes = Biomes::ClusterPalette(vec![cells.clone(), CellPalette::single(b(VanillaBiome::RIVER))]);
        let column = biomes.as_column(&fallback());
        // y = 0 layer of cell (x4 1, z4 3) is plains; the swamp cell is at y4 2.
        assert_eq!(column[12 << 4 | 4], b(VanillaBiome::PLAINS));
        let cell_vec = biomes.as_4x4(&fallback());
        assert_eq!(cell_vec[2 << 4 | 3 << 2 | 1], b(VanillaBiome::SWAMP));
        assert_eq!(cell_vec[1 << 6], b(VanillaBiome::RIVER));
        assert_eq!(cell_vec[9 << 6], b(VanillaBiome::RIVER)); // last palette repeats
        let palettes = biomes.as_palettes();
        assert_eq!(palettes.len(), 2);
        assert_eq!(palettes[0].get(4, 8, 12), Some(&b(VanillaBiome::SWAMP)));
        assert_eq!(palettes[0].get(7, 11, 15), Some(&b(VanillaBiome::SWAMP)));
        assert_eq!(palettes[0].get(8, 8, 12), Some(&b(VanillaBiome::PLAINS)));
        assert_eq!(palettes[1], BiomePalette::Single(b(VanillaBiome::RIVER)));
        assert_eq!(biomes.as_4x4_palette(0), cells);
        assert_eq!(biomes.as_4x4_palette(7), CellPalette::single(b(VanillaBiome::RIVER)));
        assert_eq!(Biomes::ClusterPalette(vec![CellPalette::Empty]).as_palettes(), vec![BiomePalette::Empty]);
    }

    #[test]
    fn remap_replaces_biomes() {
        let mut column = Biomes::Column(column_biomes());
        column.remap(|x| if *x == b(VanillaBiome::SWAMP) { b(VanillaBiome::RIVER) } else { x.clone() });
        let Biomes::Column(v) = &column else { unreachable!() };
        assert_eq!(v[2 << 4 | 2], b(VanillaBiome::RIVER));
        let mut pal = Biomes::Palette(vec![BiomePalette::single(b(VanillaBiome::SWAMP))]);
        pal.remap(|_| b(VanillaBiome::BEACH));
        assert_eq!(pal, Biomes::Palette(vec![BiomePalette::single(b(VanillaBiome::BEACH))]));
        let mut cp = Biomes::ClusterPalette(vec![CellPalette::single(b(VanillaBiome::SWAMP)), CellPalette::Empty]);
        cp.remap(|_| b(VanillaBiome::BEACH));
        assert_eq!(cp, Biomes::ClusterPalette(vec![CellPalette::single(b(VanillaBiome::BEACH)), CellPalette::Empty]));
    }
}
