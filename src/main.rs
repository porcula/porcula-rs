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

use clap::{Arg, SubCommand};
use rouille::{Request, Response};
use serde::{Deserialize, Serialize};

use std::collections::{HashMap, HashSet};
use std::fs::DirEntry;
use std::io::prelude::*;
use std::io::BufReader;
use std::path::Path;
use std::time::Instant;

mod assets;
mod fb2_parser;
mod fts;
mod genre_map;
mod img_resizer;
mod types;

use fts::{BookReader, BookWriter};
use genre_map::GenreMap;
use types::*;

const SETTINGS_FILE: &'static str = "porcula_index_settings.json";
const DEFAULT_LANGUAGE: &'static str = "ru";
const DEFAULT_INDEX_DIR: &'static str = "index";
const DEFAULT_BOOKS_DIR: &'static str = "books";
const DEFAULT_HEAP_SIZE: &'static str = "100";
const DEFAULT_BATCH_SIZE: &'static str = "100";
const DEFAULT_LISTEN_ADDR: &'static str = "127.0.0.1:8083";
const DEFAULT_QUERY_HITS: &'static str = "10";
const DEFAULT_BASE_URL: &'static str = "/porcula";
const DEFAULT_ASSETS_DIR: &'static str = "static";

const COVER_IMAGE_WIDTH: u32 = 96;
const COVER_IMAGE_HEIGHT: u32 = 144;
const DEFAULT_COVER_IMAGE: &'static str = "defcover.png";

#[derive(Serialize, Deserialize)]
struct Settings {
    langs: Vec<String>,
    stemmer: String,
    books_dir: String,
    no_body: bool,
}

type BookFormats = HashMap<&'static str, Box<dyn BookFormat + Send + Sync>>;

