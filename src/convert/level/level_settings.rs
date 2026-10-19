// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The settings stored in `level.dat`: Chunker's `ChunkerLevelSettings`.
//!
//! Chunker declares about 106 public fields and drives reading and writing through reflection over annotations
//! (`@Java`, `@Bedrock`, `@Category`, `@Hidden`, `@CustomType`). Here the same information is one declarative table
//! (`level_settings!` below): it generates the struct, its defaults, serde (the field names stay Chunker's, they are
//! the JSON keys and the level.dat keys) and a static [`FIELDS`] descriptor list that the generic
//! [`LevelSettings::from_nbt`], [`LevelSettings::to_nbt`] and [`LevelSettings::descriptive`] walk.
//!
//! The Rust field names are snake_case; [`FieldDesc::name`] keeps Chunker's name (also the serde name).

use serde::{Deserialize, Serialize};

use super::encoding::EncodingType;
use crate::convert::nbt::{Compound, Tag};

/// Field flag: read and written through [`SettingsIo::read_custom`] / [`SettingsIo::write_custom`].
pub const CUSTOM: u8 = 1;
/// Field flag: not shown in the descriptive (UI) listing.
pub const HIDDEN: u8 = 2;

/// The generator of the world (`ChunkerGeneratorType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GeneratorType {
    /// The normal world generator.
    Normal,
    /// Grass flat map generator.
    Flat,
    /// A generator that has nothing.
    #[default]
    Void,
    /// Cannot be converted unless input and output format match (the NBT is then copied).
    Custom,
}

impl GeneratorType {
    /// Chunker's constant name.
    pub fn name(self) -> &'static str {
        match self {
            GeneratorType::Normal => "NORMAL",
            GeneratorType::Flat => "FLAT",
            GeneratorType::Void => "VOID",
            GeneratorType::Custom => "CUSTOM",
        }
    }
}

/// UI category of a setting (`Category.Type`), in Chunker's enum order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    WorldSettings,
    GameRules,
    Restrictions,
    Weather,
    Misc,
    Unknown,
}

impl Category {
    /// The display name used as a key in the descriptive listing.
    pub fn name(self) -> &'static str {
        match self {
            Category::WorldSettings => "World Settings",
            Category::GameRules => "Game Rules",
            Category::Restrictions => "Restrictions",
            Category::Weather => "Weather",
            Category::Misc => "Misc",
            Category::Unknown => "Unknown",
        }
    }
}

/// The type of a setting field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    Bool,
    Byte,
    Short,
    Int,
    Long,
    Float,
    Double,
    Str,
    Generator,
}

impl SettingKind {
    /// Chunker's type label in the descriptive listing.
    pub fn label(self) -> &'static str {
        match self {
            SettingKind::Bool => "Boolean",
            SettingKind::Byte => "Byte",
            SettingKind::Short => "Int16",
            SettingKind::Int => "Int32",
            SettingKind::Long => "Int64",
            SettingKind::Float => "Single",
            SettingKind::Double => "Double",
            SettingKind::Str => "String",
            SettingKind::Generator => "ChunkerGeneratorType",
        }
    }
}

/// A dynamically typed setting value (the boxed Java value).
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Str(String),
    Generator(GeneratorType),
}

impl SettingValue {
    /// The value of a scalar NBT tag (`Tag.getBoxedValue`); `None` for lists, compounds and arrays.
    pub fn from_tag(tag: &Tag) -> Option<SettingValue> {
        Some(match tag {
            Tag::Byte(v) => SettingValue::Byte(*v),
            Tag::Short(v) => SettingValue::Short(*v),
            Tag::Int(v) => SettingValue::Int(*v),
            Tag::Long(v) => SettingValue::Long(*v),
            Tag::Float(v) => SettingValue::Float(*v),
            Tag::Double(v) => SettingValue::Double(*v),
            Tag::String(v) => SettingValue::Str(v.clone()),
            _ => return None,
        })
    }

    /// The matching NBT tag (`None` for the generator, which is always a custom field).
    pub fn to_tag(&self) -> Option<Tag> {
        Some(match self {
            SettingValue::Bool(v) => Tag::Byte(*v as i8),
            SettingValue::Byte(v) => Tag::Byte(*v),
            SettingValue::Short(v) => Tag::Short(*v),
            SettingValue::Int(v) => Tag::Int(*v),
            SettingValue::Long(v) => Tag::Long(*v),
            SettingValue::Float(v) => Tag::Float(*v),
            SettingValue::Double(v) => Tag::Double(*v),
            SettingValue::Str(v) => Tag::String(v.clone()),
            SettingValue::Generator(_) => return None,
        })
    }

    /// Java's `toString` of the boxed value.
    pub fn to_java_string(&self) -> String {
        match self {
            SettingValue::Bool(v) => v.to_string(),
            SettingValue::Byte(v) => v.to_string(),
            SettingValue::Short(v) => v.to_string(),
            SettingValue::Int(v) => v.to_string(),
            SettingValue::Long(v) => v.to_string(),
            SettingValue::Float(v) => format!("{v:?}"),
            SettingValue::Double(v) => format!("{v:?}"),
            SettingValue::Str(v) => v.clone(),
            SettingValue::Generator(v) => v.name().to_owned(),
        }
    }

    fn number(&self) -> Option<Num> {
        Some(match self {
            SettingValue::Byte(v) => Num::I(*v as i64),
            SettingValue::Short(v) => Num::I(*v as i64),
            SettingValue::Int(v) => Num::I(*v as i64),
            SettingValue::Long(v) => Num::I(*v),
            SettingValue::Float(v) => Num::F(*v as f64),
            SettingValue::Double(v) => Num::F(*v),
            _ => return None,
        })
    }

    /// JSON form, as Gson would write it.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::Value;
        match self {
            SettingValue::Bool(v) => Value::Bool(*v),
            SettingValue::Byte(v) => Value::from(*v),
            SettingValue::Short(v) => Value::from(*v),
            SettingValue::Int(v) => Value::from(*v),
            SettingValue::Long(v) => Value::from(*v),
            SettingValue::Float(v) => serde_json::Number::from_f64(*v as f64).map_or(Value::Null, Value::Number),
            SettingValue::Double(v) => serde_json::Number::from_f64(*v).map_or(Value::Null, Value::Number),
            SettingValue::Str(v) => Value::String(v.clone()),
            SettingValue::Generator(v) => Value::String(v.name().to_owned()),
        }
    }
}

enum Num {
    I(i64),
    F(f64),
}

