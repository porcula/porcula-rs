extern crate clap;
extern crate image;
extern crate quick_xml;
extern crate rand;
extern crate regex;
extern crate serde;
extern crate serde_json;
extern crate tantivy;
extern crate zip;
#[macro_use]
extern crate rouille;
#[macro_use]
extern crate lazy_static;
extern crate deepsize;

use std::collections::HashMap;
use std::path::Path;

mod assets;
mod fb2_parser;
mod fts;
mod genre_map;
mod img_resizer;
mod letter_replacer;
mod sort;
mod types;
#[macro_use]
pub mod cmd;

use self::cmd::*;
use crate::types::*;

fn main() {
    std::env::set_var("RUST_BACKTRACE", "1"); //force backtrace in every environment
    let args = cmd::parse_args();
    if args.debug {
        println!("{:?}", args);
    }

    let index_path = Path::new(&args.index_dir).to_path_buf();
    //auto-create index directory when indexing
    if !index_path.exists() {
        if let Some(cmd::Command::Index(_)) = args.command {
            match std::fs::create_dir(&index_path) {
                Ok(()) => eprintln!(
                    "{}: {}",
                    tr!["Directory created", "Создан каталог"],
                    index_path.canonicalize().unwrap().display()
                ),
                Err(e) => {
                    eprintln!(
                        "{}: {}",
                        tr!["Error creating directory", "Ошибка создания каталога"],
                        e
                    );
                    std::process::exit(1);
                }
            }
        } else {
            eprintln!(
                "{}: {}",
                tr![
                    "Creating non-existent index directory",
                    "Создаём отсутствующий каталог"
                ],
                index_path.display()
            );
        }
    }
    //canonicalize() DON'T WORK ON WINDOWS WITH DIRECTORY SYMLINK
    #[cfg(not(target_os = "windows"))]
    let index_path = index_path.canonicalize().unwrap_or_else(|e| {
        eprintln!(
            "{}: {}\n{}\n{}",
            tr!["Not found index directory", "Не найден индексный каталог"],
            index_path.display(),
            e,
            tr![
                "Run 'index' command or use --index-dir option",
                "Запустите команду 'index' или укажите путь опцией --index-dir"
            ],
        );
        std::process::exit(1);
    });

    let index_settings = IndexSettings::load(&args).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    });

    let mut books_path = Path::new(&index_settings.books_dir).to_path_buf();
    books_path = books_path.canonicalize().unwrap_or_else(|_| {
        eprintln!(
            "{}: {}\n{}",
            tr!["Not found books directory", "Не найден каталог с книгами"],
            index_settings.books_dir,
            tr!["Use --books-dir option", "Укажите путь опцией --books-dir"],
        );
        std::process::exit(1);
    });

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
