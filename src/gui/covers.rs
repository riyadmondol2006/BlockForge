//! Covers: a picture for every instance, drawn rather than chosen. Each one is a small Minecraft landscape in pixel
//! art, made from the instance's seed, so two instances rarely look alike and the same instance always looks the
//! same. The scene is painted at a low resolution (one block is four pixels) and blown up with whole-number
//! nearest-neighbour scaling, so the pixels read as blocks at any size.
//!
//! The picture is built in layers, back to front: a sky with its sun or moon, clouds or stars; two hazy ranges of
//! distant hills (sometimes with a house or a temple on them); the near terrain as a grid of blocks from a seeded
//! heightmap, with water or lava, trees and small plants; then light (glowing blocks), a soft vignette and, when
//! asked, a dark fade on the left so white text stays readable there. The mod loader shows as a small banner and a
//! few ores in its colour.

use std::collections::HashMap;

use egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions};
use serde::{Deserialize, Serialize};

/// The kind of landscape a cover shows. `Auto` lets the seed choose, so instances differ without anyone picking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverTheme {
    #[default]
    Auto,
    Meadow,
    Forest,
    Cherry,
    Sunset,
    Night,
    Snow,
    Desert,
    Ocean,
    Nether,
    End,
}

impl CoverTheme {
    pub const ALL: [CoverTheme; 11] = [
        CoverTheme::Auto,
        CoverTheme::Meadow,
        CoverTheme::Forest,
        CoverTheme::Cherry,
        CoverTheme::Sunset,
        CoverTheme::Night,
        CoverTheme::Snow,
        CoverTheme::Desert,
        CoverTheme::Ocean,
        CoverTheme::Nether,
        CoverTheme::End,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CoverTheme::Auto => "Automatic",
            CoverTheme::Meadow => "Meadow",
            CoverTheme::Forest => "Forest",
            CoverTheme::Cherry => "Cherry grove",
            CoverTheme::Sunset => "Sunset",
            CoverTheme::Night => "Night",
            CoverTheme::Snow => "Snow",
            CoverTheme::Desert => "Desert",
            CoverTheme::Ocean => "Ocean",
            CoverTheme::Nether => "Nether",
            CoverTheme::End => "The End",
        }
    }

    /// A stable name for saving (the same as the serde name).
    pub fn id(self) -> &'static str {
        match self {
            CoverTheme::Auto => "auto",
            CoverTheme::Meadow => "meadow",
            CoverTheme::Forest => "forest",
            CoverTheme::Cherry => "cherry",
            CoverTheme::Sunset => "sunset",
            CoverTheme::Night => "night",
            CoverTheme::Snow => "snow",
            CoverTheme::Desert => "desert",
            CoverTheme::Ocean => "ocean",
            CoverTheme::Nether => "nether",
            CoverTheme::End => "end",
        }
    }

    pub fn from_id(id: &str) -> Option<CoverTheme> {
        CoverTheme::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The theme that is actually drawn: `Auto` becomes one picked by the seed, mostly daytime overworld (a cherry
    /// grove only for versions that have them).
    pub fn resolve(self, seed: u64, mc_version: &str) -> CoverTheme {
        if self != CoverTheme::Auto {
            return self;
        }
        let cherry = if era(mc_version) == Era::Modern && has_cherry(mc_version) { 8 } else { 0 };
        let weights = [
            (CoverTheme::Meadow, 24),
            (CoverTheme::Forest, 14),
            (CoverTheme::Cherry, cherry),
            (CoverTheme::Sunset, 12),
            (CoverTheme::Night, 6),
            (CoverTheme::Snow, 10),
            (CoverTheme::Desert, 10),
            (CoverTheme::Ocean, 10),
            (CoverTheme::Nether, 4),
            (CoverTheme::End, 2),
        ];
        let total: u64 = weights.iter().map(|w| w.1).sum();
        let mut pick = splitmix(seed ^ 0x7E3A_11C0) % total;
        for (theme, w) in weights {
            if pick < w {
                return theme;
            }
            pick -= w;
        }
        CoverTheme::Meadow
    }
}

/// What a cover is drawn from. Everything in here goes into the picture, so two equal specs give equal pictures.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CoverSpec {
    /// From the instance's folder name or id (see [`seed_from`]).
    pub seed: u64,
    pub mc_version: String,
    /// "Fabric", "Forge", "NeoForge", "Quilt", or `None` for vanilla.
    pub loader: Option<String>,
    pub theme: CoverTheme,
    /// Darken the left side softly, for a cover with text over it.
    pub fade_left: bool,
}

impl CoverSpec {
    pub fn new(key: &str, mc_version: &str, loader: Option<&str>, theme: CoverTheme) -> CoverSpec {
        CoverSpec {
            seed: seed_from(key),
            mc_version: mc_version.to_string(),
            loader: loader.map(str::to_string),
            theme,
            fade_left: false,
        }
    }

    pub fn with_fade_left(mut self, fade: bool) -> CoverSpec {
        self.fade_left = fade;
        self
    }
}

/// A seed from text (an instance folder name or uuid). FNV-1a, so it stays the same across Rust versions and runs,
/// unlike the standard hasher.
pub fn seed_from(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    splitmix(h)
}

/// The cover textures made so far, keyed by what they show and their size, so each is drawn once.
#[derive(Default)]
pub struct Covers {
    textures: HashMap<(CoverSpec, usize, usize), TextureHandle>,
}

impl Covers {
    /// The cover for `spec` at `w` × `h` pixels, drawn the first time it is asked for. Loaded with nearest filtering
    /// so the blocks stay crisp.
    pub fn texture(&mut self, ctx: &Context, spec: &CoverSpec, w: usize, h: usize) -> TextureHandle {
        let key = (spec.clone(), w, h);
        if let Some(t) = self.textures.get(&key) {
            return t.clone();
        }
        let name = format!("cover-{:016x}-{}-{w}x{h}", spec.seed, spec.theme.id());
        let tex = ctx.load_texture(name, render(spec, w, h), TextureOptions::NEAREST);
        self.textures.insert(key, tex.clone());
        tex
    }
}

/// Draw the cover for `spec` at exactly `w` × `h` pixels. The same spec and size always give the same pixels.
pub fn render(spec: &CoverSpec, w: usize, h: usize) -> ColorImage {
    let (w, h) = (w.max(1), h.max(1));
    // the whole number each scene pixel is blown up by: about 160 × 90 scene pixels for a wide cover, and more of
    // the landscape (about 240 across) on a banner, which is shown far wider than a tile
    let banner = w > h * 2;
    let scale = if banner { w.div_ceil(240).max(h.div_ceil(100)) } else { w.div_ceil(160).max(h.div_ceil(90)) }.max(1);
    let (iw, ih) = (w.div_ceil(scale), h.div_ceil(scale));
    let mut scene = Scene::new(spec, iw, ih);
    scene.paint();

    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &scene.img[(y / scale) * iw..(y / scale + 1) * iw];
        for x in 0..w {
            pixels.push(row[x / scale].to_color());
        }
    }
    ColorImage::new([w, h], pixels)
}

// --- colours and randomness ----------------------------------------------------------------------------------------

/// A colour while painting (0 to 1 per channel), so blending and shading don't lose precision.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgb {
    r: f32,
    g: f32,
    b: f32,
}

const fn hex(v: u32) -> Rgb {
    Rgb { r: ((v >> 16) & 0xff) as f32 / 255.0, g: ((v >> 8) & 0xff) as f32 / 255.0, b: (v & 0xff) as f32 / 255.0 }
}

impl Rgb {
    fn mix(self, o: Rgb, t: f32) -> Rgb {
        Rgb { r: self.r + (o.r - self.r) * t, g: self.g + (o.g - self.g) * t, b: self.b + (o.b - self.b) * t }
    }
    fn scale(self, k: f32) -> Rgb {
        Rgb { r: self.r * k, g: self.g * k, b: self.b * k }
    }
    fn mul(self, o: Rgb) -> Rgb {
        Rgb { r: self.r * o.r, g: self.g * o.g, b: self.b * o.b }
    }
    fn add(self, o: Rgb) -> Rgb {
        Rgb { r: self.r + o.r, g: self.g + o.g, b: self.b + o.b }
    }
    fn to_color(self) -> Color32 {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        Color32::from_rgb(c(self.r), c(self.g), c(self.b))
    }
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A random number for a spot: the same seed and coordinates always give the same value.
fn hash(seed: u64, a: i32, b: i32) -> u64 {
    splitmix(
        seed ^ (a as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (b as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F),
    )
}

/// 0 to 1.
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Smooth value noise along a line, 0 to 1.
fn noise1(seed: u64, x: f32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let a = unit(hash(seed, i as i32, 7));
    let b = unit(hash(seed, i as i32 + 1, 7));
    let t = f * f * (3.0 - 2.0 * f);
    a + (b - a) * t
}

/// A few octaves of [`noise1`], about -1 to 1: big rolling shapes with a little roughness on top.
fn fbm(seed: u64, x: f32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for o in 0..3 {
        sum += (noise1(seed.wrapping_add(o * 977), x * freq) * 2.0 - 1.0) * amp;
        norm += amp;
        amp *= 0.45;
        freq *= 2.1;
    }
    sum / norm
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A simple sequence of random numbers for one-off choices (where the trees go, how many clouds).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        splitmix(self.0)
    }
    fn f(&mut self) -> f32 {
        unit(self.next())
    }
    /// `lo` to `hi`, both included.
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next() % (hi - lo + 1) as u64) as i32
    }
    fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }
    fn between(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
}