impl Num {
    /// Java's `Number.intValue()` / `longValue()` narrowing.
    fn int(&self) -> i32 {
        match self {
            Num::I(v) => *v as i32,
            Num::F(v) => *v as i32,
        }
    }
    fn long(&self) -> i64 {
        match self {
            Num::I(v) => *v,
            Num::F(v) => *v as i64,
        }
    }
    fn byte(&self) -> i8 {
        match self {
            Num::I(v) => *v as i8,
            Num::F(v) => *v as i32 as i8,
        }
    }
    fn double(&self) -> f64 {
        match self {
            Num::I(v) => *v as f64,
            Num::F(v) => *v,
        }
    }
    fn float(&self) -> f32 {
        match self {
            Num::I(v) => *v as f32,
            Num::F(v) => *v as f32,
        }
    }
}

/// `ChunkerLevelSettings.convertType`: converts a value to the target type the way Chunker does.
pub fn convert_value(value: &SettingValue, target: SettingKind) -> Result<SettingValue, String> {
    use SettingKind as K;
    use SettingValue as V;
    // Already the right type.
    match (value, target) {
        (V::Bool(_), K::Bool)
        | (V::Byte(_), K::Byte)
        | (V::Short(_), K::Short)
        | (V::Int(_), K::Int)
        | (V::Long(_), K::Long)
        | (V::Float(_), K::Float)
        | (V::Double(_), K::Double)
        | (V::Str(_), K::Str)
        | (V::Generator(_), K::Generator) => return Ok(value.clone()),
        _ => {}
    }
    if let Some(n) = value.number() {
        match target {
            K::Bool => return Ok(V::Bool(n.byte() != 0)),
            K::Int => return Ok(V::Int(n.int())),
            K::Byte => return Ok(V::Byte(n.byte())),
            K::Long => return Ok(V::Long(n.long())),
            K::Double => return Ok(V::Double(n.double())),
            K::Float => return Ok(V::Float(n.float())),
            _ => {}
        }
    }
    if let (V::Bool(b), K::Byte) = (value, target) {
        return Ok(V::Byte(*b as i8));
    }
    if let V::Str(s) = value {
        let bad = |what: &str| format!("could not parse {s:?} as {what}");
        match target {
            K::Bool => return Ok(V::Bool(s.eq_ignore_ascii_case("true"))),
            K::Int => {
                return match s.as_str() {
                    "true" => Ok(V::Int(1)),
                    "false" => Ok(V::Int(0)),
                    _ => s.trim().parse().map(V::Int).map_err(|_| bad("an int")),
                };
            }
            K::Double => return s.trim().parse().map(V::Double).map_err(|_| bad("a double")),
            K::Byte => return s.parse().map(V::Byte).map_err(|_| bad("a byte")),
            K::Long => return s.parse().map(V::Long).map_err(|_| bad("a long")),
            K::Float => return s.trim().parse().map(V::Float).map_err(|_| bad("a float")),
            _ => {}
        }
    }
    if target == K::Str {
        return Ok(V::Str(value.to_java_string()));
    }
    Err(format!("No valid conversion method for {value:?} to {target:?}"))
}

/// A Rust type usable as a settings field.
pub trait SettingType: Sized {
    const KIND: SettingKind;
    fn to_value(&self) -> SettingValue;
    /// Exact match only (the value has been through [`convert_value`] first).
    fn from_value(value: &SettingValue) -> Option<Self>;
}

macro_rules! setting_type {
    ($ty:ty, $kind:ident, $variant:ident) => {
        impl SettingType for $ty {
            const KIND: SettingKind = SettingKind::$kind;
            fn to_value(&self) -> SettingValue {
                SettingValue::$variant(self.clone())
            }
            fn from_value(value: &SettingValue) -> Option<Self> {
                match value {
                    SettingValue::$variant(v) => Some(v.clone()),
                    _ => None,
                }
            }
        }
    };
}
setting_type!(bool, Bool, Bool);
setting_type!(i8, Byte, Byte);
setting_type!(i32, Int, Int);
setting_type!(i64, Long, Long);
setting_type!(f32, Float, Float);
setting_type!(f64, Double, Double);
setting_type!(String, Str, Str);
setting_type!(GeneratorType, Generator, Generator);

/// One entry of the field table.
pub struct FieldDesc {
    /// Chunker's field name (also the JSON key).
    pub name: &'static str,
    /// The Rust field name.
    pub rust_name: &'static str,
    pub kind: SettingKind,
    pub category: Category,
    /// The level.dat path when the field exists in Java (`None` = not a Java setting). Dots descend into compounds.
    pub java: Option<&'static str>,
    /// The level.dat path when the field exists in Bedrock.
    pub bedrock: Option<&'static str>,
    /// Output type override for Java (`@Java(type = ...)`).
    pub java_type: Option<SettingKind>,
    /// Output type override for Bedrock.
    pub bedrock_type: Option<SettingKind>,
    /// [`CUSTOM`] | [`HIDDEN`].
    pub flags: u8,
    pub get: fn(&LevelSettings) -> SettingValue,
    pub set: fn(&mut LevelSettings, SettingValue) -> bool,
}

impl FieldDesc {
    pub fn is_custom(&self) -> bool {
        self.flags & CUSTOM != 0
    }
    pub fn is_hidden(&self) -> bool {
        self.flags & HIDDEN != 0
    }
    /// The level.dat path for an edition, if the field exists there.
    pub fn path(&self, edition: EncodingType) -> Option<&'static str> {
        match edition {
            EncodingType::Java => self.java,
            EncodingType::Bedrock => self.bedrock,
        }
    }
    fn output_kind(&self, edition: EncodingType) -> SettingKind {
        let over = match edition {
            EncodingType::Java => self.java_type,
            EncodingType::Bedrock => self.bedrock_type,
        };
        over.unwrap_or(self.kind)
    }
}

/// The hooks a level reader/writer provides (`LevelReader.readCustomLevelSetting`, `LevelWriter.writeCustomLevelSetting`
/// and `Converter.logNonFatalException`).
pub trait SettingsIo {
    /// The edition being read or written.
    fn encoding_type(&self) -> EncodingType;
    /// Reads a [`CUSTOM`] field; `None` keeps the default. The reader may also change `settings` directly.
    fn read_custom(
        &self,
        _root: &Compound,
        _settings: &mut LevelSettings,
        _name: &str,
        _kind: SettingKind,
    ) -> Option<SettingValue> {
        None
    }
    /// Writes a [`CUSTOM`] field into `output`.
    fn write_custom(&self, _settings: &LevelSettings, _output: &mut Compound, _name: &str, _value: &SettingValue) {}
    /// Reports a recoverable problem.
    fn log_non_fatal(&self, message: String);
}

