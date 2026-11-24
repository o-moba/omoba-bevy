//! The game's synthetic roster identities and their packaged portraits.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotPersonaId {
    Sever,
    Rikoshet,
    Myata,
    Buran,
    Iskra,
    Sova,
    Kaktus,
    Kometa,
    Fonar,
    Tuman,
    Yantar,
    Bambuk,
    Prizma,
    Volna,
    Shpil,
    Klyukva,
    Kremen,
    Runa,
    Shum,
    Plomba,
    Vetka,
    Kvarts,
    Sirena,
    Krater,
    Zefir,
    Nyra,
    Yasha,
    Kobalt,
    Sol,
    Smola,
}

impl BotPersonaId {
    pub const ALL: [Self; 30] = [
        Self::Sever,
        Self::Rikoshet,
        Self::Myata,
        Self::Buran,
        Self::Iskra,
        Self::Sova,
        Self::Kaktus,
        Self::Kometa,
        Self::Fonar,
        Self::Tuman,
        Self::Yantar,
        Self::Bambuk,
        Self::Prizma,
        Self::Volna,
        Self::Shpil,
        Self::Klyukva,
        Self::Kremen,
        Self::Runa,
        Self::Shum,
        Self::Plomba,
        Self::Vetka,
        Self::Kvarts,
        Self::Sirena,
        Self::Krater,
        Self::Zefir,
        Self::Nyra,
        Self::Yasha,
        Self::Kobalt,
        Self::Sol,
        Self::Smola,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Sever => "sever",
            Self::Rikoshet => "rikoshet",
            Self::Myata => "myata",
            Self::Buran => "buran",
            Self::Iskra => "iskra",
            Self::Sova => "sova",
            Self::Kaktus => "kaktus",
            Self::Kometa => "kometa",
            Self::Fonar => "fonar",
            Self::Tuman => "tuman",
            Self::Yantar => "yantar",
            Self::Bambuk => "bambuk",
            Self::Prizma => "prizma",
            Self::Volna => "volna",
            Self::Shpil => "shpil",
            Self::Klyukva => "klyukva",
            Self::Kremen => "kremen",
            Self::Runa => "runa",
            Self::Shum => "shum",
            Self::Plomba => "plomba",
            Self::Vetka => "vetka",
            Self::Kvarts => "kvarts",
            Self::Sirena => "sirena",
            Self::Krater => "krater",
            Self::Zefir => "zefir",
            Self::Nyra => "nyra",
            Self::Yasha => "yasha",
            Self::Kobalt => "kobalt",
            Self::Sol => "sol",
            Self::Smola => "smola",
        }
    }

    pub const fn nickname(self) -> &'static str {
        match self {
            Self::Sever => "Север",
            Self::Rikoshet => "Рикошет",
            Self::Myata => "Мята",
            Self::Buran => "Буран",
            Self::Iskra => "Искра",
            Self::Sova => "Сова",
            Self::Kaktus => "Кактус",
            Self::Kometa => "Комета",
            Self::Fonar => "Фонарь",
            Self::Tuman => "Туман",
            Self::Yantar => "Янтарь",
            Self::Bambuk => "Бамбук",
            Self::Prizma => "Призма",
            Self::Volna => "Волна",
            Self::Shpil => "Шпиль",
            Self::Klyukva => "Клюква",
            Self::Kremen => "Кремень",
            Self::Runa => "Руна",
            Self::Shum => "Шум",
            Self::Plomba => "Пломба",
            Self::Vetka => "Ветка",
            Self::Kvarts => "Кварц",
            Self::Sirena => "Сирена",
            Self::Krater => "Кратер",
            Self::Zefir => "Зефир",
            Self::Nyra => "Ныра",
            Self::Yasha => "Яша",
            Self::Kobalt => "Кобальт",
            Self::Sol => "Соль",
            Self::Smola => "Смола",
        }
    }
}

/// Resolve a server roster key through the bundled catalog before loading it.
pub fn portrait_asset_path(is_bot: bool, slug: Option<&str>) -> Option<String> {
    if !is_bot {
        return None;
    }
    let slug = slug?;
    BotPersonaId::ALL
        .iter()
        .find(|id| id.slug() == slug)
        .map(|id| format!("ui/bot-profiles/{}.png", id.slug()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs::{self, File};
    use std::io::Read;
    use std::path::Path;

    #[test]
    fn personas_have_distinct_valid_names_and_packaged_paths() {
        let mut names = HashSet::new();
        let mut paths = HashSet::new();
        for id in BotPersonaId::ALL {
            assert!(names.insert(id.nickname()));
            assert_eq!(
                crate::career::normalize_nickname(id.nickname()).unwrap(),
                id.nickname()
            );
            assert!(paths.insert(portrait_asset_path(true, Some(id.slug())).unwrap()));
            assert!(portrait_asset_path(false, Some(id.slug())).is_none());
        }
        assert!(portrait_asset_path(true, Some("../unsupported")).is_none());
    }

    #[test]
    fn packaged_portraits_cover_the_catalog_within_budget() {
        const MAX_PORTRAIT_BYTES: u64 = 128 * 1024;
        const MAX_CATALOG_BYTES: u64 = 3 * 1024 * 1024;
        const MAX_SIDE: u32 = 192;
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../client/assets");
        let directory = assets.join("ui/bot-profiles");
        let mut files: HashSet<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
            .collect();
        let mut total_bytes = 0;
        for persona in BotPersonaId::ALL {
            let relative = portrait_asset_path(true, Some(persona.slug())).unwrap();
            let path = assets.join(relative);
            assert!(files.remove(&path), "Portrait missing: {}", path.display());
            let metadata = path.symlink_metadata().unwrap();
            assert!(metadata.file_type().is_file(), "{}", path.display());
            let bytes = metadata.len();
            assert!(
                bytes <= MAX_PORTRAIT_BYTES,
                "{}: {bytes} bytes",
                path.display()
            );
            total_bytes += bytes;

            let mut header = [0; 24];
            File::open(&path).unwrap().read_exact(&mut header).unwrap();
            assert_eq!(&header[..16], b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR");
            let width = u32::from_be_bytes(header[16..20].try_into().unwrap());
            let height = u32::from_be_bytes(header[20..24].try_into().unwrap());
            assert_eq!(width, height, "{}: {width} × {height}", path.display());
            assert!(
                (1..=MAX_SIDE).contains(&width),
                "{}: {width}px",
                path.display()
            );
        }
        assert!(
            files.is_empty(),
            "Portraits missing from bot catalog: {files:?}"
        );
        assert!(
            total_bytes <= MAX_CATALOG_BYTES,
            "Catalog uses {total_bytes} bytes"
        );
    }
}
