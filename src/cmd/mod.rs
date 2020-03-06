use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::prelude::*;
use std::io::BufReader;
use std::path::{Path, PathBuf};

mod facet;
mod index;
mod query;
mod server;
pub use self::facet::run_facet;
pub use self::index::run_index;
pub use self::query::run_query;
pub use self::server::run_server;

use crate::assets;
use crate::fts::BookReader;
use crate::genre_map::GenreMap;
use crate::types::BookFormats;

pub const INDEX_SETTINGS_FILE: &'static str = "porcula_index_settings.json";
pub const DEFAULT_LANGUAGE: &'static str = "ru";
pub const DEFAULT_INDEX_DIR: &'static str = "index";
pub const DEFAULT_BOOKS_DIR: &'static str = "books";
pub const DEFAULT_HEAP_SIZE_MB: &'static str = "100";
pub const DEFAULT_BATCH_SIZE_MB: &'static str = "300";
pub const DEFAULT_LISTEN_ADDR: &'static str = "127.0.0.1:8083";
pub const DEFAULT_QUERY_HITS: usize = 10;
pub const DEFAULT_BASE_URL: &'static str = "/porcula";
pub const DEFAULT_ASSETS_DIR: &'static str = "static";
pub const GENRE_MAP_FILENAME: &'static str = "genre-map.txt";

pub const COVER_IMAGE_WIDTH: u32 = 96;
pub const COVER_IMAGE_HEIGHT: u32 = 144;
pub const DEFAULT_COVER_IMAGE: &'static str = "defcover.png";

#[derive(Serialize, Deserialize)]
pub struct IndexSettings {
    pub langs: Vec<String>,
    pub stemmer: String,
    pub books_dir: String,
    pub disabled: HashSet<String>,
}

pub struct Application {
    pub index_settings: IndexSettings,
    pub index_path: PathBuf,
    pub books_path: PathBuf,
    pub book_formats: BookFormats,
    pub genre_map: GenreMap,
    pub debug: bool,
}

//language for user messages
lazy_static! {
    pub static ref MESSAGE_LANG: String = {
        std::env::var("LC_MESSAGES")
            .unwrap_or_else(|_| std::env::var("LANG").unwrap_or(DEFAULT_LANGUAGE.to_string()))
            .chars()
            .take(2)
            .collect::<String>()
            .to_lowercase()
    };
}

//dumb message translation: first &str is English, second is localized [Russian]
//all resources compiled in
#[macro_export]
macro_rules! tr {
    ( $def:expr, $loc:expr ) => {
        if *crate::cmd::MESSAGE_LANG == "ru" {
            $loc
        } else {
            $def
        }
    };
}

impl IndexSettings {
    // load or create settings stored with index
    pub fn load(index_path: &Path, debug: bool) -> Result<Self, String> {
        let filename = index_path.join(INDEX_SETTINGS_FILE);
        if let Ok(f) = std::fs::File::open(&filename) {
            if debug {
                println!(
                    "{} {}",
                    tr!["Reading settings", "Читаем настройки"],
                    filename.display()
                );
            }
            match serde_json::from_reader(f) {
                Ok(s) => return Ok(s),
                Err(e) => {
                    return Err(format!(
                        "{}: {}: {}",
                        tr![
                            "Invalid settings file for index",
                            "Неправильный файл с настройками индекса"
                        ],
                        filename.display(),
                        e
                    ))
                }
            }
        }
        //file not exists yet - use defaults
        Ok(IndexSettings {
            langs: vec![DEFAULT_LANGUAGE.to_string()],
            stemmer: DEFAULT_LANGUAGE.to_string(),
            books_dir: DEFAULT_BOOKS_DIR.to_string(),
            disabled: HashSet::<String>::new(),
        })
    }

    pub fn save(&self, index_path: &Path) -> Result<(), String> {
        let filename = index_path.join(INDEX_SETTINGS_FILE);
        let mut f = std::fs::File::create(&filename).unwrap();
        let json = serde_json::to_string(&self).unwrap();
        match f.write(json.as_bytes()) {
            Ok(_) => Ok(()),
            Err(e) => Err(format!(
                "{} {}: {}",
                tr!["Error saving file", "Ошибка сохранения файла"],
                filename.display(),
                e
            )),
        }
    }
}

impl Application {
    pub fn open_book_reader(&self) -> Result<BookReader, String> {
        assert!(
            self.index_settings.langs.len() > 0,
            "{}",
            tr!["Empty language list", "Пустой список языков"],
        );
        let primary_lang = &self.index_settings.langs[0];
        match BookReader::new(&self.index_path, primary_lang) {
            Ok(r) => Ok(r),
            Err(e) => Err(format!(
                "{} '{}': {}\n{}",
                tr!["Error opening index in", "Ошибка открытия индекса в"],
                self.index_path.display(),
                e,
                tr![
                    "Try to rebuild with 'index full' command",
                    "Попробуйте пересоздать индекс командой 'index full'"
                ],
            )),
        }
    }

    pub fn load_genre_map(&mut self) {
        let genre_map_path = Path::new(DEFAULT_ASSETS_DIR).join(GENRE_MAP_FILENAME);
        let maybe_map = if genre_map_path.exists() {
            //load file
            let mut f = BufReader::new(std::fs::File::open(genre_map_path).unwrap());
            GenreMap::load(&mut f)
        } else {
            //load static asset
            let data = assets::get(GENRE_MAP_FILENAME)
                .expect("Genre map not found")
                .content;
            let mut f = BufReader::new(data);
            GenreMap::load(&mut f)
        };
        self.genre_map = maybe_map.unwrap_or_else(|_| {
            eprintln!(
                "{}: {}",
                tr!["Invalid file format", "Неправильный формат файла"],
                GENRE_MAP_FILENAME
            );
            std::process::exit(1);
        })
    }
}

pub fn file_extension(s: &str) -> String {
    match s.rfind('.') {
        Some(i) => s[i..].to_lowercase(),
        None => String::new(),
    }
}
