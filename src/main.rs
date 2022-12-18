#[macro_use]
extern crate rouille;
#[macro_use]
extern crate lazy_static;

use log::{debug, info, LevelFilter};
use std::collections::HashMap;
use std::path::Path;

mod assets;
mod fb2_parser;
mod fts;
mod genre_map;
mod img_resizer;
mod letter_replacer;
mod mtime_checker;
mod sort;
mod types;
#[macro_use]
pub mod cmd;

use self::cmd::*;
use crate::types::*;

fn main() -> ProcessResult {
    std::env::set_var("RUST_BACKTRACE", "1"); //force backtrace in every environment
    let args = cmd::parse_args();
    env_logger::Builder::new()
        .filter_level(if args.debug {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        })
        .format_timestamp_secs()
        .filter_module("tantivy", LevelFilter::Warn)
        .parse_default_env()
        .init();
    debug!("{:?}", args);

    let index_path = Path::new(&args.index_dir).to_path_buf();
    //auto-create index directory when indexing
    if !index_path.exists() {
        if let Some(cmd::Command::Index(_)) = args.command {
            match std::fs::create_dir(&index_path) {
                Ok(()) => info!(
                    "{}: {}",
                    tr!["Directory created", "Создан каталог"],
                    index_path.canonicalize().unwrap().display()
                ),
                Err(e) => {
                    return ProcessResult::IndexError(format!(
                        "{}: {}",
                        tr!["Error creating directory", "Ошибка создания каталога"],
                        e
                    ));
                }
            }
        }
    }
    //canonicalize() DON'T WORK ON WINDOWS WITH DIRECTORY SYMLINK
    #[cfg(not(target_os = "windows"))]
    let index_path = match index_path.canonicalize() {
        Ok(x) => x,
        Err(e) => {
            return ProcessResult::ConfigError(format!(
                "{}: {}\n{}\n{}",
                tr!["Not found index directory", "Не найден индексный каталог"],
                index_path.display(),
                e,
                tr![
                    "Run 'index' command or use --index-dir option",
                    "Запустите команду 'index' или укажите путь опцией --index-dir"
                ],
            ));
        }
    };

    let index_settings = match IndexSettings::load(&args) {
        Ok(x) => x,
        Err(e) => return ProcessResult::IndexError(e),
    };

    let mut books_path = Path::new(&index_settings.books_dir).to_path_buf();
    books_path = match books_path.canonicalize() {
        Ok(x) => x,
        Err(e) => {
            return ProcessResult::ConfigError(format!(
                "{}: {}, {}\n{}",
                tr!["Not found books directory", "Не найден каталог с книгами"],
                index_settings.books_dir,
                e,
                tr!["Use --books-dir option", "Укажите путь опцией --books-dir"],
            ));
        }
    };

    let mut book_formats: BookFormats = HashMap::new();
    book_formats.insert(".fb2", Box::new(fb2_parser::Fb2BookFormat {}));

    let app = Application {
        index_settings,
        books_path,
        index_path,
        book_formats,
        debug: args.debug,
    };

    match args.command {
        None => run_server(&ServerArgs::default(), app),
        Some(Command::Server(args)) => run_server(&args, app),
        Some(Command::Index(args)) => run_index(&args, app),
        Some(Command::Query(args)) => run_query(&args, app),
        Some(Command::Facet(args)) => run_facet(&args, app),
    }
}
