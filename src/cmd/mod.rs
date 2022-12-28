use clap::{Args, Parser, Subcommand};
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::io::prelude::*;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Termination};

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

pub const INDEX_SETTINGS_FILE: &str = "porcula_index_settings.json";
pub const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:8083";
pub const DEFAULT_QUERY_HITS: usize = 20;
pub const DEFAULT_LANGUAGE: &str = "ru";
pub const DEFAULT_BASE_URL: &str = "/porcula";
pub const DEFAULT_ASSETS_DIR: &str = "static";
pub const GENRE_MAP_FILENAME: &str = "genre-map.txt";

pub const COVER_IMAGE_WIDTH: u32 = 96;
pub const COVER_IMAGE_HEIGHT: u32 = 144;
pub const DEFAULT_COVER_IMAGE: &str = "defcover.png";

//language for user messages
lazy_static! {
    pub static ref MESSAGE_LANG: String = {
        std::env::var("LC_MESSAGES")
            .unwrap_or_else(|_| {
                std::env::var("LANG").unwrap_or_else(|_| DEFAULT_LANGUAGE.to_string())
            })
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
        if *$crate::cmd::MESSAGE_LANG == "ru" {
            $loc
        } else {
            $def
        }
    };
}

#[repr(u8)]
pub enum ProcessResult {
    Ok,
    ConfigError(String),
    IndexError(String),
    QueryError(String),
}

impl Termination for ProcessResult {
    fn report(self) -> ExitCode {
        match self {
            ProcessResult::Ok => ExitCode::from(0),
            //exit code 2 == Clap::error:Error.exit() code
            ProcessResult::ConfigError(e) => {
                error!(
                    "{}: {}",
                    tr!["Configuration error", "Ошибка конфигурации"],
                    e
                );
                ExitCode::from(3)
            }
            ProcessResult::IndexError(e) => {
                error!("{}: {}", tr!["Index error", "Ошибка индекса"], e);
                ExitCode::from(4)
            }
            ProcessResult::QueryError(e) => {
                error!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
                ExitCode::from(5)
            }
        }
    }
}

pub fn parse_args() -> AppArgs {
    AppArgs::parse()
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = tr!(
        "Full-text search on collection of e-books",
        "Полнотекстовый поиск по коллекции электронных книг"
    )
)]
pub struct AppArgs {
    #[clap(short, long, help=tr!(
        "Print debug information",
        "Вывод отладочной информации"
    ))]
    pub debug: bool,
    #[clap(short, long, default_value_t = String::from("index"), help=tr!("Index directory, read/write",
    "Каталог для индекса, чтение и запись"))]
    pub index_dir: String,
    #[clap(short, long, default_value_t = String::from("books"), help=tr!("Books directory, read only",
    "Каталог с книгами, только чтение"))]
    pub books_dir: String,
    #[clap(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    #[clap(about=tr!("Index/reindex books", "Индексация книг"))]
    Index(IndexArgs),
    #[clap(about=tr!("Start web server [default mode]", "Запустить веб-сервер [основной режим работы]"))]
    Server(ServerArgs),
    #[clap(about=tr!("Run single query, print result as JSON and exit", "Выполнить запрос, результат в формате JSON"))]
    Query(QueryArgs),
    #[clap(about=tr!("Run single facet query, print result as JSON and exit", "Выполнить фасетный запрос, результат в формате JSON"))]
    Facet(FacetArgs),
}

#[derive(Eq, PartialEq, Debug, strum::Display, strum::EnumString, Clone)]
#[strum(serialize_all = "lowercase")]
pub enum IndexMode {
    Full,
    Delta,
}

#[derive(Eq, PartialEq, Debug, strum::Display, strum::EnumString, Clone)]
#[strum(serialize_all = "lowercase")]
pub enum OnOff {
    On,
    Off,
}

#[derive(Args, Debug)]
pub struct IndexArgs {
    #[clap(default_value = "delta", help = tr!("Index mode: full or incremental",
    "Режим индексирования: полный или добавление"), value_name="full|delta")]
    pub mode: IndexMode,
    #[clap(short, long, help=tr!("Archive file name to reindex",
    "Имя отдельного архива для переиндексации"))]
    pub file: Vec<String>,
    #[clap(short, long, use_value_delimiter=true, help=tr!(
        "Language of books, one or more",
        "Язык книг, можно несколько"
    ), value_name = tr!("2 letter code | any", "2-буквенный код | any"))]
    pub lang: Vec<String>,
    #[clap(short, long, help=tr!("Word stemmer", "Алгоритм определения основы слова"), value_name=tr!("language code | off", "код языка | off"))]
    pub stemmer: Option<String>,
    #[clap(short, long, help=tr!("Memory size", "Размер памяти"), value_name = "MB")]
    pub memory_size: Option<usize>,
    #[clap(short, long, help=tr!("Number of indexing workers", "Число потоков индексирования"))]
    pub index_threads: Option<usize>,
    #[clap(short, long, default_value_t = 1, help=tr!("Number of read workers", "Число потоков чтения"))]
    pub read_threads: usize,
    #[clap(short='q', long, default_value_t=64, help=tr!("Length of read queue", "Длина очереди чтения"))]
    pub read_queue: usize,
    #[clap(short='B', long, help=tr!("Batch size between commits","Размер данных между сохранениями"), value_name="MB")]
    pub batch_size: Option<usize>,
    #[clap(long, help=tr!("Index book's body", "Индексировать текст книги (без учёта склонения)"), value_name="on|off")]
    pub body: Option<OnOff>,
    #[clap(long, help=tr!("Index book's body with stemming", "Индексировать текст книги (по основам слов)"), value_name="on|off")]
    pub xbody: Option<OnOff>,
    #[clap(long, help=tr!("Index book's annotation", "Индексировать аннотацию"), value_name="on|off")]
    pub annotation: Option<OnOff>,
    #[clap(long, help=tr!("Extract book's cover image", "Извлекать обложку книги"), value_name="on|off")]
    pub cover: Option<OnOff>,
}

