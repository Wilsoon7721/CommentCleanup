//! File icons: a built-in set that needs nothing installed, plus VS Code icon themes (SVG or font based) when an editor provides them.

use crate::vscode::{IconThemeEntry, parse_jsonc};
use iced::widget::svg;
use iced::{Color, Font};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub const BUILT_IN: &str = "Built-in";

/// Windows' own icon font, used for everything that isn't a language badge.
pub const SYMBOLS: Font = Font::with_name("Segoe MDL2 Assets");

pub mod glyph {
    pub const WINDOW_CLOSE: char = '\u{E8BB}';
    pub const WINDOW_MAXIMIZE: char = '\u{E922}';
    pub const WINDOW_MINIMIZE: char = '\u{E921}';
    pub const WINDOW_RESTORE: char = '\u{E923}';
    pub const CHECK: char = '\u{E73E}';
    pub const RETRY: char = '\u{E72C}';
    pub const CHEVRON_DOWN: char = '\u{E70D}';
    pub const CHEVRON_UP: char = '\u{E70E}';
    pub const SEARCH: char = '\u{E721}';
    pub const CHEVRON_LEFT: char = '\u{E76B}';
    pub const CHEVRON_RIGHT: char = '\u{E76C}';
    pub const CLOSE: char = '\u{E711}';
    pub const DELETE: char = '\u{E74D}';
    pub const FILE: char = '\u{E7C3}';
    pub const FIX: char = '\u{E90F}';
    pub const FIX_ALL: char = '\u{E771}';
    pub const FOLDER: char = '\u{E8B7}';
    pub const FOLDER_OPEN: char = '\u{E838}';
    pub const IGNORE: char = '\u{F140}';
    pub const IMAGE: char = '\u{E91B}';
    pub const INFO: char = '\u{E946}';
    pub const KEY: char = '\u{E8D7}';
    pub const LOCK: char = '\u{E72E}';
    pub const PACKAGE: char = '\u{E7B8}';
    pub const PLAY: char = '\u{E768}';
    pub const REWORD: char = '\u{E70F}';
    pub const SETTINGS: char = '\u{E713}';
    pub const SQUASH: char = '\u{E73F}';
    pub const STOP: char = '\u{E71A}';
    pub const UNDO: char = '\u{E7A7}';
}