// --- versions ------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
    /// Alpha, beta and the early tests: the bright old grass.
    Classic,
    /// 1.0 to 1.8: the older, yellower grass.
    Old,
    Modern,
}

/// The minor number of a release ("1.20.4" is 20), if the version looks like one.
fn minor(v: &str) -> Option<u32> {
    let rest = v.strip_prefix("1.")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn era(v: &str) -> Era {
    let v = v.trim();
    let classic = ["a1.", "b1.", "c0.", "inf-", "rd-", "in-"];
    if classic.iter().any(|p| v.starts_with(p)) {
        return Era::Classic;
    }
    match minor(v) {
        Some(m) if m <= 8 => Era::Old,
        _ => Era::Modern,
    }
}

/// Deepslate arrived in 1.17; older worlds are stone all the way down.
fn has_deepslate(v: &str) -> bool {
    era(v) == Era::Modern && minor(v).is_none_or(|m| m >= 17)
}

fn has_cherry(v: &str) -> bool {
    minor(v).is_none_or(|m| m >= 20)
}

fn loader_rgb(loader: Option<&str>) -> Option<Rgb> {
    match loader? {
        "Fabric" => Some(hex(0xD9B58A)),
        "Forge" => Some(hex(0x7FA8E8)),
        "NeoForge" => Some(hex(0xF09A4F)),
        "Quilt" => Some(hex(0xB98BE0)),
        _ => None,
    }
}

// --- the look of each theme ----------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ground {
    Grass,
    Snow,
    Sand,
    Island,
    Nether,
    End,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wood {
    Oak,
    Birch,
    Spruce,
    Cherry,
    Crimson,
    Warped,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Celestial {
    None,
    Sun,
    Moon,
}

/// Everything a theme decides about colour and content.
struct Look {
    sky: [Rgb; 3],
    celestial: Celestial,
    /// Where the sun or moon sits, as fractions of the picture.
    sun_at: (f32, f32),
    sun_size: f32,
    clouds: Option<Rgb>,
    cloud_count: f32,
    stars: f32,
    /// The far and the nearer range of distant hills.
    far: [Rgb; 2],
    /// Light on the near terrain (multiplied in).
    light: Rgb,
    ground: Ground,
    grass: Rgb,
    trees: &'static [Wood],
    /// Trees per column, roughly.
    tree_density: f32,
    /// Water (or lava) level below the ground line, in terrain heights (0.5: only the deeper dips fill); `None` for
    /// no lakes.
    sea: Option<f32>,
    water: Rgb,
    snowfall: bool,
    /// A building on the nearer hills now and then.
    structure: Option<&'static [&'static str]>,
    /// Lit windows (at night).
    lit: bool,
}

const HOUSE: &[&str] = &[
    "........#....",
    "....#####....",
    "...#######...",
    "..#########..",
    "...#######...",
    "...#ww##d#...",
    "...#ww##d#...",
    "...#######...",
];

const TOWER: &[&str] =
    &["#.#.#.#", "#######", ".#####.", ".##w##.", ".#####.", ".#####.", ".##w##.", ".#####.", ".#####."];

const TEMPLE: &[&str] =
    &[".##.......##.", ".##...#...##.", "####.###.####", "#############", "#############", "######d######"];

const IGLOO: &[&str] = &["...###...", ".#######.", "#########", "####d####"];

fn look_for(theme: CoverTheme, spec: &CoverSpec, rng: &mut Rng) -> Look {
    let old = era(&spec.mc_version);
    let grass = match old {
        Era::Classic => hex(0x6FBF3E),
        Era::Old => hex(0x86B84E),
        Era::Modern => hex(0x7DB84A),
    };
    let sun_x = rng.between(0.6, 0.86);
    let sun_y = rng.between(0.12, 0.26);
    let day = Look {
        sky: [hex(0x5E9BEF), hex(0x8DBBF5), hex(0xC6DDF7)],
        celestial: Celestial::Sun,
        sun_at: (sun_x, sun_y),
        sun_size: 1.0,
        clouds: Some(hex(0xFFFFFF)),
        cloud_count: 1.0,
        stars: 0.0,
        far: [hex(0xA3C3DC), hex(0x6E9C78)],
        light: hex(0xFFFFFF),
        ground: Ground::Grass,
        grass,
        trees: &[Wood::Oak, Wood::Oak, Wood::Birch],
        tree_density: 0.07,
        sea: Some(0.5),
        water: hex(0x3D6FD4),
        snowfall: false,
        structure: if rng.chance(0.5) { Some(HOUSE) } else { None },
        lit: false,
    };
    match theme {
        CoverTheme::Auto | CoverTheme::Meadow => day,
        CoverTheme::Forest => Look {
            sky: [hex(0x6A9FE0), hex(0x98BFE8), hex(0xC9DCEB)],
            far: [hex(0x93B5C9), hex(0x4F7D5C)],
            grass: grass.mix(hex(0x4F9A3A), 0.4),
            trees: &[Wood::Oak, Wood::Birch, Wood::Oak, Wood::Spruce],
            tree_density: 0.16,
            cloud_count: 0.7,
            structure: if rng.chance(0.4) { Some(TOWER) } else { None },
            ..day
        },
        CoverTheme::Cherry => Look {
            sky: [hex(0x7AA8F0), hex(0xA8C6F4), hex(0xE6D8EE)],
            far: [hex(0xC2B9D9), hex(0xC98DB4)],
            grass: hex(0x8CC25A),
            trees: &[Wood::Cherry],
            tree_density: 0.09,
            structure: None,
            ..day
        },
        CoverTheme::Sunset => Look {
            sky: [hex(0x3B3F7A), hex(0xC0607A), hex(0xF5B26B)],
            sun_at: (sun_x, rng.between(0.24, 0.3)),
            sun_size: 1.4,
            clouds: Some(hex(0xF3A08C)),
            cloud_count: 0.8,
            far: [hex(0xA86A86), hex(0x5E3F63)],
            light: hex(0xE8B89A),
            water: hex(0x5A6FC0),
            ..day
        },
        CoverTheme::Night => Look {
            sky: [hex(0x0B1026), hex(0x18234A), hex(0x2B3A66)],
            celestial: Celestial::Moon,
            clouds: Some(hex(0x3A4570)),
            cloud_count: 0.5,
            stars: 1.0,
            far: [hex(0x273358), hex(0x1C2744)],
            light: hex(0x7383B5),
            water: hex(0x2A3F8A),
            structure: Some(if rng.chance(0.6) { HOUSE } else { TOWER }),
            lit: true,
            ..day
        },
        CoverTheme::Snow => Look {
            sky: [hex(0x7FA7D6), hex(0xB0CBE8), hex(0xE2ECF6)],
            clouds: Some(hex(0xF4F7FB)),
            far: [hex(0xC9D6E6), hex(0x9FB3C9)],
            light: hex(0xF4F8FF),
            ground: Ground::Snow,
            grass: hex(0x6FA05A),
            trees: &[Wood::Spruce],
            tree_density: 0.1,
            water: hex(0x3A6AD0),
            snowfall: true,
            structure: if rng.chance(0.5) { Some(IGLOO) } else { None },
            ..day
        },
        CoverTheme::Desert => Look {
            sky: [hex(0x5C9AE6), hex(0x9BC2EC), hex(0xEADBB8)],
            clouds: Some(hex(0xFFFCF4)),
            cloud_count: 0.4,
            far: [hex(0xD9C6A0), hex(0xCDAE76)],
            light: hex(0xFFF6EA),
            ground: Ground::Sand,
            trees: &[],
            tree_density: 0.0,
            sea: None,
            structure: if rng.chance(0.6) { Some(TEMPLE) } else { None },
            ..day
        },
        CoverTheme::Ocean => Look {
            sky: [hex(0x4F8FEA), hex(0x86B6F2), hex(0xCDE3F8)],
            far: [hex(0x8FB0CF), hex(0x6E9A7C)],
            ground: Ground::Island,
            trees: &[Wood::Oak],
            tree_density: 0.15,
            sea: Some(0.0),
            structure: None,
            ..day
        },
        CoverTheme::Nether => Look {
            sky: [hex(0x180505), hex(0x300B09), hex(0x52160F)],
            celestial: Celestial::None,
            clouds: None,
            far: [hex(0x43120F), hex(0x5A1A15)],
            light: hex(0xF2C8B4),
            ground: Ground::Nether,
            trees: if rng.chance(0.55) { &[Wood::Crimson] } else { &[Wood::Warped] },
            tree_density: 0.08,
            sea: Some(0.1),
            water: hex(0xE8641A),
            structure: None,
            ..day
        },
        CoverTheme::End => Look {
            sky: [hex(0x07040C), hex(0x150D22), hex(0x2B1B3D)],
            celestial: Celestial::None,
            clouds: None,
            stars: 0.6,
            far: [hex(0x2E2342), hex(0x3A2D4F)],
            light: hex(0xC8BCD8),
            ground: Ground::End,
            trees: &[],
            tree_density: 0.0,
            sea: None,
            structure: None,
            ..day
        },
    }
}

// --- blocks --------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Block {
    Air,
    Grass,
    SnowyGrass,
    Dirt,
    Stone,
    Deepslate,
    Gravel,
    Sand,
    Sandstone,
    Ice,
    Water,
    Lava,
    Coal,
    /// An ore in the loader's colour.
    LoaderOre,
    Log(Wood),
    Leaves(Wood),
    /// Leaves with snow on top.
    SnowyLeaves,
    Cactus,
    Netherrack,
    Nylium(bool),
    Wart(bool),
    Shroomlight,
    Glowstone,
    EndStone,
    Obsidian,
    Crystal,
    Chorus,
    Coral(u8),
}

