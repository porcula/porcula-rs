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

use clap::{Arg, SubCommand};
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
use crate::genre_map::GenreMap;
use crate::types::*;

#[allow(clippy::cognitive_complexity)]
fn cmd_line_matches<'a>() -> clap::ArgMatches<'a> {
    clap::App::new("Porcula")
        .version(env!("CARGO_PKG_VERSION"))
        .about(tr![
            "Full-text search on collection of e-books",
            "Полнотекстовый поиск по коллекции электронных книг"
        ])
        .arg(
            Arg::with_name("index-dir")
                .short("i")
                .long("index-dir")
                .takes_value(true)
                .value_name("DIR")
                .default_value(DEFAULT_INDEX_DIR)
                .help(tr![
                    "Index directory, read/write",
                    "Каталог для индекса, чтение и запись"
                ]),
        )
        .arg(
            Arg::with_name("books-dir")
                .short("b")
                .long("books-dir")
                .takes_value(true)
                .value_name("DIR")
                .default_value(DEFAULT_BOOKS_DIR)
                .help(tr![
                    "Books directory, read only",
                    "Каталог с книгами, только чтение"
                ]),
        )
        .arg(Arg::with_name("debug").short("d").long("debug").help(tr![
            "Print debug information",
            "Вывод отладочной информации"
        ]))
        .subcommand(
            SubCommand::with_name("index")
                .about(tr!["Index/reindex books", "Индексация книг"])
                .arg(
                    Arg::with_name("INDEX-MODE")
                        .required(true)
                        .index(1)
                        .possible_values(&["full", "delta"])
                        .default_value("delta")
                        .help(tr![
                            "Index mode: full or incremental",
                            "Режим индексирования: полный или добавление"
                        ]),
                )
                .arg(
                    Arg::with_name("file")
                        .short("f")
                        .long("file")
                        .takes_value(true)
                        .required(false)
                        .multiple(true)
                        .help(tr![
                            "Archive file name to reindex",
                            "Имя отдельного архива для переиндексации"
                        ]),
                )
                .arg(
                    Arg::with_name("language")
                        .short("l")
                        .long("lang")
                        .takes_value(true)
                        .multiple(true)
                        .use_delimiter(true)
                        .value_name(tr!["2 letter code | ANY", "2-буквенный код | ANY"])
                        .default_value(DEFAULT_LANGUAGE)
                        .help(tr![
                            "Language of books, one or more",
                            "Язык книг, можно несколько"
                        ]),
                )
                .arg(
                    Arg::with_name("stemmer")
                        .long("stemmer")
                        .takes_value(true)
                        .value_name(tr!["language code | OFF", "код языка | OFF"])
                        .default_value("OFF")
                        .help(tr!["Word stemmer", "Алгоритм определения основы слова"]),
                )
                .arg(
                    Arg::with_name("index-threads")
                        .short("t")
                        .long("index-threads")
                        .takes_value(true)
                        .value_name("number")
                        .default_value(tr!["all CPUs", "все CPU"])
                        .help(tr![
                            "Number of indexing workers",
                            "Число потоков индексирования"
                        ]),
                )
                .arg(
                    Arg::with_name("heap-memory")
                        .default_value(DEFAULT_HEAP_SIZE_MB)
                        .short("m")
                        .long("heap-memory")
                        .takes_value(true)
                        .value_name("MB")
                        .help(tr!["Heap memory size", "Размер памяти"]),
                )
                .arg(
                    Arg::with_name("batch-size")
                        .default_value(DEFAULT_BATCH_SIZE_MB)
                        .short("B")
                        .long("batch-size")
                        .takes_value(true)
                        .value_name("MB")
                        .help(tr![
                            "Batch size between commits",
                            "Размер данных между сохранениями"
                        ]),
                )
                .arg(
                    Arg::with_name("read-threads")
                        .short("r")
                        .long("read-threads")
                        .takes_value(true)
                        .value_name("number")
                        .default_value("1")
                        .help(tr!["Number of read workers", "Число потоков чтения"]),
                )
                .arg(
                    Arg::with_name("read-queue")
                        .short("q")
                        .long("read-queue")
                        .takes_value(true)
                        .value_name("number")
                        .default_value("64")
                        .help(tr!["Size of read queue", "Размер очереди чтения"]),
                )
                .arg(
                    Arg::with_name("with-body")
                        .long("with-body")
                        .help(tr![
                            "Enable indexing of book's body",
                            "Индексировать текст книги (без учёта склонения)"
                        ])
                        .conflicts_with("without-body"),
                )
                .arg(
                    Arg::with_name("without-body")
                        .long("without-body")
                        .help(tr![
                            "Disable indexing of book's body",
                            "Не индексировать текст книги"
                        ])
                        .conflicts_with("with-body"),
                )
                .arg(
                    Arg::with_name("with-xbody")
                        .long("with-xbody")
                        .help(tr![
                            "Enable indexing of book's body with stemming",
                            "Индексировать текст книги (по основам слов)"
                        ])
                        .conflicts_with("without-xbody"),
                )
                .arg(
                    Arg::with_name("without-xbody")
                        .long("without-xbody")
                        .help(tr![
                            "Disable indexing of book's body with stemming",
                            "Не индексировать текст книги (по основам слов)"
                        ])
                        .conflicts_with("with-xbody"),
                )
                .arg(
                    Arg::with_name("with-annotation")
                        .long("with-annotation")
                        .help(tr![
                            "Enable indexing of book's annotation",
                            "Индексировать аннотацию"
                        ])
                        .conflicts_with("without-annotation"),
                )
                .arg(
                    Arg::with_name("without-annotation")
                        .long("without-annotation")
                        .help(tr![
                            "Disable indexing of book's annotation",
                            "Не индексировать аннотацию"
                        ])
                        .conflicts_with("with-annotation"),
                )
                .arg(
                    Arg::with_name("with-cover")
                        .long("with-cover")
                        .help(tr![
                            "Enable extraction of book's cover image",
                            "Извлекать обложку книги"
                        ])
                        .conflicts_with("without-cover"),
                )
                .arg(
                    Arg::with_name("without-cover")
                        .long("without-cover")
                        .help(tr![
                            "Disable extraction of book's cover image",
                            "Не извлекать обложку книги"
                        ])
                        .conflicts_with("with-cover"),
                ),
        )
        .subcommand(
            SubCommand::with_name("query")
                .about(tr![
                    "Run single query, print result as JSON and exit",
                    "Выполнить запрос, результат в формате JSON"
                ])
                .arg(
                    Arg::with_name("QUERY-TEXT")
                        .required(true)
                        .index(1)
                        .help(tr!["Query text", "Текст запроса"]),
                )
                .arg(
                    Arg::with_name("hits")
                        .default_value(DEFAULT_QUERY_HITS_STR)
                        .short("h")
                        .long("hits")
                        .takes_value(true)
                        .value_name("INT")
                        .help(tr![
                            "Limit results to N top hits",
                            "Ограничить число найденных книг"
                        ]),
                ),
        )
        .subcommand(
            SubCommand::with_name("facet")
                .about(tr![
                    "Run single facet query, print result as JSON and exit",
                    "Выполнить фасетный запрос, результат в формате JSON"
                ])
                .arg(Arg::with_name("PATH").required(true).index(1).help(tr![
                    "Facet path, i.e. '/author/K' or '/genre/sf'",
                    "Путь по категориям, например '/author/K' или '/genre/sf'"
                ]))
                .arg(
                    Arg::with_name("hits")
                        .default_value(DEFAULT_QUERY_HITS_STR)
                        .short("h")
                        .long("hits")
                        .takes_value(true)
                        .value_name("INT")
                        .help(tr![
                            "Limit results to N top hits",
                            "Ограничить число найденного"
                        ]),
                ),
        )
        .subcommand(
            SubCommand::with_name("server")
                .about(tr![
                    "Start web server [default mode]",
                    "Запустить веб-сервер [основной режим работы]"
                ])
                .arg(
                    Arg::with_name("listen")
                        .default_value(DEFAULT_LISTEN_ADDR)
                        .short("L")
                        .long("listen")
                        .takes_value(true)
                        .value_name("IP:PORT")
                        .help(tr!["Listen address", "Адрес сервера"]),
                ),
        )
        .get_matches()
}