//language for user messages
lazy_static! {
    static ref MESSAGE_LANG: String = {
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
macro_rules! tr {
    ( $def:expr, $loc:expr ) => {
        if *MESSAGE_LANG == "ru" {
            $loc
        } else {
            $def
        }
    };
}

fn main() {
    std::env::set_var("RUST_BACKTRACE", "1"); //force backtrace in every environment

    let mut book_formats: BookFormats = HashMap::new();
    let fb2 = fb2_parser::FB2BookFormat {};
    book_formats.insert(fb2.file_extension(), Box::new(fb2));

    let matches = clap::App::new("Porcula")
        .version("0.1")
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
                        .default_value(DEFAULT_LANGUAGE)
                        .help(tr!["Word stemmer", "Алгоритм определения основы слова"]),
                )
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
                    Arg::with_name("threads")
                        .short("t")
                        .long("threads")
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
                        .default_value(DEFAULT_HEAP_SIZE)
                        .short("m")
                        .long("heap-memory")
                        .takes_value(true)
                        .value_name("MB")
                        .help(tr!["Heap memory size", "Размер памяти"]),
                )
                .arg(
                    Arg::with_name("batch-size")
                        .short("b")
                        .long("batch-size")
                        .takes_value(true)
                        .value_name("INT")
                        .default_value(DEFAULT_BATCH_SIZE)
                        .help(tr![
                            "Commit after each N-th books",
                            "Сохранение каждых N-книг"
                        ]),
                )
                .arg(Arg::with_name("no-body").long("no-body").help(tr![
                    "Disable indexing of book's body",
                    "Отключить индексацию основного текста книги"
                ])),
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
                        .default_value(DEFAULT_QUERY_HITS)
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
                ])),
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
        .get_matches();

    let debug = matches.is_present("debug");
    let books_dir_required = matches.subcommand_matches("query").is_none()
        && matches.subcommand_matches("facet").is_none();

    let index_dir = String::from(matches.value_of("index-dir").unwrap_or(DEFAULT_INDEX_DIR));
    let index_path = Path::new(&index_dir).to_path_buf();
    //auto-create index directory when indexing
    if !index_path.exists() && matches.subcommand_matches("index").is_some() {
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
    let index_path = index_path.canonicalize().unwrap_or_else(|_| {
        eprintln!(
            "{}: {}\n{}",
            tr!["Not found index directory", "Не найден индексный каталог"],
            index_path.display(),
            tr![
                "Run 'index' command or use --index-dir=... option",
                "Запустите команду 'index' или укажите путь опцией --index-dir=..."
            ],
        );
        std::process::exit(1);
    });

    // loading settings stored with index
    let settings_filename = index_path.join(SETTINGS_FILE);
    let mut settings: Settings = match std::fs::File::open(&settings_filename) {
        Ok(f) => serde_json::from_reader(f).unwrap_or_else(|e| {
            eprintln!(
                "{}: {}: {}",
                tr![
                    "Invalid settings file for index",
                    "Неправильный файл с настройками индекса"
                ],
                settings_filename.display(),
                e
            );
            std::process::exit(2);
        }),
        Err(_) => Settings {
            langs: vec![DEFAULT_LANGUAGE.to_string()],
            stemmer: DEFAULT_LANGUAGE.to_string(),
            books_dir: DEFAULT_BOOKS_DIR.to_string(),
            no_body: false,
        },
    };

    // books-dir overrided in command line?
    if matches.occurrences_of("books-dir") > 0 {
        if let Some(dir) = matches.value_of("books-dir") {
            settings.books_dir = dir.to_string();
        }
    }
    let mut books_path = Path::new(&settings.books_dir).to_path_buf();
    if books_dir_required {
        books_path = books_path.canonicalize().unwrap_or_else(|_| {
            eprintln!(
                "{}: {}\n{}",
                tr!["Not found books directory", "Не найден каталог с книгами"],
                settings.books_dir,
                tr![
                    "Use --books-dir=... option",
                    "Укажите путь опцией --books-dir=..."
                ],
            );
            std::process::exit(1);
        });
    }

    let genre_map_filename = "genre-map.txt";
    let genre_map = {
        let genre_map_path = Path::new(DEFAULT_ASSETS_DIR).join(genre_map_filename);
        if genre_map_path.exists() {
            //load file
            let mut f = BufReader::new(std::fs::File::open(genre_map_path).unwrap());
            GenreMap::load(&mut f)
        } else {
            //load static asset
            let data = assets::get(genre_map_filename)
                .expect("Genre map not found")
                .content;
            let mut f = BufReader::new(data);
            GenreMap::load(&mut f)
        }
    }
    .unwrap_or_else(|_| {
        eprintln!(
            "{}: {}",
            tr!["Invalid file format", "Неправильный формат файла"],
            genre_map_filename
        );
        std::process::exit(1);
    });

    //////////////////////INDEXING MODE
    if let Some(matches) = matches.subcommand_matches("index") {
        if matches.occurrences_of("language") > 0 {
            if let Some(v) = matches.values_of_lossy("language") {
                settings.langs = v;
            }
        }
        assert!(
            settings.langs.len() > 0,
            "{} {}",
            tr![
                "No language specified nor on command line [--lang=..], nor in settings file",
                "Не указан язык ни в командной строке [--lang=..], ни в файле настроек"
            ],
            settings_filename.display()
        );
        if matches.occurrences_of("stemmer") > 0 {
            if let Some(v) = matches.value_of("stemmer") {
                settings.stemmer = v.to_string();
            }
        }
        let delta = match matches.value_of("INDEX-MODE") {
            Some("full") => false,
            _ => true,
        };
        if matches.is_present("no-body") {
            settings.no_body = true;
        }
        let num_threads = matches
            .value_of("threads")
            .map(|x| x.parse::<usize>().unwrap_or(0));
        let heap_mb_str = matches.value_of("memory").unwrap_or(DEFAULT_HEAP_SIZE);
        let heap_size: usize = heap_mb_str.parse().expect(&format!(
            "{} {}",
            tr!["Invalid memory size", "Некорректный размер"],
            heap_mb_str
        ));
        let batch_size_str = matches.value_of("batch-size").unwrap_or(DEFAULT_BATCH_SIZE);
        let batch_size: usize = batch_size_str.parse().expect(&format!(
            "{} {}",
            tr!["Invalid batch size", "Некорректное число"],
            heap_mb_str
        ));
        //open index
        let mut book_writer = BookWriter::new(
            index_path,
            &settings.stemmer,
            num_threads,
            heap_size * 1024 * 1024,
        )
        .unwrap();
        //save settings with index
        let mut f = std::fs::File::create(&settings_filename).unwrap();
        let json = serde_json::to_string(&settings).unwrap();
        if let Err(e) = f.write(json.as_bytes()) {
            eprintln!(
                "{} {}: {}",
                tr!["Error saving file", "Ошибка сохранения файла"],
                settings_filename.display(),
                e
            );
            std::process::exit(2);
        }
        reindex(
            &mut book_writer,
            &settings,
            &book_formats,
            &genre_map,
            delta,
            batch_size,
            debug,
        );
        std::process::exit(0);
    }

    assert!(
        settings.langs.len() > 0,
        "{} {}",
        tr!["Empty language list in", "Пустой список языков в"],
        settings_filename.display()
    );

    //open index
    let fts = BookReader::new(&index_path, &settings.langs[0]).unwrap_or_else(|e| {
        eprintln!(
            "{} '{}': {}\n{}",
            tr!["Error opening index in", "Ошибка открытия индекса в"],
            index_path.display(),
            e,
            tr![
                "Try to rebuild with 'index full' command",
                "Попробуйте пересоздать индекс командой 'index full'"
            ],
        );
        std::process::exit(4);
    });

    //////////////////////QUERY MODE
    if let Some(matches) = matches.subcommand_matches("query") {
        if let Some(query) = matches.value_of("QUERY-TEXT") {
            let hits_str = matches.value_of("hits").unwrap_or(DEFAULT_QUERY_HITS);
            let hits: usize = hits_str.parse().expect(&format!(
                "{} {}",
                tr!["Invalid number of hits", "Некорректное число"],
                hits_str
            ));
            match fts.search(&query, "default", hits, 0, debug) {
                Ok(res) => {
                    println!("{}", res);
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
                    std::process::exit(2);
                }
            }
        }
        std::process::exit(0);
    }
    if let Some(matches) = matches.subcommand_matches("facet") {
        if let Some(path) = matches.value_of("PATH") {
            match fts.get_facet(&path) {
                Ok(res) => {
                    println!("{}", serde_json::to_string(&res).unwrap());
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
                    std::process::exit(2);
                }
            }
        }
        std::process::exit(0);
    }

    //////////////////////SERVER MODE [default]
    let listen_addr = matches.value_of("listen").unwrap_or(DEFAULT_LISTEN_ADDR);
    println!("{}: {}", tr!["Index dir", "Индекс"], index_path.display());
    println!("{}: {}", tr!["Books dir", "Книги "], books_path.display());
    println!("{}: {:?}", tr!["Language", "Язык"], &settings.langs);
    println!("{}: http://{}/home.html", tr!["Application", "Приложение"], &listen_addr);

    rouille::start_server(&listen_addr, move |req| {
        if debug {
            println!("req {}", req.raw_url())
        }
        let mut req = req;
        let req_no_prefix;

        // map: /home.html -> home.html -> ./static/home.html
        // map: /porcula/home.html -> home.html -> ./static/home.html
        if let Some(r) = req.remove_prefix(DEFAULT_BASE_URL) {
            req_no_prefix = r;
            req = &req_no_prefix;
        }
        let res = rouille::match_assets(&req, DEFAULT_ASSETS_DIR);
        if res.is_success() {
            return res;
        }
        // match included asset
        let url = &req.url();
        let mut maybe_file = url.split('/').skip(1); //skip root /
        if let Some(filename) = maybe_file.next() {
            if let Some(asset) = assets::get(filename) {
                let res =
                    Response::from_data(asset.content_type, asset.content).with_public_cache(86400);
                return res;
            }
        }

        router!(req,
            (GET) (/book/count) => { handler_count(&req, &fts) },
            (GET) (/search) => { handler_search(&req, &fts, debug) },
            (GET) (/facet) => { handler_facet(&req, &fts) },
            (GET) (/genre/translation) => { Response::json(&genre_map.translation) },
            (GET) (/genre/category) => { Response::json(&genre_map.category) },
            (GET) (/book/{zipfile: String}/{filename: String}/cover) => { handler_cover(&req, &fts, &zipfile, &filename) },
            (GET) (/book/{zipfile: String}/{filename: String}/render) => { handler_render(&req, &fts, &books_path, &zipfile, &filename, &book_formats) },
            (GET) (/book/{zipfile: String}/{filename: String}) => { handler_file(&req, &books_path, &zipfile, &filename, &book_formats) },
            (GET) (/book/{zipfile: String}/{filename: String}/{_saveas: String}) => { handler_file(&req, &books_path, &zipfile, &filename, &book_formats) },
            (GET) (/opensearch) => { handler_opensearch_xml(&req) },
            _ => { Response::empty_404() },
        )
    });
}