impl Block {
    /// Light and water pass this, so it doesn't darken what's around it.
    fn open(self) -> bool {
        matches!(self, Block::Air | Block::Water | Block::Lava | Block::Leaves(_) | Block::SnowyLeaves | Block::Chorus)
    }
    /// Part of the land itself (not a tree, plant, liquid or air).
    fn ground(self) -> bool {
        !self.open()
            && !matches!(
                self,
                Block::Log(_)
                    | Block::Cactus
                    | Block::Wart(_)
                    | Block::Shroomlight
                    | Block::Coral(_)
                    | Block::Obsidian
                    | Block::Crystal
            )
    }
    fn emissive(self) -> bool {
        matches!(self, Block::Lava | Block::Glowstone | Block::Shroomlight | Block::Crystal)
    }
}

/// Block size in scene pixels.
const B: i32 = 4;

/// Speck pattern of an ore (one block).
const ORE: [[bool; 4]; 4] =
    [[false, true, false, false], [true, true, false, false], [false, false, true, true], [false, false, true, false]];

struct Light {
    x: f32,
    y: f32,
    color: Rgb,
    radius: f32,
    strength: f32,
}

struct Scene<'a> {
    spec: &'a CoverSpec,
    theme: CoverTheme,
    look: Look,
    seed: u64,
    iw: i32,
    ih: i32,
    cols: i32,
    rows: i32,
    /// Wide enough that the left side is kept calm for text.
    wide: bool,
    img: Vec<Rgb>,
    grid: Vec<Block>,
    /// Where the ground starts in each column (block row), before trees.
    surface: Vec<i32>,
    /// The water (or lava) line, in block rows.
    sea_row: Option<i32>,
    /// Where the distant hills start, in scene pixels.
    horizon: i32,
    lights: Vec<Light>,
    loader: Option<Rgb>,
    rng: Rng,
}

impl<'a> Scene<'a> {
    fn new(spec: &'a CoverSpec, iw: usize, ih: usize) -> Scene<'a> {
        let theme = spec.theme.resolve(spec.seed, &spec.mc_version);
        let seed = splitmix(spec.seed ^ hash(0x5EED, theme as i32, 0));
        let mut rng = Rng(seed);
        let look = look_for(theme, spec, &mut rng);
        let (iw, ih) = (iw as i32, ih as i32);
        let cols = (iw + B - 1) / B;
        let rows = (ih + B - 1) / B;
        Scene {
            spec,
            theme,
            look,
            seed,
            iw,
            ih,
            cols,
            rows,
            wide: iw as f32 > ih as f32 * 1.3,
            img: vec![Rgb { r: 0.0, g: 0.0, b: 0.0 }; (iw * ih) as usize],
            grid: vec![Block::Air; (cols * rows) as usize],
            surface: vec![rows; cols as usize],
            sea_row: None,
            horizon: ih / 2,
            lights: Vec::new(),
            loader: loader_rgb(spec.loader.as_deref()),
            rng,
        }
    }

    fn paint(&mut self) {
        self.build_terrain();
        self.paint_sky();
        self.paint_far();
        self.paint_blocks();
        self.paint_plants();
        self.paint_banner();
        self.paint_weather();
        self.paint_lights();
        self.finish();
    }

    // -- helpers --

    fn at(&self, c: i32, r: i32) -> Block {
        if c < 0 || c >= self.cols || r >= self.rows {
            // beyond the sides the ground carries on; below the picture it is solid
            return if r >= self.rows { Block::Stone } else { self.at(c.clamp(0, self.cols - 1), r) };
        }
        if r < 0 {
            // above the picture: the Nether's ceiling carries on, everywhere else it is sky
            return if self.look.ground == Ground::Nether { Block::Netherrack } else { Block::Air };
        }
        self.grid[(r * self.cols + c) as usize]
    }

    fn set(&mut self, c: i32, r: i32, b: Block) {
        if c >= 0 && c < self.cols && r >= 0 && r < self.rows {
            self.grid[(r * self.cols + c) as usize] = b;
        }
    }

    fn put(&mut self, x: i32, y: i32, c: Rgb) {
        if x >= 0 && x < self.iw && y >= 0 && y < self.ih {
            self.img[(y * self.iw + x) as usize] = c;
        }
    }

    fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x >= 0 && x < self.iw && y >= 0 && y < self.ih {
            let p = &mut self.img[(y * self.iw + x) as usize];
            *p = p.mix(c, a.clamp(0.0, 1.0));
        }
    }

    /// 0 at the left edge to 1 past the first 40 % of a wide cover: how much detail is allowed there.
    fn calm(&self, x_frac: f32) -> f32 {
        if self.wide { smoothstep(0.0, 0.42, x_frac) } else { 1.0 }
    }

    fn lit(&self, c: Rgb) -> Rgb {
        c.mul(self.look.light)
    }

    // -- terrain --

    fn build_terrain(&mut self) {
        let rows = self.rows as f32;
        let ground_frac = match self.look.ground {
            Ground::Island => 0.86,
            Ground::Nether => 0.62,
            Ground::End => 0.62,
            _ => 0.64,
        };
        let ground = rows * ground_frac;
        let rough = self.rng.between(0.7, 1.3);
        let amp = match self.look.ground {
            Ground::Nether => 3.6,
            Ground::Island => 1.2,
            Ground::End => 1.2,
            _ => 2.4,
        } * rough
            * (rows / 22.0).clamp(0.6, 1.2);
        let freq = self.rng.between(0.06, 0.1) * (40.0 / self.cols as f32).clamp(0.8, 2.0);
        let tseed = self.seed ^ 0xA11CE;
        let island_at = self.rng.between(0.62, 0.8);

        for c in 0..self.cols {
            let xf = (c as f32 + 0.5) / self.cols as f32;
            let calm = self.calm(xf);
            let mut h = ground - fbm(tseed, c as f32 * freq) * amp * (0.45 + 0.55 * calm);
            if self.wide {
                // a little lower on the left, so text sits over sky and soft ground
                h += (1.0 - calm) * 1.2;
            }
            if self.look.ground == Ground::Island {
                // one island rising out of the sea
                let d = (xf - island_at) / 0.14;
                h -= rows * 0.4 * (-d * d * d * d).exp();
            }
            let s = (h.round() as i32).min(self.rows - 2).max(self.rows / 3);
            self.surface[c as usize] = s;
        }

        self.sea_row = self.look.sea.map(|off| match self.look.ground {
            Ground::Island => (rows * 0.58).round() as i32,
            _ => (ground + off * amp).round() as i32,
        });
        // a pit one column wide makes a puddle that looks like a hole: fill it in
        if let Some(w) = self.sea_row {
            for c in 0..self.cols as usize {
                let wet = |n: usize| self.surface.get(n).is_some_and(|&s| s > w);
                if wet(c) && !wet(c.wrapping_sub(1)) && !wet(c + 1) {
                    self.surface[c] = w;
                }
            }
        }
        self.horizon = match self.look.ground {
            // the open sea shows as a band above the near water, as if seen from a little above
            Ground::Island => self.sea_row.unwrap_or(0) * B - (self.ih / 9).max(3),
            _ => ((ground - amp - 2.0) * B as f32) as i32,
        };

        match self.look.ground {
            Ground::Nether => self.build_nether(),
            Ground::End => self.build_end(),
            _ => self.build_overworld(),
        }
        self.place_ores();
    }

    fn build_overworld(&mut self) {
        let deepslate = has_deepslate(&self.spec.mc_version);
        let sea = self.sea_row;
        for c in 0..self.cols {
            let s = self.surface[c as usize];
            let wet = sea.is_some_and(|w| s > w);
            // sand along the shore: low ground next to water
            let near_water =
                (c - 2..=c + 2).any(|n| sea.is_some_and(|w| self.surface[n.clamp(0, self.cols - 1) as usize] > w));
            let beach = near_water && sea.is_some_and(|w| s >= w) && self.look.ground != Ground::Snow;
            let deep_at = self.rows - 2 + (fbm(self.seed ^ 0xDEE9, c as f32 * 0.3) * 1.2).round() as i32;
            for r in s..self.rows {
                let d = r - s;
                let b = match self.look.ground {
                    Ground::Sand => match d {
                        0..=2 => Block::Sand,
                        3..=4 => Block::Sandstone,
                        _ => Block::Stone,
                    },
                    _ if wet || beach => match d {
                        0..=1 if self.look.ground == Ground::Island || !wet => Block::Sand,
                        0 if unit(hash(self.seed, c, 91)) < 0.4 => Block::Gravel,
                        0..=1 => Block::Dirt,
                        2 => Block::Dirt,
                        _ => Block::Stone,
                    },
                    Ground::Snow if d == 0 => Block::SnowyGrass,
                    _ if d == 0 => Block::Grass,
                    _ if d <= 3 + (hash(self.seed, c, 5) % 2) as i32 => Block::Dirt,
                    _ => Block::Stone,
                };
                let b = if b == Block::Stone && deepslate && r >= deep_at { Block::Deepslate } else { b };
                self.set(c, r, b);
            }
            if let Some(w) = sea {
                for r in w..s {
                    let frozen = self.look.ground == Ground::Snow && r == w;
                    self.set(c, r, if frozen { Block::Ice } else { Block::Water });
                }
            }
        }
        self.place_trees();
        if self.look.ground == Ground::Sand {
            self.place_cacti();
        }
        if self.look.ground == Ground::Island {
            self.place_coral();
        }
    }