fn main() {
    std::env::set_var("RUST_BACKTRACE", "1"); //force backtrace in every environment
    let matches = cmd_line_matches();

    let debug = matches.is_present("debug");
    let index_mode_matches = matches.subcommand_matches("index");
    let query_mode_matches = matches.subcommand_matches("query");
    let facet_mode_matches = matches.subcommand_matches("facet");
    let server_mode_matches = matches.subcommand_matches("server");

    let books_dir_required = query_mode_matches.is_none() && facet_mode_matches.is_none();

    let index_dir = String::from(matches.value_of("index-dir").unwrap_or(DEFAULT_INDEX_DIR));
    let index_path = Path::new(&index_dir).to_path_buf();
    //auto-create index directory when indexing
    if !index_path.exists() && index_mode_matches.is_some() {
        eprintln!(
            "{}: {}",
            tr![
                "Creating non-existent index directory",
                "Создаём отсутствующий каталог"
            ],
            index_path.display()
        );
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

    let mut index_settings = IndexSettings::load(&index_path, debug).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    });

    // books-dir overrided in command line?
    if matches.occurrences_of("books-dir") > 0 {
        if let Some(dir) = matches.value_of("books-dir") {
            index_settings.books_dir = dir.to_string();
        }
    }
    let mut books_path = Path::new(&index_settings.books_dir).to_path_buf();
    if books_dir_required {
        books_path = books_path.canonicalize().unwrap_or_else(|_| {
            eprintln!(
                "{}: {}\n{}",
                tr!["Not found books directory", "Не найден каталог с книгами"],
                index_settings.books_dir,
                tr!["Use --books-dir option", "Укажите путь опцией --books-dir"],
            );
            std::process::exit(1);
        });
    }

    let mut book_formats: BookFormats = HashMap::new();
    book_formats.insert(".fb2", Box::new(fb2_parser::Fb2BookFormat {}));

    let mut app = Application {
        books_path,
        index_path,
        book_formats,
        index_settings,
        genre_map: GenreMap::default(), //defer load
        debug,
    };

    //////////////////////INDEXING MODE
    if let Some(matches) = index_mode_matches {
        run_index(matches, &mut app);
    }
    //////////////////////QUERY MODE
    else if let Some(matches) = query_mode_matches {
        run_query(matches, &app);
    } else if let Some(matches) = facet_mode_matches {
        run_facet(matches, &app);
    }
    //////////////////////SERVER MODE [default]
    else {
        app.load_genre_map();
        let matches = match server_mode_matches {
            Some(x) => x,
            None => &matches,
        };
        run_server(&matches, app).unwrap();
    }
}