#[derive(Args, Debug)]
pub struct ServerArgs {
    #[clap(short, long, default_value = DEFAULT_LISTEN_ADDR, help=tr!("Listen address", "Адрес сервера"), value_name = "ADDRESS:PORT")]
    pub listen: String,
}
impl Default for ServerArgs {
    fn default() -> Self {
        ServerArgs {
            listen: DEFAULT_LISTEN_ADDR.into(),
        }
    }
}

#[derive(Args, Debug)]
pub struct QueryArgs {
    #[clap(help=tr!("Query text", "Текст запроса"))]
    pub query: String,
    #[clap(short = 'H', long, default_value_t = DEFAULT_QUERY_HITS, help=tr!("Limit results to N top hits", "Ограничить число найденных книг"))]
    pub hits: usize,
    #[clap(short = 'x', long, help=tr!("Search in stemmed fields", "Поиск по всем формам слова"))]
    pub stem: bool,
    #[clap(short = 'o', long="or", help=tr!("Logical OR by default", "Логическое ИЛИ по умолчанию"))]
    pub disjunction: bool,
}

#[derive(Args, Debug)]
pub struct FacetArgs {
    #[clap(help=tr!("Facet path, i.e. '/author/K' or '/genre/fiction/sf'","Путь по категориям, например '/author/K' или '/genre/fiction/sf'"))]
    pub path: String,
    #[clap(short = 'H', long, default_value_t = DEFAULT_QUERY_HITS, help=tr!("Limit results to N top hits", "Ограничить число найденных книг"))]
    pub hits: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseOpts {
    pub body: bool,
    pub xbody: bool,
    pub annotation: bool,
    pub cover: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct IndexSettings {
    pub langs: Vec<String>,
    pub stemmer: String,
    pub books_dir: String,
    pub options: ParseOpts,
}

//for web-app
#[derive(Serialize, Clone)]
pub struct IndexInfo {
    pub count: usize, //book count
    pub settings: IndexSettings,
}

pub struct Application {
    pub index_settings: IndexSettings,
    pub index_path: PathBuf,
    pub books_path: PathBuf,
    pub book_formats: BookFormats,
    pub debug: bool,
}

impl IndexSettings {
    // load or create settings stored with index
    pub fn load(args: &AppArgs) -> Result<Self, String> {
        let index_path = Path::new(&args.index_dir).to_path_buf();
        let filename = index_path.join(INDEX_SETTINGS_FILE);
        let mut res: IndexSettings = if let Ok(f) = std::fs::File::open(&filename) {
            debug!(
                "{} {}",
                tr!["Reading settings", "Читаем настройки"],
                filename.display()
            );
            match serde_json::from_reader(f) {
                Ok(s) => s,
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
        } else {
            //file not exists yet - use defaults
            IndexSettings {
                langs: vec![DEFAULT_LANGUAGE.to_string()],
                stemmer: "off".to_string(),
                books_dir: args.books_dir.clone(),
                options: ParseOpts {
                    body: true,
                    xbody: true,
                    annotation: true,
                    cover: true,
                },
            }
        };
        if let Some(Command::Index(args)) = &args.command {
            if !args.lang.is_empty() {
                res.langs = args.lang.clone();
            }
            if let Some(stemmer) = &args.stemmer {
                res.stemmer = stemmer.clone();
            }
            if let Some(x) = &args.body {
                res.options.body = *x == OnOff::On;
            }
            if let Some(x) = &args.xbody {
                res.options.xbody = *x == OnOff::On;
            }
            if let Some(x) = &args.annotation {
                res.options.annotation = *x == OnOff::On;
            }
            if let Some(x) = &args.cover {
                res.options.cover = *x == OnOff::On;
            }
        }
        assert!(
            !res.langs.is_empty(),
            "{} {}",
            tr![
                "No language specified nor on command line [--lang], nor in settings file",
                "Не указан язык ни в командной строке [--lang], ни в файле настроек"
            ],
            INDEX_SETTINGS_FILE
        );
        Ok(res)
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
            !self.index_settings.langs.is_empty(),
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

    pub fn load_genre_map(&self) -> Result<GenreMap, String> {
        let genre_map_path = Path::new(DEFAULT_ASSETS_DIR).join(GENRE_MAP_FILENAME);
        if genre_map_path.exists() {
            //load file
            let file = match std::fs::File::open(genre_map_path) {
                Ok(f) => f,
                Err(e) => return Err(e.to_string()),
            };
            let mut buf = BufReader::new(file);
            GenreMap::load(&mut buf).map_err(|e| e.to_string())
        } else {
            //load static asset
            let data = assets::get(GENRE_MAP_FILENAME)
                .expect("Genre map not found")
                .content;
            let mut buf = BufReader::new(data);
            GenreMap::load(&mut buf).map_err(|e| e.to_string())
        }
    }
}

pub fn file_extension(s: &str) -> String {
    match s.rfind('.') {
        Some(i) => s[i..].to_lowercase(),
        None => String::new(),
    }
}