#[derive(Clone, Debug)]
pub enum Icon {
    Badge { label: &'static str, color: Color },
    Glyph { glyph: char, color: Option<Color>, font: Font, size: f32 },
    Svg(svg::Handle),
}

const fn rgb(hex: u32) -> Color {
    Color {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

fn symbol(glyph: char, color: Option<Color>) -> Icon {
    Icon::Glyph { glyph, color, font: SYMBOLS, size: 14.0 }
}

const BY_NAME: &[(&str, &str, u32)] = &[
    ("package.json", "NP", 0xcb3837),
    ("package-lock.json", "NP", 0xcb3837),
    ("cargo.toml", "RS", 0xdea584),
    ("cargo.lock", "RS", 0xdea584),
    ("dockerfile", "DK", 0x2496ed),
    ("makefile", "MK", 0x6d8086),
    (".gitignore", "GI", 0xe8674b),
    (".gitattributes", "GI", 0xe8674b),
    (".gitmodules", "GI", 0xe8674b),
];

const BY_EXTENSION: &[(&str, &str, u32)] = &[
    ("astro", "AS", 0xff5d01),
    ("bat", ">_", 0x4d5a5e),
    ("c", "C", 0x5c6bc0),
    ("cc", "C+", 0x5c6bc0),
    ("cjs", "JS", 0xf1d83a),
    ("clj", "CL", 0x5881d8),
    ("cmd", ">_", 0x4d5a5e),
    ("cpp", "C+", 0x5c6bc0),
    ("cs", "C#", 0x68217a),
    ("css", "#", 0x2965f1),
    ("cts", "TS", 0x3178c6),
    ("dart", "DT", 0x00b4ab),
    ("ex", "EX", 0x6e4a7e),
    ("exs", "EX", 0x6e4a7e),
    ("go", "GO", 0x00add8),
    ("h", "H", 0x7986cb),
    ("hpp", "H+", 0x7986cb),
    ("hs", "HS", 0x5e5086),
    ("htm", "<>", 0xe44d26),
    ("html", "<>", 0xe44d26),
    ("java", "JV", 0xb07219),
    ("jl", "JL", 0x9558b2),
    ("js", "JS", 0xf1d83a),
    ("json", "{}", 0xcbcb41),
    ("json5", "{}", 0xcbcb41),
    ("jsonc", "{}", 0xcbcb41),
    ("jsx", "JX", 0x61dafb),
    ("kt", "KT", 0x7f52ff),
    ("kts", "KT", 0x7f52ff),
    ("less", "LS", 0x1d365d),
    ("lua", "LU", 0x000080),
    ("markdown", "MD", 0x519aba),
    ("md", "MD", 0x519aba),
    ("mdx", "MX", 0x519aba),
    ("mjs", "JS", 0xf1d83a),
    ("mts", "TS", 0x3178c6),
    ("pdf", "PD", 0xd32f2f),
    ("php", "PH", 0x777bb4),
    ("pl", "PL", 0x39457e),
    ("proto", "PB", 0x6d8086),
    ("prisma", "PR", 0x5a67d8),
    ("ps1", ">_", 0x012456),
    ("py", "PY", 0x3776ab),
    ("r", "R", 0x276dc3),
    ("rb", "RB", 0xcc342d),
    ("rs", "RS", 0xdea584),
    ("sass", "S", 0xc6538c),
    ("scala", "SC", 0xdc322f),
    ("scss", "S", 0xc6538c),
    ("sh", "$_", 0x4eaa25),
    ("bash", "$_", 0x4eaa25),
    ("zsh", "$_", 0x4eaa25),
    ("sql", "SQ", 0xe38c00),
    ("svelte", "SV", 0xff3e00),
    ("swift", "SW", 0xf05138),
    ("tf", "TF", 0x7b42bc),
    ("toml", "TL", 0x9c4221),
    ("ts", "TS", 0x3178c6),
    ("tsx", "TX", 0x61dafb),
    ("vue", "VU", 0x41b883),
    ("wasm", "WA", 0x654ff0),
    ("xml", "<>", 0xf29111),
    ("yaml", "YM", 0xcb4b4b),
    ("yml", "YM", 0xcb4b4b),
    ("zig", "ZG", 0xf7a41d),
];

const FOLDER_COLOR: Color = rgb(0xdcb67a);

fn folder_color(name: &str) -> Color {
    match name {
        "src" | "lib" | "app" | "apps" | "packages" => rgb(0x5c9fd6),
        "test" | "tests" | "__tests__" | "spec" => rgb(0x4db6ac),
        "node_modules" | "vendor" | "target" => rgb(0x7e9a5a),
        "dist" | "build" | "out" | "public" => rgb(0xd98c5f),
        ".vscode" | ".github" | ".turbo" | ".astro" | ".wrangler" => rgb(0x9a8fbf),
        _ => FOLDER_COLOR,
    }
}

/// The icon used when no icon theme is chosen, or the theme has nothing for this file.
pub fn built_in(name: &str, is_dir: bool, expanded: bool) -> Icon {
    let name = name.to_lowercase();
    if is_dir {
        let glyph = if expanded { glyph::FOLDER_OPEN } else { glyph::FOLDER };
        return symbol(glyph, Some(folder_color(&name)));
    }
    if let Some((_, label, color)) = BY_NAME.iter().find(|(n, ..)| *n == name) {
        return Icon::Badge { label, color: rgb(*color) };
    }
    if name.starts_with("tsconfig") && name.ends_with(".json") {
        return Icon::Badge { label: "TS", color: rgb(0x3178c6) };
    }
    if name.starts_with(".env") || name.ends_with(".vars") {
        return symbol(glyph::KEY, Some(rgb(0xe2c044)));
    }
    if name.starts_with("readme") {
        return symbol(glyph::INFO, Some(rgb(0x519aba)));
    }
    if name.starts_with("license") || name.starts_with("licence") {
        return symbol(glyph::LOCK, Some(rgb(0xcbcb41)));
    }
    let extension = name.rsplit_once('.').map_or("", |(_, e)| e);
    if let Some((_, label, color)) = BY_EXTENSION.iter().find(|(e, ..)| *e == extension) {
        return Icon::Badge { label, color: rgb(*color) };
    }
    match extension {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "avif" => {
            symbol(glyph::IMAGE, Some(rgb(0xa074c4)))
        }
        "zip" | "tar" | "gz" | "tgz" | "7z" | "rar" | "xz" => {
            symbol(glyph::PACKAGE, Some(rgb(0xc08b5c)))
        }
        "lock" => symbol(glyph::LOCK, Some(rgb(0x9aa5ae))),
        _ => symbol(glyph::FILE, None),
    }
}

#[derive(Default)]
struct Associations {
    file: Option<String>,
    folder: Option<String>,
    folder_expanded: Option<String>,
    file_extensions: HashMap<String, String>,
    file_names: HashMap<String, String>,
    folder_names: HashMap<String, String>,
    folder_names_expanded: HashMap<String, String>,
    language_ids: HashMap<String, String>,
}

impl Associations {
    fn parse(value: &Value) -> Self {
        let id = |key: &str| value[key].as_str().map(str::to_owned);
        let map = |key: &str| -> HashMap<String, String> {
            value[key]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| Some((k.to_lowercase(), v.as_str()?.to_owned())))
                .collect()
        };
        Self {
            file: id("file"),
            folder: id("folder"),
            folder_expanded: id("folderExpanded"),
            file_extensions: map("fileExtensions"),
            file_names: map("fileNames"),
            folder_names: map("folderNames"),
            folder_names_expanded: map("folderNamesExpanded"),
            language_ids: map("languageIds"),
        }
    }

    fn file_id(&self, name: &str, language: &str) -> Option<&String> {
        self.file_names
            .get(name)
            .or_else(|| {
                name.match_indices('.')
                    .find_map(|(i, _)| self.file_extensions.get(&name[i + 1..]))
            })
            .or_else(|| self.language_ids.get(language))
    }

    fn folder_id(&self, name: &str, expanded: bool) -> Option<&String> {
        if expanded {
            self.folder_names_expanded
                .get(name)
                .or_else(|| self.folder_names.get(name))
                .or(self.folder_expanded.as_ref())
                .or(self.folder.as_ref())
        } else {
            self.folder_names.get(name).or(self.folder.as_ref())
        }
    }
}

pub struct IconTheme {
    definitions: HashMap<String, Icon>,
    dark: Associations,
    light: Option<Associations>,
}

impl std::fmt::Debug for IconTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IconTheme({} icons)", self.definitions.len())
    }
}