fn handler_count(_req: &Request, fts: &BookReader) -> Response {
    match &fts.count_all() {
        Ok(count) => Response::text(count.to_string()),
        Err(_) => Response::text("0".to_string()),
    }
}

fn handler_search(req: &Request, fts: &BookReader, debug: bool) -> Response {
    match req.get_param("query") {
        Some(query) => {
            let limit: usize = req
                .get_param("limit")
                .unwrap_or(String::new())
                .parse()
                .unwrap_or(20);
            let offset: usize = req
                .get_param("offset")
                .unwrap_or(String::new())
                .parse()
                .unwrap_or(0);
            let order: String = req.get_param("order").unwrap_or(String::from("default"));
            match fts.search(&query, &order, limit, offset, debug) {
                Ok(json) => Response::from_data("application/json", json),
                Err(e) => Response::text(e.to_string()).with_status_code(500),
            }
        }
        None => Response::empty_404(),
    }
}

fn handler_facet(req: &Request, fts: &BookReader) -> Response {
    match req.get_param("path") {
        Some(path) => match fts.get_facet(&path) {
            Ok(ref data) => Response::json(data),
            Err(e) => Response::text(e.to_string()).with_status_code(500),
        },
        None => Response::empty_404(),
    }
}

fn handler_cover(_req: &Request, fts: &BookReader, zipfile: &str, filename: &str) -> Response {
    match fts.get_cover(zipfile, filename) {
        Ok(Some(img)) if img.len() > 0 => Response::from_data("image/jpeg", img),
        _ => rouille::match_assets(
            &Request::fake_http("GET", DEFAULT_COVER_IMAGE, vec![], vec![]),
            DEFAULT_ASSETS_DIR,
        ),
    }
}