    /// Columns where something tall may stand: dry ground of the right kind, away from the calm left.
    fn dry(&self, c: i32) -> bool {
        let s = self.surface[c as usize];
        let top = self.at(c, s);
        matches!(top, Block::Grass | Block::SnowyGrass | Block::Sand | Block::Nylium(_) | Block::Netherrack)
            && self.at(c, s - 1) == Block::Air
    }

    fn place_trees(&mut self) {
        if self.look.trees.is_empty() {
            return;
        }
        let mut c = self.rng.range(1, 4);
        let mut placed = 0;
        while c < self.cols {
            let xf = (c as f32 + 0.5) / self.cols as f32;
            let p = self.look.tree_density * 6.0 * self.calm(xf).powf(1.5);
            let flat = self.surface[c as usize] == self.surface[(c + 1).min(self.cols - 1) as usize];
            if self.dry(c) && flat && self.rng.chance(p.min(0.9)) && self.at(c, self.surface[c as usize]) != Block::Sand
            {
                let wood = self.look.trees[self.rng.range(0, self.look.trees.len() as i32 - 1) as usize];
                self.tree(c, self.surface[c as usize], wood);
                placed += 1;
                c += self.rng.range(4, 7);
            } else {
                c += 1;
            }
        }
        // a cover of the overworld without a single tree looks bare: plant one on the right
        if placed == 0 && self.look.tree_density > 0.0 {
            let start = (self.cols as f32 * 0.6) as i32;
            if let Some(c) = (start..self.cols - 1).find(|&c| self.dry(c)) {
                let wood = self.look.trees[0];
                self.tree(c, self.surface[c as usize], wood);
            }
        }
    }

    fn tree(&mut self, c: i32, ground: i32, wood: Wood) {
        let snowy = self.look.ground == Ground::Snow;
        let leaves = |s: &mut Scene, c: i32, r: i32, top: bool| {
            if s.at(c, r) == Block::Air {
                s.set(c, r, if snowy && top { Block::SnowyLeaves } else { Block::Leaves(wood) });
            }
        };
        let (trunk, crown): (i32, &[i32]) = match wood {
            Wood::Oak => (self.rng.range(3, 4), &[1, 2, 2, 1]),
            Wood::Birch => (self.rng.range(4, 5), &[1, 2, 2, 1]),
            Wood::Spruce => (self.rng.range(5, 7), &[0, 1, 0, 1, 2, 1, 2]),
            Wood::Cherry => (self.rng.range(3, 4), &[2, 3, 3]),
            Wood::Crimson | Wood::Warped => (self.rng.range(3, 5), &[1, 2]),
        };
        for i in 1..=trunk {
            self.set(c, ground - i, Block::Log(wood));
        }
        let n = crown.len() as i32;
        let top = ground - trunk - 1;
        // oaks, birches and cherries hold their leaves around the top of the trunk; spruces and fungi above it
        let sink = if matches!(wood, Wood::Spruce | Wood::Crimson | Wood::Warped) { 0 } else { 1 };
        for (i, &half) in crown.iter().enumerate() {
            let r = top + i as i32 - sink;
            for dc in -half..=half {
                // round the corners off the widest rows now and then
                let corner = dc.abs() == half && half >= 2 && (i == 0 || i as i32 == n - 1);
                if corner && self.rng.chance(0.7) {
                    continue;
                }
                if matches!(wood, Wood::Crimson | Wood::Warped) {
                    let b = if self.rng.chance(0.12) { Block::Shroomlight } else { Block::Wart(wood == Wood::Warped) };
                    if self.at(c + dc, r) == Block::Air || dc == 0 {
                        self.set(c + dc, r, b);
                        if b == Block::Shroomlight {
                            self.add_light_block(c + dc, r, hex(0xFFB060), 10.0, 0.35);
                        }
                    }
                } else {
                    let exposed = i == 0 || crown[i - 1] < dc.abs();
                    leaves(self, c + dc, r, exposed);
                }
            }
        }
        if wood == Wood::Spruce {
            leaves(self, c, top - 1, true);
        }
    }

    fn place_cacti(&mut self) {
        let mut c = self.rng.range(2, 5);
        while c < self.cols {
            let xf = (c as f32 + 0.5) / self.cols as f32;
            if self.dry(c) && self.rng.chance(0.35 * self.calm(xf)) {
                let s = self.surface[c as usize];
                for i in 1..=self.rng.range(2, 3) {
                    self.set(c, s - i, Block::Cactus);
                }
                c += self.rng.range(5, 9);
            } else {
                c += 1;
            }
        }
    }

    fn place_coral(&mut self) {
        let Some(w) = self.sea_row else { return };
        for c in 0..self.cols {
            let s = self.surface[c as usize];
            if s > w + 2 && self.rng.chance(0.12) {
                let kind = self.rng.range(0, 2) as u8;
                self.set(c, s - 1, Block::Coral(kind));
            }
        }
    }

    fn build_nether(&mut self) {
        let sea = self.sea_row.unwrap_or(self.rows);
        let warped = self.look.trees.first() == Some(&Wood::Warped);
        // the ceiling: a ragged band of netherrack across the top, with glowstone hanging under it
        let ceil_seed = self.seed ^ 0xCE11;
        let mut ceiling = vec![0; self.cols as usize];
        for c in 0..self.cols {
            let xf = (c as f32 + 0.5) / self.cols as f32;
            let base = 1.4 + fbm(ceil_seed, c as f32 * 0.22) * 1.6 * (0.4 + 0.6 * self.calm(xf));
            let drip = if unit(hash(ceil_seed, c, 3)) < 0.15 * self.calm(xf) { 1 } else { 0 };
            let bottom = (base.round() as i32 + drip).max(0);
            ceiling[c as usize] = bottom;
            for r in 0..bottom {
                self.set(c, r, Block::Netherrack);
            }
        }
        let clusters = self.rng.range(1, 3);
        for _ in 0..clusters {
            let c = self.rng.range(self.cols * 2 / 5, self.cols - 2);
            let bottom = ceiling[c as usize];
            let w = self.rng.range(1, 2);
            for dc in 0..=w {
                let len = if dc == 0 || dc == w { 1 } else { 2 };
                let cc = c + dc - w / 2;
                if cc < 0 || cc >= self.cols {
                    continue;
                }
                let b0 = ceiling[cc as usize];
                for i in 0..len {
                    self.set(cc, b0 + i, Block::Glowstone);
                }
            }
            self.add_light_block(c, bottom, hex(0xFFD98A), 16.0, 0.25);
        }
        // the ground: netherrack, a top of nylium here and there, a lava sea in the dips
        for c in 0..self.cols {
            let s = self.surface[c as usize];
            let nylium = unit(hash(self.seed, c, 77)) < 0.75;
            for r in s..self.rows {
                let b = if r == s && s < sea && nylium { Block::Nylium(warped) } else { Block::Netherrack };
                self.set(c, r, b);
            }
            for r in sea..s {
                self.set(c, r, Block::Lava);
            }
        }
        // one fall of lava from the ceiling, over the sea if there is one
        let falls: Vec<i32> = (self.cols / 2..self.cols - 1).filter(|&c| self.surface[c as usize] > sea).collect();
        if !falls.is_empty() && self.rng.chance(0.75) {
            let c = falls[self.rng.range(0, falls.len() as i32 - 1) as usize];
            for r in ceiling[c as usize]..sea {
                self.set(c, r, Block::Lava);
            }
        }
        // a glow over the lava sea
        for c in (0..self.cols).step_by(3) {
            if self.surface[c as usize] > sea {
                self.add_light_block(c, sea, hex(0xFF7A20), 12.0, 0.22);
            }
        }
        self.place_trees();
    }

    fn build_end(&mut self) {
        // one floating island, thickest under its middle, with the void around it
        let centre = self.rng.between(0.5, 0.62);
        let half = if self.wide { self.rng.between(0.42, 0.5) } else { 0.62 };
        let depth = self.rows as f32 * 0.3;
        for c in 0..self.cols {
            let xf = (c as f32 + 0.5) / self.cols as f32;
            let d = (xf - centre) / half;
            if d.abs() >= 1.0 {
                self.surface[c as usize] = self.rows;
                continue;
            }
            let s = self.surface[c as usize] + (d.abs().powi(4) * 2.0) as i32;
            self.surface[c as usize] = s;
            let thick = (1.0 + depth * (1.0 - d * d) + fbm(self.seed ^ 0xE4D, c as f32 * 0.4) * 1.5).max(1.0) as i32;
            for r in s..(s + thick).min(self.rows) {
                self.set(c, r, Block::EndStone);
            }
        }
        // an obsidian pillar or two with a crystal on top
        let pillars = if self.wide { self.rng.range(1, 2) } else { 1 };
        for i in 0..pillars {
            let lo = (self.cols as f32 * (centre + 0.05 + i as f32 * 0.18)) as i32;
            let c = lo.min(self.cols - 3).max(1);
            if c + 1 >= self.cols {
                continue;
            }
            let s = self.surface[c as usize].min(self.surface[c as usize + 1]);
            if s >= self.rows {
                continue;
            }
            let top = (s - self.rng.range(self.rows / 3, self.rows / 2)).max(2);
            for r in top..s {
                self.set(c, r, Block::Obsidian);
                self.set(c + 1, r, Block::Obsidian);
            }
            self.set(c, top - 1, Block::Crystal);
            self.add_light_block(c, top - 1, hex(0xE58AF0), 12.0, 0.3);
        }
        // chorus plants
        let mut c = self.rng.range(1, 4);
        while c < self.cols {
            let xf = (c as f32 + 0.5) / self.cols as f32;
            let s = self.surface[c as usize];
            if s < self.rows && self.at(c, s - 1) == Block::Air && self.rng.chance(0.3 * self.calm(xf)) {
                let h = self.rng.range(2, 4);
                for i in 1..=h {
                    self.set(c, s - i, Block::Chorus);
                }
                let side = if self.rng.chance(0.5) { -1 } else { 1 };
                if self.at(c + side, s - h + 1) == Block::Air {
                    self.set(c + side, s - h + 1, Block::Chorus);
                }
                c += self.rng.range(4, 8);
            } else {
                c += 1;
            }
        }
    }