macro_rules! level_settings {
    ($( $field:ident : $ty:ty = $def:expr ; $json:literal ; $cat:ident ; java $java:expr ; bedrock $bed:expr ;
        jtype $jt:expr ; btype $bt:expr ; flags $fl:expr ; )*) => {
        /// The level settings (`ChunkerLevelSettings`). Field names in the JSON form are Chunker's.
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct LevelSettings {
            $( #[serde(rename = $json)] pub $field: $ty, )*
        }

        impl Default for LevelSettings {
            fn default() -> Self {
                LevelSettings { $( $field: $def, )* }
            }
        }

        /// Every field of [`LevelSettings`], in Chunker's declaration order.
        pub static FIELDS: &[FieldDesc] = &[
            $( FieldDesc {
                name: $json,
                rust_name: stringify!($field),
                kind: <$ty as SettingType>::KIND,
                category: Category::$cat,
                java: $java,
                bedrock: $bed,
                java_type: $jt,
                bedrock_type: $bt,
                flags: $fl,
                get: |s| s.$field.to_value(),
                set: |s, v| match <$ty as SettingType>::from_value(&v) {
                    Some(x) => {
                        s.$field = x;
                        true
                    }
                    None => false,
                },
            }, )*
        ];
    };
}

level_settings! {
    bonus_chest_enabled: bool = false; "bonusChestEnabled"; WorldSettings; java Some("BonusChest"); bedrock Some("bonusChestEnabled"); jtype None; btype None; flags 0;
    bonus_chest_spawned: bool = false; "bonusChestSpawned"; WorldSettings; java None; bedrock Some("bonusChestSpawned"); jtype None; btype None; flags 0;
    center_maps_to_origin: bool = false; "CenterMapsToOrigin"; Misc; java None; bedrock Some("CenterMapsToOrigin"); jtype None; btype None; flags 0;
    commandblockoutput: bool = true; "commandblockoutput"; GameRules; java Some("GameRules.commandBlockOutput"); bedrock Some("commandblockoutput"); jtype None; btype None; flags 0;
    commandblocksenabled: bool = true; "commandblocksenabled"; GameRules; java None; bedrock Some("commandblocksenabled"); jtype None; btype None; flags 0;
    commands_enabled: bool = true; "commandsEnabled"; GameRules; java Some("allowCommands"); bedrock Some("commandsEnabled"); jtype None; btype None; flags 0;
    confirmed_platform_locked_content: bool = false; "ConfirmedPlatformLockedContent"; Restrictions; java None; bedrock Some("ConfirmedPlatformLockedContent"); jtype None; btype None; flags 0;
    difficulty: i32 = 1; "Difficulty"; WorldSettings; java Some("Difficulty"); bedrock Some("Difficulty"); jtype Some(SettingKind::Byte); btype None; flags 0;
    dodaylightcycle: bool = true; "dodaylightcycle"; GameRules; java Some("GameRules.doDaylightCycle"); bedrock Some("dodaylightcycle"); jtype None; btype None; flags 0;
    doentitydrops: bool = true; "doentitydrops"; GameRules; java Some("GameRules.doEntityDrops"); bedrock Some("doentitydrops"); jtype None; btype None; flags 0;
    dofiretick: bool = true; "dofiretick"; GameRules; java Some("GameRules.doFireTick"); bedrock Some("dofiretick"); jtype None; btype None; flags 0;
    doimmediaterespawn: bool = false; "doimmediaterespawn"; GameRules; java None; bedrock Some("doimmediaterespawn"); jtype None; btype None; flags 0;
    doinsomnia: bool = true; "doinsomnia"; GameRules; java None; bedrock Some("doinsomnia"); jtype None; btype None; flags 0;
    domobloot: bool = true; "domobloot"; GameRules; java Some("GameRules.doMobLoot"); bedrock Some("domobloot"); jtype None; btype None; flags 0;
    domobspawning: bool = true; "domobspawning"; GameRules; java Some("GameRules.doMobSpawning"); bedrock Some("domobspawning"); jtype None; btype None; flags 0;
    dotiledrops: bool = true; "dotiledrops"; GameRules; java Some("GameRules.doTileDrops"); bedrock Some("dotiledrops"); jtype None; btype None; flags 0;
    doweathercycle: bool = true; "doweathercycle"; GameRules; java Some("GameRules.doWeatherCycle"); bedrock Some("doweathercycle"); jtype None; btype None; flags 0;
    drowningdamage: bool = true; "drowningdamage"; GameRules; java None; bedrock Some("drowningdamage"); jtype None; btype None; flags 0;
    education_features_enabled: bool = false; "educationFeaturesEnabled"; Misc; java None; bedrock Some("educationFeaturesEnabled"); jtype None; btype None; flags 0;
    edu_level: i8 = 0; "eduLevel"; Misc; java None; bedrock Some("eduLevel"); jtype None; btype None; flags 0;
    experimentalgameplay: bool = false; "experimentalgameplay"; WorldSettings; java None; bedrock Some("experimentalgameplay"); jtype None; btype None; flags 0;
    flat_world_version: i32 = 1; "FlatWorldVersion"; WorldSettings; java Some("FlatWorldVersion"); bedrock Some("FlatWorldVersion"); jtype None; btype None; flags CUSTOM | HIDDEN;
    caves_and_cliffs: bool = false; "CavesAndCliffs"; WorldSettings; java Some("CavesAndCliffs"); bedrock Some("CavesAndCliffs"); jtype None; btype None; flags CUSTOM | HIDDEN;
    r20_support: bool = false; "R20Support"; WorldSettings; java Some("R20Support"); bedrock Some("R20Support"); jtype None; btype None; flags CUSTOM | HIDDEN;
    r21_support: bool = false; "R21Support"; WorldSettings; java Some("R21Support"); bedrock Some("R21Support"); jtype None; btype None; flags CUSTOM | HIDDEN;
    winter_drop2024: bool = false; "WinterDrop2024"; WorldSettings; java Some("WinterDrop2024"); bedrock Some("WinterDrop2024"); jtype None; btype None; flags CUSTOM | HIDDEN;
    summer_drop2025: bool = false; "SummerDrop2025"; WorldSettings; java Some("SummerDrop2025"); bedrock Some("SummerDrop2025"); jtype None; btype None; flags CUSTOM | HIDDEN;
    autumn_drop2025: bool = false; "AutumnDrop2025"; WorldSettings; java Some("AutumnDrop2025"); bedrock Some("AutumnDrop2025"); jtype None; btype None; flags CUSTOM | HIDDEN;
    summer_drop2026: bool = false; "SummerDrop2026"; WorldSettings; java Some("SummerDrop2026"); bedrock Some("SummerDrop2026"); jtype None; btype None; flags CUSTOM | HIDDEN;
    autumn_drop2026: bool = false; "AutumnDrop2026"; WorldSettings; java Some("AutumnDrop2026"); bedrock Some("AutumnDrop2026"); jtype None; btype None; flags CUSTOM | HIDDEN;
    winter_drop2026: bool = false; "WinterDrop2026"; WorldSettings; java Some("WinterDrop2026"); bedrock Some("WinterDrop2026"); jtype None; btype None; flags CUSTOM | HIDDEN;
    falldamage: bool = true; "falldamage"; GameRules; java None; bedrock Some("falldamage"); jtype None; btype None; flags 0;
    firedamage: bool = true; "firedamage"; GameRules; java None; bedrock Some("firedamage"); jtype None; btype None; flags 0;
    force_game_type: bool = false; "ForceGameType"; GameRules; java None; bedrock Some("ForceGameType"); jtype None; btype None; flags 0;
    functioncommandlimit: i32 = 10000; "functioncommandlimit"; GameRules; java None; bedrock Some("functioncommandlimit"); jtype None; btype None; flags 0;
    game_type: i32 = 0; "GameType"; WorldSettings; java Some("GameType"); bedrock Some("GameType"); jtype None; btype None; flags 0;
    generator_type: GeneratorType = GeneratorType::Void; "GeneratorType"; WorldSettings; java Some("GeneratorType"); bedrock Some("GeneratorType"); jtype None; btype None; flags CUSTOM;
    has_been_loaded_in_creative: bool = true; "hasBeenLoadedInCreative"; Restrictions; java None; bedrock Some("hasBeenLoadedInCreative"); jtype None; btype None; flags 0;
    has_locked_behavior_pack: bool = false; "hasLockedBehaviorPack"; Restrictions; java None; bedrock Some("hasLockedBehaviorPack"); jtype None; btype None; flags 0;
    has_locked_resource_pack: bool = false; "hasLockedResourcePack"; Restrictions; java None; bedrock Some("hasLockedResourcePack"); jtype None; btype None; flags 0;
    immutable_world: bool = false; "immutableWorld"; Restrictions; java None; bedrock Some("immutableWorld"); jtype None; btype None; flags 0;
    is_from_locked_template: bool = false; "isFromLockedTemplate"; Restrictions; java None; bedrock Some("isFromLockedTemplate"); jtype None; btype None; flags 0;
    is_from_world_template: bool = false; "isFromWorldTemplate"; Restrictions; java None; bedrock Some("isFromWorldTemplate"); jtype None; btype None; flags 0;
    is_world_template_option_locked: bool = false; "isWorldTemplateOptionLocked"; Restrictions; java None; bedrock Some("isWorldTemplateOptionLocked"); jtype None; btype None; flags 0;
    keepinventory: bool = false; "keepinventory"; GameRules; java Some("GameRules.keepInventory"); bedrock Some("keepinventory"); jtype None; btype None; flags 0;
    lan_broadcast: bool = true; "LANBroadcast"; Misc; java None; bedrock Some("LANBroadcast"); jtype None; btype None; flags 0;
    lan_broadcast_intent: bool = true; "LANBroadcastIntent"; Misc; java None; bedrock Some("LANBroadcastIntent"); jtype None; btype None; flags 0;
    level_name: String = "Converted World".to_owned(); "LevelName"; WorldSettings; java Some("LevelName"); bedrock Some("LevelName"); jtype None; btype None; flags 0;
    lightning_level: f32 = 0.0; "lightningLevel"; Weather; java None; bedrock Some("lightningLevel"); jtype None; btype None; flags 0;
    lightning_time: i32 = 96000; "lightningTime"; Weather; java None; bedrock Some("lightningTime"); jtype None; btype None; flags 0;
    limited_world_origin_x: i32 = 0; "LimitedWorldOriginX"; Restrictions; java None; bedrock Some("LimitedWorldOriginX"); jtype None; btype None; flags 0;
    limited_world_origin_y: i32 = 32767; "LimitedWorldOriginY"; Restrictions; java None; bedrock Some("LimitedWorldOriginY"); jtype None; btype None; flags 0;
    limited_world_origin_z: i32 = 0; "LimitedWorldOriginZ"; Restrictions; java None; bedrock Some("LimitedWorldOriginZ"); jtype None; btype None; flags 0;
    mobgriefing: bool = true; "mobgriefing"; GameRules; java Some("GameRules.mobGriefing"); bedrock Some("mobgriefing"); jtype None; btype None; flags 0;
    multiplayer_game: bool = true; "MultiplayerGame"; Misc; java None; bedrock Some("MultiplayerGame"); jtype None; btype None; flags 0;
    multiplayer_game_intent: bool = true; "MultiplayerGameIntent"; Misc; java None; bedrock Some("MultiplayerGameIntent"); jtype None; btype None; flags 0;
    naturalregeneration: bool = true; "naturalregeneration"; GameRules; java Some("GameRules.naturalRegeneration"); bedrock Some("naturalregeneration"); jtype None; btype None; flags 0;
    nether_scale: i32 = 8; "NetherScale"; WorldSettings; java None; bedrock Some("NetherScale"); jtype None; btype None; flags 0;
    player_permissions_level: i32 = 2; "playerPermissionsLevel"; Restrictions; java None; bedrock Some("playerPermissionsLevel"); jtype None; btype None; flags CUSTOM;
    rain_level: f32 = 0.0; "rainLevel"; Weather; java None; bedrock Some("rainLevel"); jtype None; btype None; flags 0;
    rain_time: i32 = 48000; "rainTime"; Weather; java Some("rainTime"); bedrock Some("rainTime"); jtype None; btype None; flags 0;
    random_seed: String = "1234".to_owned(); "RandomSeed"; WorldSettings; java Some("RandomSeed"); bedrock Some("RandomSeed"); jtype None; btype None; flags CUSTOM;
    randomtickspeed: i32 = 1; "randomtickspeed"; GameRules; java Some("GameRules.randomTickSpeed"); bedrock Some("randomtickspeed"); jtype None; btype None; flags 0;
    sendcommandfeedback: bool = true; "sendcommandfeedback"; GameRules; java Some("GameRules.sendCommandFeedback"); bedrock Some("sendcommandfeedback"); jtype None; btype None; flags 0;
    server_chunk_tick_range: i32 = 10; "serverChunkTickRange"; Misc; java None; bedrock Some("serverChunkTickRange"); jtype None; btype None; flags 0;
    showcoordinates: bool = false; "showcoordinates"; GameRules; java None; bedrock Some("showcoordinates"); jtype None; btype None; flags 0;
    showdeathmessages: bool = true; "showdeathmessages"; GameRules; java Some("GameRules.showDeathMessages"); bedrock Some("showdeathmessages"); jtype None; btype None; flags 0;
    spawn_mobs: bool = true; "spawnMobs"; WorldSettings; java None; bedrock Some("spawnMobs"); jtype None; btype None; flags 0;
    spawn_x: i32 = 0; "SpawnX"; WorldSettings; java Some("SpawnX"); bedrock Some("SpawnX"); jtype None; btype None; flags 0;
    spawn_y: i32 = 32767; "SpawnY"; WorldSettings; java Some("SpawnY"); bedrock Some("SpawnY"); jtype None; btype None; flags 0;
    spawn_z: i32 = 0; "SpawnZ"; WorldSettings; java Some("SpawnZ"); bedrock Some("SpawnZ"); jtype None; btype None; flags 0;
    start_with_map_enabled: bool = false; "startWithMapEnabled"; Misc; java None; bedrock Some("startWithMapEnabled"); jtype None; btype None; flags 0;
    texture_packs_required: bool = false; "texturePacksRequired"; Restrictions; java None; bedrock Some("texturePacksRequired"); jtype None; btype None; flags 0;
    time: i64 = 0; "Time"; WorldSettings; java Some("Time"); bedrock Some("Time"); jtype None; btype None; flags 0;
    tntexplodes: bool = true; "tntexplodes"; GameRules; java None; bedrock Some("tntexplodes"); jtype None; btype None; flags 0;
    use_msa_gamertags_only: bool = false; "useMsaGamertagsOnly"; Misc; java None; bedrock Some("useMsaGamertagsOnly"); jtype None; btype None; flags 0;
    world_start_count: i64 = 0; "worldStartCount"; Misc; java None; bedrock Some("worldStartCount"); jtype None; btype None; flags 0;
    xbl_broadcast_intent: i32 = 3; "XBLBroadcastIntent"; Misc; java None; bedrock Some("XBLBroadcastIntent"); jtype None; btype None; flags 0;
    announce_advancements: bool = true; "announceAdvancements"; GameRules; java Some("GameRules.announceAdvancements"); bedrock None; jtype None; btype None; flags 0;
    disable_elytra_movement_check: bool = false; "disableElytraMovementCheck"; GameRules; java Some("GameRules.disableElytraMovementCheck"); bedrock None; jtype None; btype None; flags 0;
    do_limited_crafting: bool = false; "doLimitedCrafting"; GameRules; java Some("GameRules.doLimitedCrafting"); bedrock None; jtype None; btype None; flags 0;
    log_admin_commands: bool = true; "logAdminCommands"; GameRules; java Some("GameRules.logAdminCommands"); bedrock None; jtype None; btype None; flags 0;
    reduced_debug_info: bool = false; "reducedDebugInfo"; GameRules; java Some("GameRules.reducedDebugInfo"); bedrock None; jtype None; btype None; flags 0;
    spectators_generate_chunks: bool = true; "spectatorsGenerateChunks"; GameRules; java Some("GameRules.spectatorsGenerateChunks"); bedrock None; jtype None; btype None; flags 0;
    spawnradius: i32 = 10; "spawnradius"; GameRules; java Some("GameRules.spawnRadius"); bedrock Some("spawnradius"); jtype None; btype None; flags 0;
    max_entity_cramming: i32 = 24; "maxEntityCramming"; GameRules; java Some("GameRules.maxEntityCramming"); bedrock None; jtype None; btype None; flags 0;
    max_command_chain_length: i32 = 65535; "maxCommandChainLength"; GameRules; java Some("GameRules.maxCommandChainLength"); bedrock Some("maxcommandchainlength"); jtype None; btype None; flags 0;
    game_loop_function: String = "-".to_owned(); "gameLoopFunction"; GameRules; java Some("GameRules.gameLoopFunction"); bedrock None; jtype None; btype None; flags 0;
    border_center_x: f64 = 0.0; "BorderCenterX"; WorldSettings; java Some("BorderCenterX"); bedrock None; jtype None; btype None; flags 0;
    border_center_z: f64 = 0.0; "BorderCenterZ"; WorldSettings; java Some("BorderCenterZ"); bedrock None; jtype None; btype None; flags 0;
    border_damage_per_block: f64 = 0.2; "BorderDamagePerBlock"; WorldSettings; java Some("BorderDamagePerBlock"); bedrock None; jtype None; btype None; flags 0;
    border_safe_zone: f64 = 5.0; "BorderSafeZone"; WorldSettings; java Some("BorderSafeZone"); bedrock None; jtype None; btype None; flags 0;
    border_size: f64 = 60000000.0; "BorderSize"; WorldSettings; java Some("BorderSize"); bedrock None; jtype None; btype None; flags 0;
    border_size_lerp_target: f64 = 60000000.0; "BorderSizeLerpTarget"; WorldSettings; java Some("BorderSizeLerpTarget"); bedrock None; jtype None; btype None; flags 0;
    border_size_lerp_time: i64 = 0; "BorderSizeLerpTime"; WorldSettings; java Some("BorderSizeLerpTime"); bedrock None; jtype None; btype None; flags 0;
    border_warning_blocks: f64 = 5.0; "BorderWarningBlocks"; WorldSettings; java Some("BorderWarningBlocks"); bedrock None; jtype None; btype None; flags 0;
    border_warning_time: f64 = 15.0; "BorderWarningTime"; WorldSettings; java Some("BorderWarningTime"); bedrock None; jtype None; btype None; flags 0;
    hardcore: bool = false; "hardcore"; WorldSettings; java Some("hardcore"); bedrock Some("IsHardcore"); jtype None; btype None; flags 0;
    difficulty_locked: bool = false; "DifficultyLocked"; Restrictions; java Some("DifficultyLocked"); bedrock None; jtype None; btype None; flags 0;
    raining: bool = false; "raining"; Weather; java Some("raining"); bedrock None; jtype None; btype None; flags 0;
    thundering: bool = false; "thundering"; Weather; java Some("thundering"); bedrock None; jtype None; btype None; flags 0;
    map_features: bool = true; "MapFeatures"; WorldSettings; java Some("MapFeatures"); bedrock None; jtype None; btype None; flags 0;
    initialized: bool = true; "initialized"; WorldSettings; java Some("initialized"); bedrock None; jtype None; btype None; flags 0;
    thunder_time: i32 = 96256; "thunderTime"; Weather; java Some("thunderTime"); bedrock None; jtype None; btype None; flags 0;
    clear_weather_time: i32 = 0; "clearWeatherTime"; Weather; java Some("clearWeatherTime"); bedrock None; jtype None; btype None; flags 0;
    day_time: i64 = 315; "DayTime"; Weather; java Some("DayTime"); bedrock None; jtype None; btype None; flags 0;
}

/// One setting of the descriptive listing.
#[derive(Clone, Debug, Serialize)]
pub struct DescribedSetting {
    pub name: &'static str,
    pub bedrock: bool,
    pub java: bool,
    #[serde(rename = "type")]
    pub type_name: &'static str,
    pub value: serde_json::Value,
}

/// A category of the descriptive listing (`toDescriptiveJSON`).
#[derive(Clone, Debug, Serialize)]
pub struct DescribedCategory {
    pub name: &'static str,
    pub settings: Vec<DescribedSetting>,
}

impl LevelSettings {
    /// The field table entry named `name` (Chunker's name).
    pub fn field(name: &str) -> Option<&'static FieldDesc> {
        FIELDS.iter().find(|f| f.name == name)
    }

    /// Gets a value by Chunker's field name.
    pub fn get(&self, name: &str) -> Option<SettingValue> {
        Self::field(name).map(|f| (f.get)(self))
    }

    /// Sets a value by Chunker's field name (converted to the field type like Chunker does).
    pub fn set(&mut self, name: &str, value: SettingValue) -> Result<(), String> {
        let field = Self::field(name).ok_or_else(|| format!("unknown setting {name}"))?;
        let value = convert_value(&value, field.kind)?;
        if (field.set)(self, value) { Ok(()) } else { Err(format!("could not set {name}")) }
    }

    /// Reads the settings from a level.dat root (`fromNBT`). Problems keep the default and are logged.
    pub fn from_nbt(root: &Compound, io: &dyn SettingsIo) -> LevelSettings {
        let edition = io.encoding_type();
        let mut settings = LevelSettings::default();
        for field in FIELDS {
            let Some(path) = field.path(edition) else { continue };
            if field.is_custom() {
                if let Some(value) = io.read_custom(root, &mut settings, path, field.kind) {
                    if !(field.set)(&mut settings, value.clone()) {
                        io.log_non_fatal(format!("Could not set property {} Value: {value:?}", field.name));
                    }
                }
                continue;
            }
            // Walk dotted names into nested compounds; a missing compound skips the field.
            let mut compound = root;
            let mut name = path;
            let mut found = true;
            while let Some((head, rest)) = name.split_once('.') {
                match compound.0.get(head) {
                    Some(Tag::Compound(inner)) => compound = inner,
                    _ => {
                        found = false;
                        break;
                    }
                }
                name = rest;
            }
            if !found {
                continue;
            }
            let Some(tag) = compound.0.get(name) else { continue };
            let converted = match SettingValue::from_tag(tag) {
                Some(value) => convert_value(&value, field.kind),
                None if field.kind == SettingKind::Str => Ok(SettingValue::Str(format!("{tag:?}"))),
                None => Err(format!("No valid conversion method for {tag:?} to {:?}", field.kind)),
            };
            match converted {
                Ok(value) => {
                    if !(field.set)(&mut settings, value) {
                        io.log_non_fatal(format!("Could not set property {} Tag: {tag:?}", field.name));
                    }
                }
                Err(error) => io.log_non_fatal(format!("Could not set property {} Tag: {tag:?}: {error}", field.name)),
            }
        }
        settings
    }

    /// Writes the settings into a level.dat root (`toNBT`).
    pub fn to_nbt(&self, output: &mut Compound, io: &dyn SettingsIo) {
        let edition = io.encoding_type();
        for field in FIELDS {
            let Some(path) = field.path(edition) else { continue };
            let mut kind = field.output_kind(edition);
            if path.starts_with("GameRules.") {
                kind = SettingKind::Str;
            }
            let value = (field.get)(self);
            if field.is_custom() {
                io.write_custom(self, output, path, &value);
                continue;
            }
            let mut compound = &mut *output;
            let mut name = path;
            while let Some((head, rest)) = name.split_once('.') {
                let entry = compound.0.entry(head.to_owned()).or_insert_with(|| Tag::Compound(Compound::new()));
                if !matches!(entry, Tag::Compound(_)) {
                    *entry = Tag::Compound(Compound::new());
                }
                let Tag::Compound(inner) = entry else { unreachable!() };
                compound = inner;
                name = rest;
            }
            let target = match kind {
                // A boolean is stored as a byte.
                SettingKind::Bool => SettingKind::Byte,
                other => other,
            };
            match convert_value(&value, target).ok().and_then(|v| v.to_tag()) {
                Some(tag) => {
                    compound.0.insert(name.to_owned(), tag);
                }
                None => io.log_non_fatal(format!("Unable to write type {name} with type {kind:?}")),
            }
        }
    }

    /// The settings as JSON (`toJSON`).
    pub fn to_json(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        }
    }

    /// Settings from JSON; missing keys keep their default (`fromJSON`).
    pub fn from_json(json: &serde_json::Map<String, serde_json::Value>) -> Result<LevelSettings, serde_json::Error> {
        serde_json::from_value(serde_json::Value::Object(json.clone()))
    }

    /// Applies a shallow JSON overlay (the `changedSettings` of a conversion request).
    pub fn with_overrides(
        &self,
        overrides: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<LevelSettings, serde_json::Error> {
        let mut base = self.to_json();
        for (key, value) in overrides {
            base.insert(key.clone(), value.clone());
        }
        LevelSettings::from_json(&base)
    }

    /// The descriptive listing for a UI (`toDescriptiveJSON`): non-hidden settings grouped by category in enum order.
    pub fn descriptive(&self) -> Vec<DescribedCategory> {
        let mut out: Vec<DescribedCategory> = Vec::new();
        for category in [
            Category::WorldSettings,
            Category::GameRules,
            Category::Restrictions,
            Category::Weather,
            Category::Misc,
            Category::Unknown,
        ] {
            let settings: Vec<DescribedSetting> = FIELDS
                .iter()
                .filter(|f| f.category == category && !f.is_hidden() && (f.java.is_some() || f.bedrock.is_some()))
                .map(|f| DescribedSetting {
                    name: f.name,
                    bedrock: f.bedrock.is_some(),
                    java: f.java.is_some(),
                    type_name: f.kind.label(),
                    value: (f.get)(self).to_json(),
                })
                .collect();
            if !settings.is_empty() {
                out.push(DescribedCategory { name: category.name(), settings });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;

    use super::*;

    struct Io {
        edition: EncodingType,
        log: Mutex<Vec<String>>,
        seed: Option<String>,
        written: Mutex<Vec<(String, SettingValue)>>,
    }

    impl Io {
        fn new(edition: EncodingType) -> Io {
            Io { edition, log: Mutex::new(Vec::new()), seed: None, written: Mutex::new(Vec::new()) }
        }
    }

    impl SettingsIo for Io {
        fn encoding_type(&self) -> EncodingType {
            self.edition
        }
        fn read_custom(&self, _: &Compound, _: &mut LevelSettings, name: &str, _: SettingKind) -> Option<SettingValue> {
            if name == "RandomSeed" { self.seed.clone().map(SettingValue::Str) } else { None }
        }
        fn write_custom(&self, _: &LevelSettings, _: &mut Compound, name: &str, value: &SettingValue) {
            self.written.lock().unwrap().push((name.to_owned(), value.clone()));
        }
        fn log_non_fatal(&self, message: String) {
            self.log.lock().unwrap().push(message);
        }
    }

    fn compound(entries: Vec<(&str, Tag)>) -> Compound {
        let mut c = Compound::new();
        for (k, v) in entries {
            c.insert(k, v);
        }
        c
    }

    #[test]
    fn field_table_is_complete() {
        assert_eq!(FIELDS.len(), 106);
        let names: HashSet<_> = FIELDS.iter().map(|f| f.name).collect();
        assert_eq!(names.len(), 106);
        let rust_names: HashSet<_> = FIELDS.iter().map(|f| f.rust_name).collect();
        assert_eq!(rust_names.len(), 106);
        assert_eq!(FIELDS.iter().filter(|f| f.java.is_some()).count(), 64);
        assert_eq!(FIELDS.iter().filter(|f| f.bedrock.is_some()).count(), 81);
        assert_eq!(FIELDS.iter().filter(|f| f.is_custom()).count(), 13);
        assert_eq!(FIELDS.iter().filter(|f| f.is_hidden()).count(), 10);
        assert_eq!(FIELDS[0].name, "bonusChestEnabled");
        assert_eq!(FIELDS[105].name, "DayTime");
    }

    #[test]
    fn defaults_match_chunker() {
        let s = LevelSettings::default();
        assert_eq!(s.level_name, "Converted World");
        assert_eq!(s.random_seed, "1234");
        assert_eq!((s.spawn_x, s.spawn_y, s.spawn_z), (0, 32767, 0));
        assert_eq!(s.generator_type, GeneratorType::Void);
        assert_eq!((s.difficulty, s.game_type, s.time, s.day_time), (1, 0, 0, 315));
        assert_eq!((s.rain_time, s.thunder_time, s.lightning_time), (48000, 96256, 96000));
        assert_eq!((s.functioncommandlimit, s.max_command_chain_length, s.spawnradius), (10000, 65535, 10));
        assert_eq!((s.border_size, s.border_size_lerp_target, s.border_damage_per_block), (6e7, 6e7, 0.2));
        assert_eq!((s.limited_world_origin_y, s.nether_scale, s.xbl_broadcast_intent), (32767, 8, 3));
        assert_eq!((s.max_entity_cramming, s.player_permissions_level, s.flat_world_version), (24, 2, 1));
        assert_eq!(s.game_loop_function, "-");
        assert!(s.commands_enabled && s.commandblockoutput && !s.keepinventory);
        assert!(s.has_been_loaded_in_creative && s.lan_broadcast && s.spawn_mobs && s.map_features && s.initialized);
        assert!(!s.hardcore && !s.raining && !s.bonus_chest_enabled && !s.r20_support);
    }

    #[test]
    fn get_set_by_name() {
        let mut s = LevelSettings::default();
        assert_eq!(s.get("SpawnY"), Some(SettingValue::Int(32767)));
        s.set("SpawnY", SettingValue::Long(70)).unwrap();
        assert_eq!(s.spawn_y, 70);
        s.set("hardcore", SettingValue::Byte(1)).unwrap();
        assert!(s.hardcore);
        s.set("rainTime", SettingValue::Str("12".into())).unwrap();
        assert_eq!(s.rain_time, 12);
        assert!(s.set("rainTime", SettingValue::Str("soon".into())).is_err());
        assert!(s.set("nope", SettingValue::Int(1)).is_err());
        assert!(s.set("GeneratorType", SettingValue::Int(1)).is_err());
        s.set("GeneratorType", SettingValue::Generator(GeneratorType::Flat)).unwrap();
        assert_eq!(s.generator_type, GeneratorType::Flat);
    }

    #[test]
    fn convert_value_follows_chunker() {
        use SettingKind as K;
        use SettingValue as V;
        assert_eq!(convert_value(&V::Byte(2), K::Bool).unwrap(), V::Bool(true));
        assert_eq!(convert_value(&V::Int(256), K::Bool).unwrap(), V::Bool(false)); // byteValue() == 0
        assert_eq!(convert_value(&V::Double(2.9), K::Int).unwrap(), V::Int(2));
        assert_eq!(convert_value(&V::Long(300), K::Byte).unwrap(), V::Byte(44));
        assert_eq!(convert_value(&V::Int(7), K::Long).unwrap(), V::Long(7));
        assert_eq!(convert_value(&V::Int(7), K::Float).unwrap(), V::Float(7.0));
        assert_eq!(convert_value(&V::Bool(true), K::Byte).unwrap(), V::Byte(1));
        assert_eq!(convert_value(&V::Str("TRUE".into()), K::Bool).unwrap(), V::Bool(true));
        assert_eq!(convert_value(&V::Str("yes".into()), K::Bool).unwrap(), V::Bool(false));
        assert_eq!(convert_value(&V::Str("true".into()), K::Int).unwrap(), V::Int(1));
        assert_eq!(convert_value(&V::Str("false".into()), K::Int).unwrap(), V::Int(0));
        assert_eq!(convert_value(&V::Str("-5".into()), K::Int).unwrap(), V::Int(-5));
        assert_eq!(convert_value(&V::Str("0.5".into()), K::Double).unwrap(), V::Double(0.5));
        assert!(convert_value(&V::Str("x".into()), K::Int).is_err());
        assert!(convert_value(&V::Str("300".into()), K::Byte).is_err());
        assert_eq!(convert_value(&V::Bool(true), K::Str).unwrap(), V::Str("true".into()));
        assert_eq!(convert_value(&V::Float(1.0), K::Str).unwrap(), V::Str("1.0".into()));
        assert_eq!(convert_value(&V::Generator(GeneratorType::Flat), K::Str).unwrap(), V::Str("FLAT".into()));
        assert!(convert_value(&V::Bool(true), K::Int).is_err());
        assert!(convert_value(&V::Int(1), K::Generator).is_err());
    }

    #[test]
    fn java_from_nbt() {
        let root = compound(vec![
            ("LevelName", Tag::String("My World".into())),
            ("Difficulty", Tag::Byte(3)),
            ("allowCommands", Tag::Byte(0)),
            ("Time", Tag::Long(1234)),
            ("SpawnX", Tag::Int(-5)),
            ("BorderSize", Tag::Double(1000.0)),
            ("hardcore", Tag::Byte(1)),
            ("RandomSeed", Tag::Long(99)), // custom: ignored unless the reader supplies it
            (
                "GameRules",
                Tag::Compound(compound(vec![
                    ("doFireTick", Tag::String("false".into())),
                    ("randomTickSpeed", Tag::String("3".into())),
                    ("spawnRadius", Tag::String("0".into())),
                    ("gameLoopFunction", Tag::String("a:b".into())),
                ])),
            ),
        ]);
        let mut io = Io::new(EncodingType::Java);
        io.seed = Some("424242".into());
        let s = LevelSettings::from_nbt(&root, &io);
        assert_eq!(s.level_name, "My World");
        assert_eq!((s.difficulty, s.commands_enabled, s.time, s.spawn_x), (3, false, 1234, -5));
        assert_eq!(s.border_size, 1000.0);
        assert!(s.hardcore);
        assert!(!s.dofiretick);
        assert_eq!((s.randomtickspeed, s.spawnradius), (3, 0));
        assert_eq!(s.game_loop_function, "a:b");
        assert_eq!(s.random_seed, "424242");
        // Bedrock-only fields are not read from a Java root.
        assert!(s.has_been_loaded_in_creative);
        assert!(io.log.lock().unwrap().is_empty(), "{:?}", io.log.lock().unwrap());
    }

    #[test]
    fn missing_game_rules_keep_defaults_and_bad_tags_are_logged() {
        let root = compound(vec![("Time", Tag::List(vec![Tag::Int(1)])), ("SpawnZ", Tag::String("seven".into()))]);
        let io = Io::new(EncodingType::Java);
        let s = LevelSettings::from_nbt(&root, &io);
        assert_eq!(s.time, 0);
        assert_eq!(s.spawn_z, 0);
        assert!(s.dofiretick);
        let log = io.log.lock().unwrap();
        assert_eq!(log.len(), 2);
        assert!(log.iter().any(|l| l.contains("Time")) && log.iter().any(|l| l.contains("SpawnZ")));
    }

    #[test]
    fn java_to_nbt() {
        let mut s = LevelSettings::default();
        s.dofiretick = false;
        s.randomtickspeed = 4;
        s.difficulty = 2;
        s.level_name = "Out".into();
        let io = Io::new(EncodingType::Java);
        let mut out = Compound::new();
        s.to_nbt(&mut out, &io);
        assert_eq!(out.get("Difficulty"), Some(&Tag::Byte(2)));
        assert_eq!(out.get("LevelName"), Some(&Tag::String("Out".into())));
        assert_eq!(out.get("allowCommands"), Some(&Tag::Byte(1)));
        assert_eq!(out.get("Time"), Some(&Tag::Long(0)));
        assert_eq!(out.get("BorderSize"), Some(&Tag::Double(6e7)));
        let Some(Tag::Compound(rules)) = out.get("GameRules") else { panic!("no GameRules") };
        assert_eq!(rules.get("doFireTick"), Some(&Tag::String("false".into())));
        assert_eq!(rules.get("randomTickSpeed"), Some(&Tag::String("4".into())));
        assert_eq!(rules.get("spawnRadius"), Some(&Tag::String("10".into())));
        // Custom fields go through the hook with their full path; Bedrock-only fields are absent.
        let written = io.written.lock().unwrap();
        assert!(written.iter().any(|(n, v)| n == "RandomSeed" && *v == SettingValue::Str("1234".into())));
        assert_eq!(written.len(), 12); // playerPermissionsLevel is Bedrock only
        assert!(out.get("lightningTime").is_none());
        assert!(io.log.lock().unwrap().is_empty());
    }

    #[test]
    fn bedrock_round_trip() {
        let mut s = LevelSettings::default();
        s.level_name = "Bedrock".into();
        s.lightning_level = 0.5;
        s.rain_level = 1.0;
        s.dodaylightcycle = false;
        s.max_command_chain_length = 1234;
        s.hardcore = true;
        s.world_start_count = 77;
        s.edu_level = 2;
        s.nether_scale = 4;
        let io = Io::new(EncodingType::Bedrock);
        let mut out = Compound::new();
        s.to_nbt(&mut out, &io);
        assert_eq!(out.get("dodaylightcycle"), Some(&Tag::Byte(0)));
        assert_eq!(out.get("lightningLevel"), Some(&Tag::Float(0.5)));
        assert_eq!(out.get("IsHardcore"), Some(&Tag::Byte(1)));
        assert_eq!(out.get("maxcommandchainlength"), Some(&Tag::Int(1234)));
        assert_eq!(out.get("worldStartCount"), Some(&Tag::Long(77)));
        assert_eq!(out.get("eduLevel"), Some(&Tag::Byte(2)));
        assert_eq!(out.get("NetherScale"), Some(&Tag::Int(4)));
        assert_eq!(out.get("Difficulty"), Some(&Tag::Int(1)));
        assert!(out.get("GameRules").is_none());
        let back = LevelSettings::from_nbt(&out, &io);
        // The custom fields are not written without a hook, so compare after copying them over.
        let mut expected = s.clone();
        for field in FIELDS.iter().filter(|f| f.is_custom()) {
            (field.set)(&mut expected, (field.get)(&LevelSettings::default()));
        }
        assert_eq!(back, expected);
    }

    #[test]
    fn json_round_trip_and_overlay() {
        let s = LevelSettings::default();
        let json = s.to_json();
        assert_eq!(json.len(), 106);
        assert_eq!(json["LevelName"], "Converted World");
        assert_eq!(json["GeneratorType"], "VOID");
        assert_eq!(json["BorderSize"], 6e7);
        assert_eq!(LevelSettings::from_json(&json).unwrap(), s);
        let overlay = serde_json::json!({"LevelName": "X", "GeneratorType": "FLAT", "doLimitedCrafting": true});
        let out = s.with_overrides(overlay.as_object().unwrap()).unwrap();
        assert_eq!(out.level_name, "X");
        assert_eq!(out.generator_type, GeneratorType::Flat);
        assert!(out.do_limited_crafting);
        // Missing keys keep defaults; wrong types are errors.
        let partial = serde_json::json!({"SpawnX": 3});
        assert_eq!(LevelSettings::from_json(partial.as_object().unwrap()).unwrap().spawn_x, 3);
        let bad = serde_json::json!({"SpawnX": "three"});
        assert!(LevelSettings::from_json(bad.as_object().unwrap()).is_err());
    }

    #[test]
    fn descriptive_listing() {
        let listing = LevelSettings::default().descriptive();
        let names: Vec<_> = listing.iter().map(|c| c.name).collect();
        assert_eq!(names, ["World Settings", "Game Rules", "Restrictions", "Weather", "Misc"]);
        let all: Vec<_> = listing.iter().flat_map(|c| c.settings.iter()).collect();
        assert_eq!(all.len(), 106 - 10);
        assert!(all.iter().all(|s| s.name != "R20Support"));
        let generator = all.iter().find(|s| s.name == "GeneratorType").unwrap();
        assert_eq!((generator.type_name, generator.java, generator.bedrock), ("ChunkerGeneratorType", true, true));
        assert_eq!(generator.value, "VOID");
        let time = all.iter().find(|s| s.name == "Time").unwrap();
        assert_eq!(time.type_name, "Int64");
        let json = serde_json::to_value(&listing[0].settings[0]).unwrap();
        assert!(json.get("type").is_some());
    }
}