fn handler_render(
    _req: &Request,
    fts: &BookReader,
    books_path: &Path,
    zipfile: &str,
    filename: &str,
    book_formats: &BookFormats,
) -> Response {
    let ext = file_extension(&filename);
    match book_formats.get(&ext.as_ref()) {
        Some(book_format) => {
            let (title, enc) = match fts.get_book_info(zipfile, filename).unwrap() {
                Some(x) => x,
                None => (filename.to_string(), "UTF-8".to_string()), //book not indexed yet, try defaults
            };
            let content = read_zipped_file(books_path, zipfile, filename);
            let coder = encoding::label::encoding_from_whatwg_label(&enc).unwrap();
            let utf8 = coder
                .decode(&content, encoding::DecoderTrap::Ignore)
                .unwrap();
            let content = book_format.str_to_html(&utf8).unwrap(); //result is Vec<u8> but valid UTF-8
            let template = Path::new(DEFAULT_ASSETS_DIR).join("render.html");
            let mut f = std::fs::File::open(template).unwrap();
            let mut buf = String::new();
            f.read_to_string(&mut buf).unwrap();
            let mut html: Vec<u8> = vec![];
            let mut start = 0;
            let substr = "{title}";
            if let Some(found) = buf.find(substr) {
                html.extend_from_slice(buf[start..found].as_bytes());
                html.extend_from_slice(title.as_bytes());
                start = found + substr.len();
            }
            let substr = "{content}";
            if let Some(found) = buf.find(substr) {
                html.extend_from_slice(buf[start..found].as_bytes());
                html.extend_from_slice(&content);
                start = found + substr.len();
            }
            html.extend_from_slice(buf[start..].as_bytes());
            Response::from_data("text/html", html)
        }
        None => Response::empty_404(),
    }
}

fn handler_file(
    _req: &Request,
    books_path: &Path,
    zipfile: &str,
    filename: &str,
    book_formats: &BookFormats,
) -> Response {
    match book_formats.get(&file_extension(filename).as_ref()) {
        Some(book_format) => {
            let content = read_zipped_file(books_path, zipfile, filename);
            Response::from_data(book_format.content_type(), content)
        }
        None => Response::empty_404(),
    }
}

fn handler_opensearch_xml(req: &Request) -> Response {
    let host = match req.header("X-Forwarded-Host") {
        Some(s) => s,
        None => req.header("Host").expect("Unknown server host"),
    };
    let proto = match req.header("X-Forwarded-Proto") {
        Some(s) => s,
        None => {
            if req.is_secure() {
                "https"
            } else {
                "http"
            }
        }
    };
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/">
      <ShortName>Porcula</ShortName>
      <Description>Library search</Description>
      <Url type="text/html" template="{}://{}/porcula/home.html?query={{searchTerms}}"/>  
      <Language>ru-RU</Language>
      <OutputEncoding>UTF-8</OutputEncoding>
      <InputEncoding>UTF-8</InputEncoding>
    </OpenSearchDescription>"#,
        proto, host
    );
    Response::from_data("application/xml", content)
}