    fn place_ores(&mut self) {
        let host = match self.look.ground {
            Ground::Nether => Block::Netherrack,
            Ground::End => Block::EndStone,
            _ => Block::Stone,
        };
        let candidates: Vec<(i32, i32)> = (0..self.cols)
            .flat_map(|c| (0..self.rows).map(move |r| (c, r)))
            .filter(|&(c, r)| {
                let below = r - self.surface[c as usize];
                self.at(c, r) == host && below >= 2 && (c as f32) > self.cols as f32 * 0.4
            })
            .collect();
        if candidates.is_empty() {
            return;
        }
        // a random sequence of its own, so the loader changes only the ores and not the rest of the scene
        let mut rng = Rng(self.seed ^ 0x0E5);
        let n = candidates.len() as i32 - 1;
        if host == Block::Stone {
            for _ in 0..candidates.len() / 25 {
                let (c, r) = candidates[rng.range(0, n) as usize];
                self.set(c, r, Block::Coal);
            }
        }
        if self.loader.is_some() {
            let count = if self.wide { 3 } else { 2 };
            for _ in 0..count {
                let (c, r) = candidates[rng.range(0, n) as usize];
                self.set(c, r, Block::LoaderOre);
            }
        }
    }

    fn add_light_block(&mut self, c: i32, r: i32, color: Rgb, radius: f32, strength: f32) {
        self.lights.push(Light {
            x: (c * B + B / 2) as f32,
            y: (r * B + B / 2) as f32,
            color,
            radius: radius * (self.ih as f32 / 90.0).clamp(0.6, 1.5),
            strength,
        });
    }

    // -- the sky --

    fn paint_sky(&mut self) {
        let [top, mid, low] = self.look.sky;
        let horizon = self.horizon.max(8) as f32;
        for y in 0..self.ih {
            let t = (y as f32 / horizon).clamp(0.0, 1.0);
            let c = if t < 0.55 { top.mix(mid, t / 0.55) } else { mid.mix(low, (t - 0.55) / 0.45) };
            for x in 0..self.iw {
                self.img[(y * self.iw + x) as usize] = c;
            }
        }
        if self.look.stars > 0.0 {
            let sseed = self.seed ^ 0x57A2;
            for y in 0..(horizon as i32 * 4 / 5) {
                for x in 0..self.iw {
                    let h = hash(sseed, x, y);
                    let p = 0.012 * self.look.stars * (0.35 + 0.65 * self.calm(x as f32 / self.iw as f32));
                    if unit(h) < p {
                        let bright = 0.35 + 0.6 * unit(h.rotate_left(7));
                        let fade = 1.0 - y as f32 / horizon;
                        self.blend(x, y, hex(0xF2F0FF), bright * fade);
                    }
                }
            }
        }
        match self.look.celestial {
            Celestial::Sun => self.paint_sun(),
            Celestial::Moon => self.paint_moon(),
            Celestial::None => {}
        }
        if let Some(cloud) = self.look.clouds {
            self.paint_clouds(cloud);
        }
    }

    fn sun_rect(&self) -> (i32, i32, i32) {
        let size = ((self.ih as f32 / 8.0) * self.look.sun_size).round().max(4.0) as i32;
        let (fx, fy) = self.look.sun_at;
        let fx = if self.wide { fx } else { fx.clamp(0.55, 0.72) };
        let x = (self.iw as f32 * fx) as i32 - size / 2;
        let y = (self.ih as f32 * fy) as i32 - size / 2;
        (x, y, size)
    }

    fn paint_sun(&mut self) {
        let (x0, y0, size) = self.sun_rect();
        let sunset = self.theme == CoverTheme::Sunset;
        let core = if sunset { hex(0xFFE2A0) } else { hex(0xFFF6C8) };
        let glow = if sunset { hex(0xFFC27A) } else { hex(0xFFF4D0) };
        // a stepped square halo, like light in blocks
        let reach = size + size / 2;
        for y in (y0 - reach)..(y0 + size + reach) {
            for x in (x0 - reach)..(x0 + size + reach) {
                let dx = (x0 - x).max(x - (x0 + size - 1)).max(0);
                let dy = (y0 - y).max(y - (y0 + size - 1)).max(0);
                let d = dx.max(dy);
                if d == 0 {
                    let rim = x == x0 || y == y0 || x == x0 + size - 1 || y == y0 + size - 1;
                    self.put(x, y, if rim { core.mix(glow, 0.5) } else { core });
                } else {
                    let step = (d + 1) / 2 * 2;
                    let a = (1.0 - step as f32 / reach as f32).max(0.0);
                    self.blend(x, y, glow, a * a * if sunset { 0.5 } else { 0.35 });
                }
            }
        }
    }

    fn paint_moon(&mut self) {
        let (x0, y0, size) = self.sun_rect();
        let size = (size * 4 / 5).max(4);
        let reach = size;
        for y in (y0 - reach)..(y0 + size + reach) {
            for x in (x0 - reach)..(x0 + size + reach) {
                let dx = (x0 - x).max(x - (x0 + size - 1)).max(0);
                let dy = (y0 - y).max(y - (y0 + size - 1)).max(0);
                let d = dx.max(dy);
                if d == 0 {
                    let (u, v) = (x - x0, y - y0);
                    // a few darker craters in fixed spots
                    let crater = (u * 4 / size, v * 4 / size);
                    let dark = matches!(crater, (1, 1) | (2, 3) | (3, 1));
                    self.put(x, y, if dark { hex(0xB9BFD4) } else { hex(0xE9ECF5) });
                } else {
                    let step = (d + 1) / 2 * 2;
                    let a = (1.0 - step as f32 / reach as f32).max(0.0);
                    self.blend(x, y, hex(0x9AA8D8), a * a * 0.25);
                }
            }
        }
    }

    fn paint_clouds(&mut self, color: Rgb) {
        let count = ((self.iw as f32 / 34.0) * self.look.cloud_count).round().max(1.0) as i32;
        let band = (self.horizon as f32 * 0.55).max(6.0) as i32;
        let thick = (B - 1).max(2);
        for _ in 0..count {
            let w = self.rng.range(4, 9) * B / 2 + B;
            let x = self.rng.range(-B, self.iw);
            let y = self.rng.range(2, band);
            let xf = (x as f32 + w as f32 / 2.0) / self.iw as f32;
            let alpha = 0.55 + 0.35 * self.calm(xf);
            // a long slab with a shorter one on top, like Minecraft's flat clouds seen from the side
            let top_w = w * self.rng.range(4, 7) / 10;
            let top_x = x + self.rng.range(1, (w - top_w).max(2) - 1);
            for (cx, cy, cw, ch) in [(top_x, y - thick + 1, top_w, thick - 1), (x, y, w, thick)] {
                for yy in cy..cy + ch {
                    for xx in cx..cx + cw {
                        let shade = if yy == y + thick - 1 { 0.9 } else { 1.0 };
                        self.blend(xx, yy, color.scale(shade), alpha);
                    }
                }
            }
        }
    }

    // -- the distant hills --

    fn paint_far(&mut self) {
        let fseed = self.seed ^ 0xFA4;
        let base = self.horizon as f32;
        let low = self.look.sky[2];
        let ocean = self.look.ground == Ground::Island;
        if ocean {
            // the open sea reaching to the horizon, lighter far away
            let far_sea = self.look.water.mix(low, 0.45);
            for y in self.horizon.max(0)..self.ih {
                let t = ((y - self.horizon) as f32 / 12.0).clamp(0.0, 1.0);
                let c = far_sea.mix(self.look.water, t * 0.6);
                for x in 0..self.iw {
                    let glint = unit(hash(fseed, x / 3, y)) < 0.03;
                    self.put(x, y, if glint { c.mix(hex(0xFFFFFF), 0.25) } else { c });
                }
            }
        }
        for layer in 0..2 {
            let step = 2;
            let (amp, lift, freq) = match (self.look.ground, layer) {
                (Ground::Snow, 0) => (16.0, 10.0, 0.025),
                (Ground::Island, _) => (3.0, -1.0, 0.04),
                (Ground::End, _) => (0.0, 0.0, 0.0),
                (Ground::Nether, 0) => (12.0, 14.0, 0.04),
                (_, 0) => (9.0, 12.0, 0.03),
                (_, _) => (5.0, 5.0, 0.05),
            };
            if self.look.ground == Ground::End {
                self.paint_far_end(layer);
                continue;
            }
            let color = self.look.far[layer];
            let lseed = fseed.wrapping_add(layer as u64 * 31);
            let mut tops = vec![0; self.iw as usize];
            for x in 0..self.iw {
                let qx = x / step * step;
                let mut n = noise1(lseed, qx as f32 * freq) * 0.75 + noise1(lseed ^ 9, qx as f32 * freq * 2.7) * 0.25;
                if ocean {
                    // only a distant island or two
                    n = (n - 0.55).max(0.0) * 3.0;
                }
                let top = base - lift - n * amp;
                let top = ((top / step as f32).round() as i32) * step;
                tops[x as usize] = top;
                for y in top.max(0)..self.ih {
                    // atmospheric haze: the tops of the hills fade into the sky
                    let t = ((y - top) as f32 / 10.0).clamp(0.0, 1.0);
                    let mut c = low.mix(color, 0.8 + 0.2 * t);
                    if self.look.ground == Ground::Snow && layer == 0 && y - top < 2 + (n * 4.0) as i32 {
                        c = c.mix(hex(0xF4F8FF), 0.7);
                    }
                    self.put(x, y, c);
                }
            }
            if layer == 1 && !ocean {
                self.paint_far_trees(&tops, color, lseed);
                if let Some(sprite) = self.look.structure {
                    self.paint_structure(&tops, sprite, color);
                }
            }
        }
    }