/// A chosen icon theme and the font it needs registered, if any.
pub struct LoadedIcons {
    pub theme: IconTheme,
    pub font: Option<Vec<u8>>,
    pub uses_text_color: bool,
}

impl std::fmt::Debug for LoadedIcons {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.theme.fmt(f)
    }
}

impl IconTheme {
    pub fn load(entry: &IconThemeEntry, text_color: Color) -> Result<LoadedIcons, String> {
        let text = std::fs::read_to_string(&entry.path).map_err(|e| e.to_string())?;
        let json = parse_jsonc(&text)?;
        let dir = entry.path.parent().unwrap_or(Path::new("."));

        let mut font_bytes = None;
        let mut font = SYMBOLS;
        let mut size = 14.0;
        if let Some(path) = json["fonts"][0]["src"][0]["path"].as_str() {
            let raw = std::fs::read(dir.join(path)).map_err(|e| e.to_string())?;
            let sfnt = if raw.starts_with(b"wOFF") { woff_to_sfnt(&raw)? } else { raw };
            let family = font_family(&sfnt).ok_or("the icon font has no family name")?;
            font = Font::with_name(Box::leak(family.into_boxed_str()));
            font_bytes = Some(sfnt);
            if let Some(percent) = json["fonts"][0]["size"]
                .as_str()
                .and_then(|s| s.strip_suffix('%'))
                .and_then(|s| s.parse::<f32>().ok())
            {
                size = 13.0 * percent / 100.0;
            }
        }

        let uses_text_color = json["usesCurrentColor"].as_bool().unwrap_or(false);
        let [r, g, b, _] = text_color.into_rgba8();
        let text_hex = format!("#{r:02x}{g:02x}{b:02x}");

        let mut definitions = HashMap::new();
        for (id, definition) in json["iconDefinitions"].as_object().into_iter().flatten() {
            if let Some(path) = definition["iconPath"].as_str() {
                if path.to_lowercase().ends_with(".svg") {
                    let file = dir.join(path);
                    let handle = if uses_text_color {
                        let Ok(source) = std::fs::read_to_string(&file) else {
                            continue;
                        };
                        svg::Handle::from_memory(source.replace("currentColor", &text_hex).into_bytes())
                    } else {
                        svg::Handle::from_path(file)
                    };
                    definitions.insert(id.clone(), Icon::Svg(handle));
                }
            } else if let Some(glyph) = definition["fontCharacter"].as_str().and_then(font_character) {
                let color = definition["fontColor"].as_str().map(|hex| {
                    crate::viewer::color(crate::highlight::parse_hex(hex))
                });
                definitions.insert(id.clone(), Icon::Glyph { glyph, color, font, size });
            }
        }
        if definitions.is_empty() {
            return Err("this icon theme has no SVG or font icons".into());
        }
        Ok(LoadedIcons {
            theme: IconTheme {
                definitions,
                dark: Associations::parse(&json),
                light: json.get("light").map(Associations::parse),
            },
            font: font_bytes,
            uses_text_color,
        })
    }