fn read_zipped_file(books_path: &Path, zipfile: &str, filename: &str) -> Vec<u8> {
    let zip_path = books_path.join(zipfile);
    let reader = std::fs::File::open(zip_path).unwrap();
    let buffered = std::io::BufReader::new(reader);
    let mut zip = zip::ZipArchive::new(buffered).unwrap();
    let mut file = zip.by_name(filename).unwrap();
    let mut content = vec![];
    file.read_to_end(&mut content).unwrap();
    content
}

fn is_zip_file(entry: &DirEntry) -> bool {
    entry.metadata().map(|e| e.is_file()).unwrap_or(false)
        && file_extension(entry.file_name().to_str().unwrap_or("")) == ".zip"
}

fn reindex(
    book_writer: &mut BookWriter,
    settings: &Settings,
    book_formats: &BookFormats,
    genre_map: &GenreMap,
    delta: bool,
    batch_size: usize,
    debug: bool,
) {
    let tt = std::time::Instant::now();
    let mut lang_set = HashSet::<String>::new();
    let mut any_lang = false;
    for i in &settings.langs {
        lang_set.insert(i.clone());
        if i == "ANY" {
            any_lang = true
        }
    }
    println!(
        "-----START INDEXING dir={} delta={} lang={:?} stemmer={} no_body={} files={:?}",
        &settings.books_dir,
        delta,
        &lang_set,
        &settings.stemmer,
        settings.no_body,
        book_formats.keys()
    );
    let mut zip_files: Vec<DirEntry> = std::fs::read_dir(&settings.books_dir)
        .expect("directory not readable")
        .map(|x| x.expect("invalid file"))
        .filter(is_zip_file)
        .collect();
    zip_files.sort_by_key(|e| e.file_name());
    let zip_count = zip_files.len();
    let mut zip_index = 0;
    let mut book_count = 0;
    let mut book_indexed = 0;
    let mut book_ignored = 0;
    let mut book_skipped = 0;
    let mut error_count = 0;
    let mut warning_count = 0;
    let mut time_to_open_zip = 0;
    let mut time_to_parse = 0;
    let mut time_to_image = 0;
    let mut time_to_doc = 0;
    let mut time_to_commit = 0;

    if !delta {
        println!("deleting index...");
        book_writer.delete_all_books().unwrap();
    }

    for e in zip_files {
        let zt = Instant::now();
        let os_filename = &e.file_name();
        let zipfile = &os_filename.to_str().expect("invalid filename");
        if delta {
            if let Ok(true) = book_writer.is_book_indexed(&zipfile, "WHOLE") {
                println!("[{}/{}] skip archive {}", zip_index, zip_count, &zipfile);
                zip_index += 1;
                continue;
            }
        }
        println!("[{}/{}] read archive {}", zip_index, zip_count, &zipfile);
        let reader = std::fs::File::open(&e.path()).unwrap();
        let buffered = std::io::BufReader::new(reader);
        let mut zip = zip::ZipArchive::new(buffered).unwrap();
        let files_count = zip.len();
        time_to_open_zip += zt.elapsed().as_millis();
        let mut book_in_batch = 0;
        for file_index in 0..files_count {
            let file = zip.by_index(file_index).unwrap();
            let filename: String = match decode_filename(file.name_raw()) {
                Some(s) => s,
                None => file.name().into(),
            };
            let ext = file_extension(&filename);
            if let Some(book_format) = book_formats.get(&ext.as_ref()) {
                //filter eBook by extension
                book_count += 1;
                if delta {
                    if let Ok(true) = book_writer.is_book_indexed(&zipfile, &filename) {
                        println!("  {} indexed", &filename);
                        book_skipped += 1;
                        continue;
                    }
                }
                println!(
                    "[{}%/{}%] {}/{}",
                    file_index * 100 / files_count,
                    zip_index * 100 / zip_count,
                    &zipfile,
                    &filename
                );
                let mut buf_file = BufReader::new(file);
                let pt = Instant::now();
                let parsed_book =
                    book_format.parse(&zipfile, &filename, &mut buf_file, settings.no_body);
                time_to_parse += pt.elapsed().as_millis();
                match parsed_book {
                    Ok(mut b) => {
                        warning_count += b.warning.len();
                        if debug {
                            println!("    -> {}", &b)
                        }
                        if any_lang || (b.lang.len() > 0 && lang_set.get(&b.lang[0]).is_some()) {
                            if let Some(img) = b.cover_image {
                                let it = Instant::now();
                                match img_resizer::resize(
                                    &img.as_slice(),
                                    COVER_IMAGE_WIDTH,
                                    COVER_IMAGE_HEIGHT,
                                ) {
                                    Ok(resized) => b.cover_image = Some(resized),
                                    Err(e) => {
                                        eprintln!(
                                            "{}/{} -> {} {}",
                                            zipfile,
                                            filename,
                                            tr!["image resize error", "ошибка изображения"],
                                            e
                                        );
                                        warning_count += 1;
                                        b.cover_image = None;
                                    }
                                }
                                time_to_image += it.elapsed().as_millis();
                            }

                            let at = Instant::now();
                            match book_writer.add_book(b, &genre_map.category) {
                                Ok(_) => book_indexed += 1,
                                Err(e) => eprintln!(
                                    "{}/{} -> {} {}",
                                    zipfile,
                                    filename,
                                    tr!["indexing error", "ошибка индексации"],
                                    e
                                ), //and continue
                            }
                            time_to_doc += at.elapsed().as_millis();
                            book_in_batch += 1;
                            if (book_in_batch % batch_size) == 0 {
                                if debug {
                                    println!("Commit: start");
                                }
                                let ct = Instant::now();
                                book_writer.commit().unwrap();
                                time_to_commit += ct.elapsed().as_millis();
                                if debug {
                                    println!("Commit: done");
                                }
                            }
                        } else {
                            book_ignored += 1;
                            println!(
                                "         -> {} {}",
                                tr!["ignore lang", "игнорируем язык"],
                                b.lang.iter().next().unwrap_or(&String::new())
                            );
                        }
                    }
                    Err(e) => {
                        error_count += 1;
                        eprintln!(
                            "{}/{} -> {} {}",
                            zipfile,
                            filename,
                            tr!["parse error", "ошибка разбора"],
                            e
                        );
                        //and continue
                    }
                }
            }
        }
        book_writer
            .add_file_record(&zipfile, "WHOLE", book_indexed)
            .unwrap_or(()); //mark whole archive as indexed
        if debug {
            println!("Commit: start");
        }
        let ct = Instant::now();
        book_writer.commit().unwrap();
        time_to_commit += ct.elapsed().as_millis();
        if debug {
            println!("Commit: done");
        }
        zip_index += 1;
    }
    println!("{}", tr!["Indexing: done", "Индексация завершена"]);
    println!(
        "{}: {}/{}",
        tr!["Archives", "Архивов"],
        zip_index,
        zip_count
    );
    println!(
        "{}: {}/{}, {} {}, {} {}",
        tr!["Books", "Книг"],
        book_indexed,
        book_count,
        book_ignored,
        tr!["ignored", "проигнорировано"],
        book_skipped,
        tr!["skipped", "пропущено"]
    );
    println!(
        "{}: {}, {}: {}",
        tr!["Errors", "Ошибок"],
        error_count,
        tr!["Warnings", "Предупреждений"],
        warning_count,
    );
    if debug {
        let total = tt.elapsed().as_millis();
        println!("Main thread: elapsed {}m {}s archive open {}%, parse {}%, image resize {}%, create document {}%, commit {}%",
            total/1000/60, total/1000-(total/1000/60)*60,
            time_to_open_zip*100/total,
            time_to_parse*100/total,
            time_to_image*100/total,
            time_to_doc*100/total,
            time_to_commit*100/total,
        );
    }
}

fn decode_filename(raw_filename: &[u8]) -> Option<String> {
    let (charset, confidence, _language) = chardet::detect(raw_filename);
    if confidence > 0.8 {
        if let Some(coder) =
            encoding::label::encoding_from_whatwg_label(chardet::charset2encoding(&charset))
        {
            if let Ok(utf8) = coder.decode(raw_filename, encoding::DecoderTrap::Ignore) {
                return Some(utf8);
            }
        }
    }
    None
}

fn file_extension(s: &str) -> String {
    match s.rfind('.') {
        Some(i) => s[i..].to_lowercase(),
        None => String::new(),
    }
}