    /// Little blobs of trees along the nearer hills.
    fn paint_far_trees(&mut self, tops: &[i32], color: Rgb, lseed: u64) {
        let (spruce, density) = match self.look.ground {
            Ground::Snow => (true, 0.08),
            Ground::Grass => (self.look.trees.contains(&Wood::Spruce), 0.06 + self.look.tree_density * 0.6),
            _ => (false, 0.0),
        };
        // the same colour as the ridge top, so the trees read as its outline rather than floating
        let dark = self.look.sky[2].mix(color, 0.85);
        let mut x = 2;
        while x < self.iw - 2 {
            let xf = x as f32 / self.iw as f32;
            if unit(hash(lseed, x, 1)) < density * (0.3 + self.calm(xf)) {
                let top = tops[x as usize];
                let h = 3 + (hash(lseed, x, 2) % 3) as i32;
                for dy in 0..h + 2 {
                    let half = if spruce {
                        ((dy + 1) / 2).min(2)
                    } else if dy == 0 || dy >= h - 1 {
                        1
                    } else {
                        2
                    };
                    for dx in -half..=half {
                        self.put(x + dx, top - h + dy + 1, dark);
                    }
                }
                x += 5;
            } else {
                x += 1;
            }
        }
    }

    fn paint_structure(&mut self, tops: &[i32], sprite: &[&str], color: Rgb) {
        let w = sprite[0].len() as i32;
        let x0 = (self.iw as f32 * self.rng.between(0.55, 0.85)) as i32 - w / 2;
        if x0 < 0 || x0 + w >= self.iw {
            return;
        }
        let ground = (x0..x0 + w).map(|x| tops[x as usize]).max().unwrap_or(0) + 1;
        let body = color.scale(0.82);
        let y0 = ground - sprite.len() as i32;
        for (dy, row) in sprite.iter().enumerate() {
            for (dx, ch) in row.bytes().enumerate() {
                let (x, y) = (x0 + dx as i32, y0 + dy as i32);
                match ch {
                    b'#' => self.put(x, y, body),
                    b'd' => self.put(x, y, body.scale(0.8)),
                    b'w' if self.look.lit => {
                        self.put(x, y, hex(0xFFD27A));
                        self.lights.push(Light {
                            x: x as f32,
                            y: y as f32,
                            color: hex(0xFFC060),
                            radius: 5.0,
                            strength: 0.15,
                        });
                    }
                    b'w' => self.put(x, y, body.scale(0.75)),
                    _ => {}
                }
            }
        }
        // fill under it down to the hill so it doesn't float
        for x in x0..x0 + w {
            for y in ground..=tops[x as usize].max(ground) {
                if sprite.last().is_some_and(|r| r.as_bytes()[(x - x0) as usize] != b'.') {
                    self.put(x, y, body);
                }
            }
        }
    }

    /// The End has no hills: far away there are small islands and pillars in the void.
    fn paint_far_end(&mut self, layer: usize) {
        let color = self.look.far[layer].mix(self.look.sky[2], 0.3);
        let count = if layer == 0 { 3 } else { 2 };
        for _ in 0..count {
            let w = self.rng.range(6, 14) + layer as i32 * 4;
            let x0 = self.rng.range(self.iw / 3, self.iw - 4);
            let y0 = self.rng.range(self.ih / 3, self.ih * 3 / 5) + layer as i32 * 4;
            for dx in 0..w {
                let t = (dx as f32 + 0.5) / w as f32 * 2.0 - 1.0;
                let depth = ((1.0 - t * t) * (w as f32 * 0.35)) as i32 + 1;
                for dy in 0..depth {
                    self.put(x0 + dx, y0 + dy, color.scale(1.0 - dy as f32 * 0.05));
                }
            }
            if layer == 0 && self.rng.chance(0.6) {
                let px = x0 + w / 2;
                let h = self.rng.range(8, 16);
                for dy in 0..h {
                    self.put(px, y0 - dy, color.scale(0.8));
                    self.put(px + 1, y0 - dy, color.scale(0.8));
                }
                self.put(px, y0 - h, hex(0xC47ED0));
            }
        }
    }

    // -- the near terrain --

    /// Steps from this block to open space above or below (up to 6), or 1 when a side is open: the deeper inside
    /// the ground, the darker. Measured up and down only, so the shade lies in calm layers under the surface.
    fn depth(&self, c: i32, r: i32) -> i32 {
        // trunks and plants don't shade the ground they stand on
        let open = |c: i32, r: i32| !self.at(c, r).ground();
        if open(c - 1, r) || open(c + 1, r) {
            return 1;
        }
        (1..=6).find(|&k| open(c, r - k) || open(c, r + k)).unwrap_or(6)
    }

    fn paint_blocks(&mut self) {
        let bseed = self.seed ^ 0xB10C;
        for r in 0..self.rows {
            for c in 0..self.cols {
                let b = self.at(c, r);
                if b == Block::Air {
                    continue;
                }
                let depth = self.depth(c, r);
                let up = self.at(c, r - 1).open() && !matches!(b, Block::Water | Block::Lava);
                let down = self.at(c, r + 1).open() && !matches!(b, Block::Water | Block::Lava);
                let liquid = matches!(b, Block::Water | Block::Lava);
                let left = self.at(c - 1, r).open() && !liquid;
                let right = self.at(c + 1, r).open() && !liquid;
                // the floor of a lake, seen through the water; at sea everything under the waterline is
                let floor = self.surface[c as usize];
                let under_water = match (self.sea_row, self.look.ground) {
                    (_, Ground::Nether) => None,
                    (Some(w), Ground::Island) if r >= w => Some(((r - w) as f32 / 4.0).clamp(0.4, 0.75)),
                    (Some(w), _) if floor > w && r >= floor && r < floor + 3 => Some(0.68),
                    _ => None,
                }
                .filter(|_| !b.emissive() && b != Block::Water);
                let xf = (c as f32 + 0.5) / self.cols as f32;
                let calm = 0.55 + 0.45 * self.calm(xf);
                for py in 0..B {
                    for px in 0..B {
                        let (x, y) = (c * B + px, r * B + py);
                        // water ripples run sideways
                        let h = if b == Block::Water { hash(bseed, x / 3, y) } else { hash(bseed, x, y) };
                        let q = [-1.0, -0.35, 0.35, 1.0][(h & 3) as usize];
                        let (mut col, amp, glow) = self.texel(b, c, r, px, py, h, up);
                        col = col.scale(1.0 + q * amp * calm);
                        if !glow {
                            // faces open to the air catch the light; undersides and the right sides are shaded
                            if up && py == 0 {
                                col = col.scale(1.13);
                            }
                            if left && px == 0 {
                                col = col.scale(1.04);
                            }
                            if right && px == B - 1 {
                                col = col.scale(0.86);
                            }
                            if down && py == B - 1 {
                                col = col.scale(0.8);
                            }
                            if !b.open() {
                                col = col.scale((1.06 - depth as f32 * 0.06).clamp(0.62, 1.0));
                            }
                            if let Some(t) = under_water {
                                col = col.mix(self.look.water.scale(0.6), t);
                            }
                            col = self.lit(col);
                        }
                        self.put(x, y, col);
                    }
                }
            }
        }
    }