    pub fn resolve(
        &self,
        name: &str,
        language: &str,
        is_dir: bool,
        expanded: bool,
        light: bool,
    ) -> Option<Icon> {
        let name = name.to_lowercase();
        let find = |layer: &Associations| {
            let id = if is_dir {
                layer.folder_id(&name, expanded)
            } else {
                layer.file_id(&name, language)
            };
            id.and_then(|id| self.definitions.get(id))
        };
        let default = |layer: &Associations| {
            layer
                .file
                .as_ref()
                .filter(|_| !is_dir)
                .and_then(|id| self.definitions.get(id))
        };
        let layers = [self.light.as_ref().filter(|_| light), Some(&self.dark)];
        let layers = || layers.iter().flatten().copied();
        layers()
            .find_map(find)
            .or_else(|| layers().find_map(default))
            .cloned()
    }
}

fn font_character(text: &str) -> Option<char> {
    match text.strip_prefix('\\') {
        Some(hex) => char::from_u32(u32::from_str_radix(hex, 16).ok()?),
        None => text.chars().next(),
    }
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Rebuilds the plain font inside a WOFF file, which the font loader can't read.
pub fn woff_to_sfnt(woff: &[u8]) -> Result<Vec<u8>, String> {
    const BAD: &str = "the icon font is damaged";
    let flavor = be32(woff, 4).ok_or(BAD)?;
    let count = be16(woff, 12).ok_or(BAD)? as usize;

    let mut tables = Vec::with_capacity(count);
    for index in 0..count {
        let entry = 44 + index * 20;
        let tag = woff.get(entry..entry + 4).ok_or(BAD)?;
        let offset = be32(woff, entry + 4).ok_or(BAD)? as usize;
        let compressed = be32(woff, entry + 8).ok_or(BAD)? as usize;
        let original = be32(woff, entry + 12).ok_or(BAD)? as usize;
        let checksum = be32(woff, entry + 16).ok_or(BAD)?;
        let data = woff.get(offset..offset + compressed).ok_or(BAD)?;
        let data = if compressed < original {
            miniz_oxide::inflate::decompress_to_vec_zlib(data).map_err(|_| BAD)?
        } else {
            data.to_vec()
        };
        tables.push((tag, checksum, data));
    }

    let exponent = (count.max(1) as u32).ilog2();
    let search_range = 16u16 << exponent;
    let mut out = Vec::new();
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(count as u16).to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&(exponent as u16).to_be_bytes());
    out.extend_from_slice(&((count as u16 * 16).wrapping_sub(search_range)).to_be_bytes());

    let mut offset = 12 + count * 16;
    for (tag, checksum, data) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, _, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Ok(out)
}

/// The family name stored in a font's `name` table.
fn font_family(sfnt: &[u8]) -> Option<String> {
    let count = be16(sfnt, 4)? as usize;
    let table = (0..count).find_map(|i| {
        let entry = 12 + i * 16;
        (sfnt.get(entry..entry + 4)? == b"name").then(|| be32(sfnt, entry + 8))?
    })? as usize;
    let records = be16(sfnt, table + 2)? as usize;
    let strings = table + be16(sfnt, table + 4)? as usize;
    for i in 0..records {
        let record = table + 6 + i * 12;
        if be16(sfnt, record + 6)? != 1 {
            continue;
        }
        let platform = be16(sfnt, record)?;
        let length = be16(sfnt, record + 8)? as usize;
        let start = strings + be16(sfnt, record + 10)? as usize;
        let bytes = sfnt.get(start..start + length)?;
        return Some(if platform == 1 {
            bytes.iter().map(|b| *b as char).collect()
        } else {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        });
    }
    None
}