    /// The colour of one pixel of a block before shading: (colour, how much texture noise, whether it glows).
    #[allow(clippy::too_many_arguments)]
    fn texel(&self, b: Block, c: i32, r: i32, px: i32, py: i32, h: u64, up: bool) -> (Rgb, f32, bool) {
        let dirt = hex(0x8A5F3C);
        let stone = hex(0x86868A);
        let speck = unit(h.rotate_left(9));
        match b {
            Block::Air => (hex(0), 0.0, false),
            Block::Grass => {
                // the green top, with a few blades hanging over the dirt below it
                let drip = py == 1 && unit(h.rotate_left(13)) < 0.45;
                if py == 0 || drip {
                    (self.look.grass.scale(if py == 0 { 1.0 } else { 0.88 }), 0.07, false)
                } else {
                    (dirt, 0.08, false)
                }
            }
            Block::SnowyGrass => {
                let drip = py == 2 && unit(h.rotate_left(13)) < 0.4;
                if py <= 1 || drip { (hex(0xF2F6FC), 0.025, false) } else { (dirt, 0.08, false) }
            }
            Block::Dirt => (dirt, 0.08, false),
            Block::Stone => (stone, 0.07, false),
            Block::Deepslate => {
                let streak = (py + c) % 3 == 0;
                (hex(0x4D4D55).scale(if streak { 0.9 } else { 1.0 }), 0.06, false)
            }
            Block::Gravel => (hex(0x857F7C), 0.14, false),
            Block::Sand => (hex(0xDCCB92), 0.035, false),
            Block::Sandstone => {
                let band = py == 0 || py == B - 1;
                (hex(0xD2BD82).scale(if band { 0.92 } else { 1.0 }), 0.03, false)
            }
            Block::Ice => (hex(0x9CC1F2), 0.04, false),
            Block::Water => {
                let w = self.look.water;
                let depth_rows = self.sea_row.map_or(0, |s| r - s);
                let d = (depth_rows * B + py) as f32;
                let mut col = w.scale((1.05 - d * 0.012).max(0.55));
                if depth_rows == 0 && py == 0 {
                    // the surface catches the sky
                    col = col.mix(self.look.sky[2], 0.55);
                } else if depth_rows == 0 && py == 1 && unit(h.rotate_left(11)) < 0.25 {
                    col = col.mix(hex(0xFFFFFF), 0.18);
                }
                (col, 0.015, false)
            }
            Block::Lava => {
                let hot = unit(hash(self.seed, c * B + px, (r * B + py) / 2)) < 0.25;
                let base = if hot { hex(0xFFC040) } else { hex(0xE8641A) };
                let top = self.sea_row == Some(r) && py == 0;
                (if top { hex(0xFFD060) } else { base }, 0.05, true)
            }
            Block::Coal => {
                if ORE[py as usize][px as usize] {
                    (hex(0x2E2E30), 0.05, false)
                } else {
                    (stone, 0.07, false)
                }
            }
            Block::LoaderOre => {
                let host = match self.look.ground {
                    Ground::Nether => hex(0x6E2A2A),
                    Ground::End => hex(0xD8D59E),
                    _ => stone,
                };
                match (ORE[py as usize][px as usize], self.loader) {
                    (true, Some(l)) => (l, 0.06, true),
                    _ => (host, 0.07, false),
                }
            }
            Block::Log(wood) => {
                let (bark, dark) = match wood {
                    Wood::Oak => (hex(0x6B5132), 0.85),
                    Wood::Birch => (hex(0xD9D6CC), 0.35),
                    Wood::Spruce => (hex(0x4A3521), 0.85),
                    Wood::Cherry => (hex(0x4A2A33), 0.85),
                    Wood::Crimson => (hex(0x7A2D3E), 0.8),
                    Wood::Warped => (hex(0x3A6B66), 0.8),
                };
                let stripe = if wood == Wood::Birch { unit(h.rotate_left(17)) < 0.18 } else { px % 2 == 1 };
                (bark.scale(if stripe { dark } else { 1.0 }), 0.05, false)
            }
            Block::Leaves(_) | Block::SnowyLeaves if !(b == Block::SnowyLeaves && py == 0 && up) => {
                let base = match b {
                    Block::Leaves(Wood::Oak) => self.look.grass.mix(hex(0x3F7F2B), 0.5),
                    Block::Leaves(Wood::Birch) => hex(0x7BA347),
                    Block::Leaves(Wood::Cherry) => hex(0xF2A9C9),
                    _ => hex(0x3E6B40),
                };
                // leaves are lumpy: some pixels are gaps of shade
                let hole = speck < 0.2;
                (base.scale(if hole { 0.72 } else { 1.0 }), 0.1, false)
            }
            Block::Leaves(_) | Block::SnowyLeaves => (hex(0xF2F6FC), 0.03, false),
            Block::Cactus => {
                let edge = px == 0 || px == B - 1;
                let thorn = edge && unit(h.rotate_left(15)) < 0.3;
                (if thorn { hex(0xC9D9A0) } else { hex(0x5C8A34).scale(if edge { 0.82 } else { 1.0 }) }, 0.05, false)
            }
            Block::Netherrack => (hex(0x7E302D), 0.1, false),
            Block::Nylium(warped) => {
                let top = if warped { hex(0x2B8C84) } else { hex(0xB0302E) };
                let drip = py == 1 && unit(h.rotate_left(13)) < 0.45;
                if py == 0 || drip { (top, 0.08, false) } else { (hex(0x7E302D), 0.1, false) }
            }
            Block::Wart(warped) => (if warped { hex(0x1E8C82) } else { hex(0x8E1E1E) }, 0.1, false),
            Block::Shroomlight => (hex(0xFFB054), 0.06, true),
            Block::Glowstone => {
                let bright = speck < 0.4;
                (if bright { hex(0xFFE6A0) } else { hex(0xC8913E) }, 0.05, true)
            }
            Block::EndStone => (hex(0xD8D59E), 0.05, false),
            Block::Obsidian => {
                let fleck = speck < 0.1;
                (if fleck { hex(0x3E2A5C) } else { hex(0x1A1226) }, 0.05, false)
            }
            Block::Crystal => {
                let ring = px == 0 || py == 0 || px == B - 1 || py == B - 1;
                (if ring { hex(0xB46AD4) } else { hex(0xF6C8FF) }, 0.03, true)
            }
            Block::Chorus => {
                let flower = self.at(c, r - 1) == Block::Air && unit(hash(self.seed, c, r)) < 0.5;
                (if flower { hex(0xC9A2D9) } else { hex(0x7A5A87) }, 0.08, false)
            }
            Block::Coral(k) => {
                let base = [hex(0xD45A97), hex(0xE9C64A), hex(0x3A66D9)][k as usize % 3];
                let bump = (px + py + c) % 2 == 0;
                (base.scale(if bump { 1.0 } else { 0.85 }), 0.05, false)
            }
        }
    }

    /// Small things that are not whole blocks: grass tufts and flowers, dead bushes, kelp, petals.
    fn paint_plants(&mut self) {
        let pseed = self.seed ^ 0x9A57;
        for c in 0..self.cols {
            let s = self.surface[c as usize];
            let xf = (c as f32 + 0.5) / self.cols as f32;
            let calm = self.calm(xf);
            let top = self.at(c, s);
            let above = self.at(c, s - 1);
            let y = s * B - 1;
            for px in 0..B {
                let x = c * B + px;
                let h = hash(pseed, x, 0);
                let roll = unit(h);
                match (top, above) {
                    (Block::Grass, Block::Air) => {
                        let tall = self.theme != CoverTheme::Night;
                        if roll < 0.22 * (0.5 + calm) {
                            let blade = self.lit(self.look.grass.scale(0.85));
                            self.put(x, y, blade);
                            if tall && unit(h.rotate_left(8)) < 0.4 {
                                self.put(x, y - 1, blade);
                            }
                        } else if roll < 0.26 && calm > 0.5 && self.theme != CoverTheme::Cherry {
                            let petal =
                                [hex(0xE0413A), hex(0xF3D23C), hex(0x5C7FE0), hex(0xF4F4F4)][(h >> 20) as usize % 4];
                            self.put(x, y, self.lit(hex(0x4E8A2E)));
                            self.put(x, y - 1, self.lit(petal));
                        } else if self.theme == CoverTheme::Cherry && roll < 0.4 {
                            // fallen petals on the grass
                            self.put(x, s * B, hex(0xF5B7D2));
                        }
                    }
                    (Block::Sand, Block::Air) if self.look.ground == Ground::Sand && roll < 0.05 * calm => {
                        // a dead bush
                        let twig = self.lit(hex(0x8A6A3A));
                        self.put(x, y, twig);
                        self.put(x - 1, y - 1, twig);
                        self.put(x + 1, y - 1, twig);
                        self.put(x, y - 2, twig);
                    }
                    (Block::Nylium(warped), Block::Air) if roll < 0.2 * (0.5 + calm) => {
                        let root = if warped { hex(0x2EA394) } else { hex(0xA8323A) };
                        self.put(x, y, self.lit(root));
                    }
                    _ => {}
                }
            }
            // kelp and seagrass under water
            if let Some(w) = self.sea_row
                && s > w + 1
                && matches!(self.look.ground, Ground::Island | Ground::Grass)
            {
                let x = c * B + (hash(pseed, c, 3) % B as u64) as i32;
                let roll = unit(hash(pseed, c, 4));
                let floor = s * B - 1;
                if roll < 0.35 * calm && self.look.ground == Ground::Island {
                    let len = ((s - w) * B - 3).max(2);
                    let len = (len as f32 * unit(hash(pseed, c, 5)).max(0.4)) as i32;
                    for i in 0..len {
                        let kelp = hex(0x3F7A2A).mix(self.look.water, 0.3 + 0.2 * (i % 2) as f32);
                        self.put(x, floor - i, kelp);
                        if i % 3 == 1 {
                            self.put(x + if i % 2 == 0 { 1 } else { -1 }, floor - i, kelp.scale(0.9));
                        }
                    }
                } else if roll < 0.6 {
                    let grass = hex(0x4F8A35).mix(self.look.water, 0.4);
                    self.put(x, floor, grass);
                    self.put(x + 1, floor, grass);
                    self.put(x, floor - 1, grass);
                }
            }
        }
    }

    /// The loader's banner on a post, on a hill on the right.
    fn paint_banner(&mut self) {
        let Some(cloth) = self.loader else { return };
        let start = (self.cols as f32 * if self.wide { 0.55 } else { 0.18 }) as i32;
        let end = (self.cols as f32 * if self.wide { 0.92 } else { 0.5 }) as i32;
        let spot = (start..end)
            .filter(|&c| {
                let s = self.surface[c as usize];
                s < self.rows
                    && self.at(c, s - 1) == Block::Air
                    && self.at(c, s - 2) == Block::Air
                    && self.at(c, s - 3) == Block::Air
                    && !self.at(c, s).open()
            })
            .min_by_key(|&c| (self.surface[c as usize], hash(self.seed, c, 11) % 7));
        let Some(c) = spot else { return };
        let ground_y = self.surface[c as usize] * B;
        let pole = self.lit(hex(0x6B5132));
        let x = c * B + 1;
        let height = 11;
        for i in 1..=height {
            self.put(x, ground_y - i, pole);
        }
        let bar_y = ground_y - height;
        for dx in 0..5 {
            self.put(x + dx, bar_y, pole.scale(0.9));
        }
        let cloth = if self.look.lit { cloth.scale(0.75) } else { cloth };
        for dy in 1..=7 {
            for dx in 1..=4 {
                let mut col = cloth;
                if dx == 1 {
                    col = col.scale(1.08);
                }
                if dx == 4 {
                    col = col.scale(0.82);
                }
                // a small emblem in the middle and a notched hem
                if (dy == 3 || dy == 4) && (dx == 2 || dx == 3) {
                    col = col.mix(hex(0xFFFFFF), 0.45);
                }
                if dy == 7 && (dx == 2 || dx == 3) {
                    continue;
                }
                self.put(x + dx, bar_y + dy, col);
            }
        }
    }

    fn paint_weather(&mut self) {
        if !self.look.snowfall {
            return;
        }
        let wseed = self.seed ^ 0x5110;
        for y in 0..self.ih {
            for x in 0..self.iw {
                let h = hash(wseed, x, y);
                if unit(h) < 0.004 * (0.3 + self.calm(x as f32 / self.iw as f32)) {
                    self.blend(x, y, hex(0xFFFFFF), 0.75);
                }
            }
        }
    }

    /// Glowing blocks light up what is around them, in soft steps.
    fn paint_lights(&mut self) {
        let lights = std::mem::take(&mut self.lights);
        for l in &lights {
            let r = l.radius.ceil() as i32;
            let (cx, cy) = (l.x as i32, l.y as i32);
            for y in (cy - r).max(0)..(cy + r).min(self.ih) {
                for x in (cx - r).max(0)..(cx + r).min(self.iw) {
                    let d = (((x - cx) * (x - cx) + (y - cy) * (y - cy)) as f32).sqrt();
                    let d = (d / 2.0).floor() * 2.0;
                    if d < l.radius {
                        let a = (1.0 - d / l.radius).powi(2) * l.strength;
                        let p = &mut self.img[(y * self.iw + x) as usize];
                        *p = p.add(l.color.scale(a * 0.6)).mix(l.color, a * 0.25);
                    }
                }
            }
        }
    }

    /// A gentle vignette, and the dark fade on the left when asked for.
    fn finish(&mut self) {
        let (w, h) = (self.iw as f32, self.ih as f32);
        let fade_to = hex(0x0B0D14);
        for y in 0..self.ih {
            for x in 0..self.iw {
                let (u, v) = ((x as f32 + 0.5) / w * 2.0 - 1.0, (y as f32 + 0.5) / h * 2.0 - 1.0);
                let vig = 1.0 - 0.22 * (u * u * 0.6 + v * v).powf(1.4);
                let i = (y * self.iw + x) as usize;
                let mut c = self.img[i].scale(vig);
                if self.spec.fade_left {
                    let t = (1.0 - (x as f32 + 0.5) / (w * 0.62)).max(0.0);
                    c = c.mix(fade_to, t * t * 0.72);
                }
                self.img[i] = c;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(key: &str, theme: CoverTheme, loader: Option<&str>) -> CoverSpec {
        CoverSpec::new(key, "1.21.4", loader, theme)
    }

    #[test]
    fn covers_are_deterministic() {
        let s = spec("my-instance", CoverTheme::Auto, Some("Fabric"));
        assert_eq!(render(&s, 480, 270).pixels, render(&s, 480, 270).pixels);
    }

    #[test]
    fn covers_differ_between_seeds() {
        for theme in CoverTheme::ALL {
            let a = render(&spec("alpha", theme, None), 160, 90);
            let b = render(&spec("beta", theme, None), 160, 90);
            assert_ne!(a.pixels, b.pixels, "{theme:?}");
        }
    }

    #[test]
    fn covers_every_theme_renders_and_differs() {
        let mut seen = Vec::new();
        for theme in CoverTheme::ALL.into_iter().skip(1) {
            let img = render(&spec("same", theme, Some("Forge")), 160, 90);
            assert!(img.pixels.iter().any(|p| *p != img.pixels[0]), "{theme:?} is flat");
            assert!(!seen.contains(&img.pixels), "{theme:?} looks like another theme");
            seen.push(img.pixels);
        }
    }

    #[test]
    fn covers_respect_sizes() {
        for (w, h) in [(1, 1), (96, 96), (480, 270), (333, 77), (1200, 400), (17, 200)] {
            for theme in CoverTheme::ALL {
                let img = render(&spec("sizes", theme, Some("Quilt")), w, h);
                assert_eq!(img.size, [w, h]);
                assert_eq!(img.pixels.len(), w * h);
            }
        }
    }

    #[test]
    fn covers_theme_ids_round_trip() {
        for theme in CoverTheme::ALL {
            assert_eq!(CoverTheme::from_id(theme.id()), Some(theme));
            assert_eq!(serde_json::to_string(&theme).unwrap(), format!("\"{}\"", theme.id()));
        }
        assert_eq!(CoverTheme::from_id("nope"), None);
        // auto always lands on a real theme
        for i in 0..200 {
            assert_ne!(CoverTheme::Auto.resolve(i, "1.21"), CoverTheme::Auto);
        }
    }

    #[test]
    fn covers_versions() {
        assert!(era("b1.7.3") == Era::Classic);
        assert!(era("1.8.9") == Era::Old);
        assert!(era("1.12.2") == Era::Modern);
        assert!(era("24w14a") == Era::Modern);
        assert!(!has_deepslate("1.16.5"));
        assert!(has_deepslate("1.17"));
    }

    #[test]
    fn covers_fade_left_darkens_the_left() {
        let s = spec("fade", CoverTheme::Meadow, None);
        let plain = render(&s, 160, 90);
        let faded = render(&s.clone().with_fade_left(true), 160, 90);
        let lum = |img: &ColorImage| -> u32 {
            (0..90).map(|y| img.pixels[y * 160 + 5]).map(|p| p.r() as u32 + p.g() as u32 + p.b() as u32).sum()
        };
        assert!(lum(&faded) < lum(&plain));
    }

    /// Contact sheets to look at: `cargo test covers_sheet -- --ignored`, then open /tmp/bf-covers/.
    #[test]
    #[ignore]
    fn covers_sheet() {
        use std::time::Instant;
        let dir = std::path::Path::new("/tmp/bf-covers");
        std::fs::create_dir_all(dir).unwrap();
        let save = |name: &str, img: &ColorImage| {
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b(), 255]).collect();
            image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes)
                .unwrap()
                .save(dir.join(name))
                .unwrap();
        };
        // tiles laid out in a grid with a gap
        let sheet = |tiles: &[ColorImage], per_row: usize| -> ColorImage {
            let (tw, th) = (tiles[0].size[0], tiles[0].size[1]);
            let gap = 6;
            let rows = tiles.len().div_ceil(per_row);
            let (w, h) = (per_row * (tw + gap) + gap, rows * (th + gap) + gap);
            let mut out = ColorImage::filled([w, h], Color32::from_rgb(24, 26, 32));
            for (i, t) in tiles.iter().enumerate() {
                let (ox, oy) = (gap + (i % per_row) * (tw + gap), gap + (i / per_row) * (th + gap));
                for y in 0..th {
                    for x in 0..tw {
                        out.pixels[(oy + y) * w + ox + x] = t.pixels[y * tw + x];
                    }
                }
            }
            out
        };
        let loaders = [None, Some("Fabric"), Some("Forge"), Some("NeoForge"), Some("Quilt")];
        let keys = ["survival", "create-mods", "vault", "pixelmon", "skyblock", "rlcraft"];
        for theme in CoverTheme::ALL.into_iter().skip(1) {
            let wide: Vec<_> = keys[..4]
                .iter()
                .enumerate()
                .map(|(i, k)| render(&spec(k, theme, loaders[(i + 1) % 5]).with_fade_left(i == 3), 480, 270))
                .collect();
            save(&format!("theme-{}.png", theme.id()), &sheet(&wide, 2));
        }
        let small: Vec<_> = CoverTheme::ALL
            .into_iter()
            .skip(1)
            .flat_map(|t| keys.iter().enumerate().map(move |(i, k)| render(&spec(k, t, loaders[i % 5]), 96, 96)))
            .collect();
        save("small.png", &sheet(&small, 6));
        let auto: Vec<_> = (0..12)
            .map(|i| {
                let v = ["1.21.4", "1.8.9", "b1.7.3", "1.16.5"][i % 4];
                render(&CoverSpec::new(&format!("inst-{i}"), v, loaders[i % 5], CoverTheme::Auto), 480, 270)
            })
            .collect();
        save("auto.png", &sheet(&auto, 3));
        let by_loader: Vec<_> = loaders
            .iter()
            .flat_map(|l| [CoverTheme::Meadow, CoverTheme::Nether].map(|t| render(&spec("loader", t, *l), 480, 270)))
            .collect();
        save("loaders.png", &sheet(&by_loader, 2));

        let t = Instant::now();
        for i in 0..50 {
            render(&spec(&format!("bench-{i}"), CoverTheme::ALL[i % 11], Some("Fabric")), 480, 270);
        }
        println!("wide 480x270: {:.2} ms each", t.elapsed().as_secs_f64() * 1000.0 / 50.0);
        let t = Instant::now();
        for i in 0..50 {
            render(&spec(&format!("bench-{i}"), CoverTheme::ALL[i % 11], Some("Fabric")), 96, 96);
        }
        println!("small 96x96: {:.2} ms each", t.elapsed().as_secs_f64() * 1000.0 / 50.0);
    }
}