/// `Built-in` plus every icon theme the installed editors provide.
pub fn names(entries: &[IconThemeEntry]) -> Vec<String> {
    std::iter::once(BUILT_IN.to_owned())
        .chain(entries.iter().map(|e| e.label.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seti_like() -> IconTheme {
        let glyph = Icon::Glyph { glyph: 'a', color: None, font: SYMBOLS, size: 14.0 };
        let definitions = ["ts", "dts", "pkg", "rust", "file", "dir", "dirOpen", "lightTs"]
            .into_iter()
            .map(|id| (id.to_owned(), glyph.clone()))
            .collect();
        IconTheme {
            definitions,
            dark: Associations::parse(&serde_json::json!({
                "file": "file",
                "folder": "dir",
                "folderExpanded": "dirOpen",
                "fileExtensions": { "ts": "ts", "d.ts": "dts" },
                "fileNames": { "Package.json": "pkg" },
                "languageIds": { "rust": "rust" },
            })),
            light: Some(Associations::parse(&serde_json::json!({
                "fileExtensions": { "ts": "lightTs" },
            }))),
        }
    }

    fn id_of(theme: &IconTheme, layer: &Associations, name: &str, language: &str) -> Option<String> {
        let _ = theme;
        layer.file_id(name, language).cloned()
    }

    #[test]
    fn longest_extension_and_exact_names_win() {
        let theme = seti_like();
        assert_eq!(id_of(&theme, &theme.dark, "a.d.ts", ""), Some("dts".into()));
        assert_eq!(id_of(&theme, &theme.dark, "a.ts", ""), Some("ts".into()));
        assert_eq!(id_of(&theme, &theme.dark, "package.json", ""), Some("pkg".into()));
        assert_eq!(id_of(&theme, &theme.dark, "lib.xyz", "rust"), Some("rust".into()));
        assert_eq!(id_of(&theme, &theme.dark, "lib.xyz", "other"), None);
    }

    #[test]
    fn unknown_files_get_the_default_and_folders_their_own() {
        let theme = seti_like();
        assert!(theme.resolve("notes.xyz", "plain", false, false, false).is_some());
        assert_eq!(theme.dark.folder_id("src", true), Some(&"dirOpen".to_owned()));
        assert_eq!(theme.dark.folder_id("src", false), Some(&"dir".to_owned()));
    }

    #[test]
    fn font_characters_parse() {
        assert_eq!(font_character("\\E001"), Some('\u{E001}'));
        assert_eq!(font_character("x"), Some('x'));
    }

    #[test]
    fn woff_tables_are_rebuilt_in_place() {
        let payload = b"abcde";
        let mut woff = Vec::new();
        woff.extend_from_slice(b"wOFF");
        woff.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        woff.extend_from_slice(&0u32.to_be_bytes());
        woff.extend_from_slice(&1u16.to_be_bytes());
        woff.resize(44, 0);
        woff.extend_from_slice(b"test");
        woff.extend_from_slice(&64u32.to_be_bytes());
        woff.extend_from_slice(&5u32.to_be_bytes());
        woff.extend_from_slice(&5u32.to_be_bytes());
        woff.extend_from_slice(&7u32.to_be_bytes());
        woff.extend_from_slice(payload);

        let sfnt = woff_to_sfnt(&woff).unwrap();
        assert_eq!(be32(&sfnt, 0), Some(0x0001_0000));
        assert_eq!(be16(&sfnt, 4), Some(1));
        assert_eq!(&sfnt[12..16], b"test");
        assert_eq!(be32(&sfnt, 16), Some(7));
        assert_eq!(be32(&sfnt, 20), Some(28));
        assert_eq!(&sfnt[28..33], payload);
        assert_eq!(sfnt.len() % 4, 0);
    }

    #[test]
    fn built_in_icons_cover_the_common_cases() {
        assert!(matches!(built_in("App.tsx", false, false), Icon::Badge { label: "TX", .. }));
        assert!(matches!(built_in("wrangler.jsonc", false, false), Icon::Badge { label: "{}", .. }));
        assert!(matches!(built_in(".dev.vars", false, false), Icon::Glyph { glyph: glyph::KEY, .. }));
        assert!(matches!(built_in("src", true, true), Icon::Glyph { glyph: glyph::FOLDER_OPEN, .. }));
        assert!(matches!(built_in("mystery", false, false), Icon::Glyph { glyph: glyph::FILE, .. }));
    }
}
