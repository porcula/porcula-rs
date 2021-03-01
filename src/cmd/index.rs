use clap::ArgMatches;
use crossbeam_utils::sync::WaitGroup;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter, Result};
use std::fs::DirEntry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cmd::*;
use crate::tr;
use crate::types::Book;

//const READ_BUFFER_SIZE: usize = 8 * 1024 * 1024;

#[derive(Debug)]
struct ZipPart {
    path: PathBuf,
    zipfile: String,
    first_idx: usize,
    last_idx: usize,
    pct: u64,
    total_files: u64,
}

#[derive(Default)]
struct ZipPartStats {
    zipfile: String,
    packed_size: u64,
    time_to_unzip: Duration,
    error_count: usize,
}

struct UnzippedFile {
    zipfile: String,
    filename: String,
    data: Vec<u8>,
    pct: u64,
    total_files: u64,
}

enum BookState {
    Valid(Book),
    Invalid,
    Skipped,
    Ignored,
}
impl Display for BookState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            BookState::Valid(_) => write!(f, "Valid"),
            BookState::Invalid => write!(f, "Invalid"),
            BookState::Skipped => write!(f, "Skipped"),
            BookState::Ignored => write!(f, "Ignored"),
        }
    }
}

struct ParsedBook {
    zipfile: String,
    filename: String,
    state: BookState,
    error_count: usize,
    warning_count: usize,
    readed_size: usize,
    parsed_size: usize,
    content_size: usize,
    time_to_parse: Duration,
    time_to_image: Duration,
    total_files: u64,
}
impl Default for ParsedBook {
    fn default() -> Self {
        ParsedBook {
            zipfile: String::default(),
            filename: String::default(),
            state: BookState::Invalid,
            error_count: 0,
            warning_count: 0,
            readed_size: 0,
            parsed_size: 0,
            content_size: 0,
            time_to_parse: Duration::default(),
            time_to_image: Duration::default(),
            total_files: 0,
        }
    }
}

#[derive(Clone)]
struct ParseOpts {
    body: bool,
    xbody: bool,
    annotation: bool,
    cover: bool,
}

#[allow(clippy::cognitive_complexity)]
pub fn run_index(matches: &ArgMatches, app: &mut Application) {
    let debug = app.debug;
    if matches.occurrences_of("language") > 0 {
        if let Some(v) = matches.values_of_lossy("language") {
            app.index_settings.langs = v;
        }
    }
    assert!(
        !app.index_settings.langs.is_empty(),
        "{} {}",
        tr![
            "No language specified nor on command line [--lang], nor in settings file",
            "Не указан язык ни в командной строке [--lang], ни в файле настроек"
        ],
        INDEX_SETTINGS_FILE
    );
    let files: Vec<&std::ffi::OsStr> = matches.values_of_os("file").unwrap_or_default().collect();
    if matches.occurrences_of("stemmer") > 0 {
        if let Some(v) = matches.value_of("stemmer") {
            app.index_settings.stemmer = v.to_string();
        }
    }
    let delta = !matches!(matches.value_of("INDEX-MODE"), Some("full"));
    for i in &["body", "xbody", "annotation", "cover"] {
        let s = (*i).to_string();
        if matches.is_present(format!("with-{}", i)) {
            app.index_settings.disabled.remove(&s); //enabling field
        }
        if matches.is_present(format!("without-{}", i)) {
            app.index_settings.disabled.insert(s); //disabling field
        }
    }
    let index_threads = matches
        .value_of("index-threads")
        .map(|x| x.parse::<usize>().unwrap_or(0));
    let read_threads = matches
        .value_of("read-threads")
        .map(|x| x.parse::<usize>().unwrap_or(1))
        .unwrap_or(1);
    let read_queue = matches
        .value_of("read-queue")
        .map(|x| x.parse::<usize>().unwrap_or(64))
        .unwrap_or(64);
    let heap_mb_str = matches.value_of("memory").unwrap_or(DEFAULT_HEAP_SIZE_MB);
    let heap_size = 1024
        * 1024
        * heap_mb_str.parse::<usize>().unwrap_or_else(|_| {
            eprintln!(
                "{} {}",
                tr!["Invalid memory size", "Некорректный размер"],
                heap_mb_str
            );
            std::process::exit(4);
        });
    let batch_mb_str = matches
        .value_of("batch-size")
        .unwrap_or(DEFAULT_BATCH_SIZE_MB);
    let batch_size = 1024
        * 1024
        * batch_mb_str.parse::<usize>().unwrap_or_else(|_| {
            eprintln!(
                "{} {}",
                tr!["Invalid memory size", "Некорректный размер"],
                batch_mb_str
            );
            std::process::exit(4);
        });
    app.load_genre_map();
    let genre_map = &app.genre_map;
    let book_formats = &app.book_formats;

    let mut lang_set = HashSet::<String>::new();
    let mut any_lang = false;
    for i in &app.index_settings.langs {
        lang_set.insert(i.to_string());
        if i == "ANY" {
            any_lang = true
        }
    }
    //index books with `undefined` language too
    let lang_filter = |lang: &str| any_lang || lang_set.contains(lang) || lang.is_empty();
    let opts = ParseOpts {
        body: !app.index_settings.disabled.contains("body"),
        xbody: !app.index_settings.disabled.contains("xbody"),
        annotation: !app.index_settings.disabled.contains("annotation"),
        cover: !app.index_settings.disabled.contains("cover"),
    };

    println!(
        "----{}----\ndir={} delta={} lang={:?} stemmer={} body={} xbody={} annotation={} cover={} files={:?}",
        tr!["START INDEXING","НАЧИНАЕМ ИНДЕКСАЦИЮ"],
        &app.books_path.display(),
        delta,
        &lang_set,
        &app.index_settings.stemmer,
        opts.body, opts.xbody, opts.annotation, opts.cover,
        app.book_formats.keys()
    );
    if debug {
        println!(
            "read threads={} read queue={} index threads={:?} heap={} batch={}",
            read_threads, read_queue, index_threads, heap_size, batch_size,
        );
    }
    //save settings with index
    if debug {
        println!("store settings in {}", app.index_path.display());
    }
    app.index_settings
        .save(&app.index_path)
        .unwrap_or_else(|e| {
            eprintln!("{}", e);
            std::process::exit(2);
        });

    //open index
    let mut book_writer = crate::fts::BookWriter::new(
        &app.index_path,
        &app.index_settings.stemmer,
        index_threads,
        heap_size,
    )
    .unwrap();
    //enforce reindex of books inside specified files
    let indexed_books = match files.is_empty() && delta {
        true => {
            if debug {
                println!("loading list of indexed files");
            }
            Some(book_writer.get_indexed_books().unwrap()) //read ALL indexed file names as two-level hash: zipfile->{filenames}
        }
        false => None,
    };
    //println!("DEBUG indexed_books={:?}", indexed_books);

    let mut zip_files: Vec<DirEntry> = std::fs::read_dir(&app.books_path)
        .expect("directory not readable")
        .map(|x| x.expect("invalid file"))
        .filter(is_zip_file)
        .filter(|x| files.is_empty() || files.contains(&x.file_name().as_os_str()))
        .collect();
    zip_files.sort_by_key(|x| get_numeric_sort_key(x.file_name().to_str().unwrap_or_default()));
    let zip_total_count = zip_files.len();
    let zip_total_size = zip_files.iter().fold(0, |acc, entry| {
        acc + entry.metadata().map(|m| m.len()).unwrap_or(0)
    });

    if !delta {
        println!("{}", tr!["deleting index...", "очищаем индекс..."]);
        book_writer.delete_all_books().unwrap();
    }

    let (zippart_send, zippart_recv) = crossbeam_channel::unbounded::<ZipPart>();
    let (zipstat_send, zipstat_recv) = crossbeam_channel::unbounded::<ZipPartStats>();
    let (file_send, file_recv) = crossbeam_channel::bounded::<Option<UnzippedFile>>(read_queue);
    let (book_send, book_recv) = crossbeam_channel::bounded::<Option<ParsedBook>>(read_queue);
    let unzip_wait_group = WaitGroup::new();
    let parse_wait_group = WaitGroup::new();

    //exit nicely if user press Ctrl+C
    let canceled = Arc::new(AtomicBool::new(false));
    let c = canceled.clone();
    ctrlc::set_handler(move || {
        eprintln!("Cancel indexing...");
        c.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    crossbeam_utils::thread::scope(|scope| {
        let tt = Instant::now();

        //single unzip distributor thread : zip-files -> zippart
        let unzip_thread = {
            let canceled = canceled.clone();
            let indexed_books = &indexed_books;
            scope.spawn(move |_| {
                let mut zip_progress_size = 0;
                let mut zip_queued = 0;
                if debug {
                    println!("start unzip distributor, files={}", zip_files.len());
                }
                for entry in zip_files {
                    if canceled.load(Ordering::SeqCst) {
                        break;
                    }
                    let os_filename = &entry.file_name();
                    let zip_size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let zip_progress_pct = zip_progress_size * 100 / zip_total_size;
                    let zipfile = os_filename.to_str().expect("invalid filename");
                    if let Some(indexed) = &indexed_books {
                        if let Some(files) = indexed.get(zipfile) {
                            if files.contains(crate::fts::WHOLE_MARKER) {
                                println!(
                                    "[{}/{}] {} {}",
                                    zip_queued,
                                    zip_total_count,
                                    tr!["skip archive", "пропускаем архив"],
                                    &zipfile
                                );
                                zip_progress_size += zip_size;
                                continue;
                            }
                        }
                    }
                    println!(
                        "[{}/{}={}%] {} {}",
                        zip_queued,
                        zip_total_count,
                        zip_progress_pct,
                        tr!["read archive", "читаем архив"],
                        &zipfile
                    );
                    let reader = std::fs::File::open(&entry.path()).unwrap();
                    //let reader = std::io::BufReader::with_capacity(READ_BUFFER_SIZE, reader);
                    let zip = zip::ZipArchive::new(reader).unwrap();
                    let files_count = zip.len();
                    //assume large zip-file with many books inside, 1-3 MB each
                    //reopen zip and read different parts in multiple threads
                    let read_threads = if files_count <= read_threads {
                        1
                    } else {
                        read_threads
                    };
                    let chunk_size = files_count / read_threads;
                    let mut first_idx = 0;
                    let mut last_idx = chunk_size;
                    for i in (0..read_threads).rev() {
                        if i == 0 {
                            last_idx = files_count
                        }
                        let zip_part = ZipPart {
                            path: entry.path(),
                            zipfile: zipfile.to_string(),
                            first_idx,
                            last_idx,
                            pct: zip_progress_pct,
                            total_files: files_count as u64,
                        };
                        if zippart_send.send(zip_part).is_err() {
                            break;
                        }
                        first_idx += chunk_size;
                        last_idx += chunk_size;
                    }
                    zip_queued += 1;
                    zip_progress_size += zip_size;
                }
                drop(zippart_send);
                if debug {
                    println!("stop unzip distributor");
                }
            })
        };

        //worker unzip threads : zippart -> file | skipped-book
        for thread in 0..read_threads {
            let canceled = canceled.clone();
            let unzip_wait_group = unzip_wait_group.clone();
            let zippart_recv = zippart_recv.clone();
            let zipstat_send = zipstat_send.clone();
            let book_send = book_send.clone();
            let file_send = file_send.clone();
            let indexed_books = &indexed_books;
            scope.spawn(move |_| {
                if debug {
                    println!("z#{} start", thread);
                }
                for zip_part in zippart_recv.iter() {
                    if canceled.load(Ordering::SeqCst) {
                        break;
                    }
                    if debug {
                        println!("z#{}: {:?}", thread, zip_part);
                    }
                    let zipfile = &zip_part.zipfile;
                    let mut stats = ZipPartStats {
                        zipfile: zipfile.clone(),
                        ..Default::default()
                    };
                    let zt = Instant::now();
                    let reader = std::fs::File::open(&zip_part.path).unwrap();
                    let mut zip = zip::ZipArchive::new(reader).unwrap();
                    stats.time_to_unzip += zt.elapsed();
                    for i in zip_part.first_idx..zip_part.last_idx {
                        if canceled.load(Ordering::SeqCst) {
                            break;
                        }
                        let mut file = zip.by_index(i).unwrap();
                        stats.packed_size += file.compressed_size();
                        let filename: String = match decode_filename(file.name_raw()) {
                            Some(s) => s,
                            None => file.name().into(),
                        };
                        if debug {
                            println!(
                                "z#{} [{}%] {}/{}",
                                thread, zip_part.pct, &zipfile, &filename
                            );
                        }
                        if let Some(indexed) = &indexed_books {
                            if let Some(files) = indexed.get(zipfile) {
                                if files.contains(&filename) {
                                    println!("  {} {}", &filename, tr!["indexed", "индексирован"]);
                                    let book = ParsedBook {
                                        zipfile: zipfile.to_string(),
                                        filename,
                                        state: BookState::Skipped,
                                        total_files: zip_part.total_files,
                                        ..Default::default()
                                    };
                                    if book_send.send(Some(book)).is_err() {
                                        break;
                                    }
                                    continue;
                                }
                            }
                        }
                        let zt = Instant::now();
                        let mut data = Vec::with_capacity(file.size() as usize);
                        let file = file.read_to_end(&mut data);
                        stats.time_to_unzip += zt.elapsed();
                        match file {
                            Ok(_) => {
                                let file = UnzippedFile {
                                    zipfile: zipfile.clone(),
                                    filename,
                                    data,
                                    pct: zip_part.pct,
                                    total_files: zip_part.total_files,
                                };
                                if file_send.send(Some(file)).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                eprintln!("error reading {}/{}: {}", &zipfile, &filename, e);
                                stats.error_count += 1;
                            }
                        }
                    }
                    if zipstat_send.send(stats).is_err() {
                        break;
                    }
                }
                if debug {
                    println!("z#{} stop", thread);
                }
                //end-signal to parse thread
                file_send.send(None).ok();
                drop(zipstat_send);
                drop(file_send);
                drop(book_send);
                drop(unzip_wait_group);
            });
        }
        drop(zipstat_send);
        drop(zippart_recv);

        //worker threads (book parsing + image processing) : file -> book
        for thread in 0..read_threads {
            let book_send = book_send.clone();
            let file_recv = file_recv.clone();
            let parse_wait_group = parse_wait_group.clone();
            let canceled = canceled.clone();
            let opts = opts.clone();
            scope.spawn(move |_| {
                if debug {
                    println!("p#{} start", thread);
                }
                for file in file_recv.iter() {
                    if canceled.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Some(file) = file {
                        if debug {
                            println!(
                                "p#{} [{}%] parse {}/{}",
                                thread, file.pct, &file.zipfile, &file.filename
                            );
                        }
                        let book = process_file(file, lang_filter, &book_formats, &opts, debug);
                        if book_send.send(Some(book)).is_err() {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                if debug {
                    println!("p#{} stop", thread);
                }
                drop(book_send);
                drop(parse_wait_group);
            });
        }
        drop(book_send);
        drop(file_recv);

        //main thread : book -> index, commit, join unzip threads, join parse threads
        let mut book_indexed = 0;
        let mut book_ignored = 0;
        let mut book_skipped = 0;
        let mut packed_size = 0;
        let mut book_readed_size = 0;
        let mut book_parsed_size = 0;
        let mut error_count = 0;
        let mut warning_count = 0;
        let mut time_to_unzip = Duration::default();
        let mut time_to_parse = Duration::default();
        let mut time_to_image = Duration::default();
        let mut time_to_commit = Duration::default();
        let mut running_indexed_size: usize = 0;
        let mut processed_files: HashMap<String, (u64, u64)> = HashMap::new(); //zipfile->(total,indexed)

        for book in book_recv.iter() {
            if canceled.load(Ordering::SeqCst) {
                break;
            }
            if let Some(book) = book {
                if debug {
                    println!("  {}/{} -> {}", book.zipfile, book.filename, book.state);
                }
                let mut indexed = 0;
                match book.state {
                    BookState::Invalid => (),
                    BookState::Skipped => book_skipped += 1,
                    BookState::Ignored => book_ignored += 1,
                    BookState::Valid(b) => {
                        match book_writer.add_book(
                            &book.zipfile,
                            &book.filename,
                            b,
                            &genre_map,
                            opts.body,
                            opts.xbody,
                        ) {
                            Ok(_) => {
                                book_indexed += 1;
                                running_indexed_size += book.parsed_size;
                                indexed = 1;
                            }
                            Err(e) => {
                                error_count += 1;
                                eprintln!(
                                    "{}/{} -> {} {}",
                                    book.zipfile,
                                    book.filename,
                                    tr!["indexing error", "ошибка индексации"],
                                    e
                                );
                                //and continue
                            }
                        }
                    }
                }
                processed_files
                    .entry(book.zipfile.clone())
                    .and_modify(|x| {
                        (*x).0 += 1;
                        (*x).1 += indexed;
                    })
                    .or_insert((1, indexed));
                //mark whole archives as indexed when all files are done
                if let Some((processed, indexed)) = processed_files.get(&book.zipfile) {
                    if *processed == book.total_files {
                        if debug {
                            println!("mark {} as indexed", book.zipfile);
                        }
                        book_writer
                            .mark_zipfile_as_indexed(&book.zipfile, *indexed)
                            .unwrap();
                    }
                }
                error_count += book.error_count;
                warning_count += book.warning_count;
                book_readed_size += book.readed_size;
                book_parsed_size += book.parsed_size;
                time_to_parse += book.time_to_parse;
                time_to_image += book.time_to_image;
                if running_indexed_size > batch_size {
                    running_indexed_size = 0;
                    if debug {
                        println!("Batch commit: start");
                    }
                    let ct = Instant::now();
                    book_writer.commit().unwrap();
                    time_to_commit += ct.elapsed();
                    if debug {
                        println!("Batch commit: done");
                    }
                }
            } else {
                break;
            }
        }

        if debug {
            println!("stopping...");
        }

        if debug {
            println!("unzip_thread.join");
        }
        unzip_thread.join().ok();

        if debug {
            println!("unzip_wait_group.wait");
        }
        unzip_wait_group.wait();
        if debug {
            println!("collect zip statistics");
        }
        let mut uniq_zipfile = HashSet::new();
        for s in zipstat_recv.try_iter() {
            uniq_zipfile.insert(s.zipfile);
            packed_size += s.packed_size;
            time_to_unzip += s.time_to_unzip;
            error_count += s.error_count;
        }
        drop(zipstat_recv);
        let zip_processed = uniq_zipfile.len();

        //empty messages to stop parse threads
        if debug {
            println!("stopping read threads");
        }
        for _ in 0..read_threads {
            file_send.try_send(None).unwrap_or_default();
        }

        if debug {
            println!("parse_wait_group.wait");
        }
        parse_wait_group.wait();

        if debug {
            println!("Commit: start");
        }
        let ct = Instant::now();
        book_writer.commit().unwrap();
        time_to_commit += ct.elapsed();
        if debug {
            println!("Commit: done, waiting for merging threads");
        }
        book_writer.wait_merging_threads().unwrap();

        let total = tt.elapsed().as_millis() + 1;
        let canceled = canceled.load(Ordering::SeqCst);
        if canceled {
            println!("{}", tr!["Indexing canceled", "Индексация прервана"]);
        } else {
            println!("{}", tr!["Indexing done", "Индексация завершена"]);
        }

        println!(
            "{}: {}/{} = {}/{} MB",
            tr!["Archives", "Архивов"],
            zip_processed,
            zip_total_count,
            packed_size / 1024 / 1024,
            zip_total_size / 1024 / 1024,
        );
        println!(
            "{}: {} {}: {} {}, {} {} = {} {} / {} {}",
            tr!["Books", "Книг"],
            book_indexed,
            tr!["added", "добавлено"],
            book_ignored,
            tr!["ignored", "проигнорировано"],
            book_skipped,
            tr!["skipped", "пропущено"],
            book_parsed_size / 1024 / 1024,
            tr!["MB indexed", "МБ проиндексировано"],
            book_readed_size / 1024 / 1024,
            tr!["MB readed", "МБ прочитано"],
        );
        println!(
            "{}: {} {}: {} MB/s",
            tr!["Duration", "Длительность"],
            format_duration(total),
            tr!["Average speed", "Средняя скорость"],
            (book_readed_size as u128) / total * 1000 / 1024 / 1024,
        );
        println!(
            "{}: {}, {}: {}",
            tr!["Errors", "Ошибок"],
            error_count,
            tr!["Warnings", "Предупреждений"],
            warning_count,
        );
        if debug {
            let ue = time_to_unzip.as_millis() / read_threads as u128;
            let pe = time_to_parse.as_millis() / read_threads as u128;
            let ie = time_to_image.as_millis() / read_threads as u128;
            let ce = time_to_commit.as_millis();
            println!(
                "unpacking {}%, parse {}%, image resize {}%, commit {}%",
                ue * 100 / total,
                pe * 100 / total,
                ie * 100 / total,
                ce * 100 / total,
            );
        }
    })
    .unwrap(); //scope
}

// extract number from string and left-pad it
lazy_static! {
    static ref RE_NUMBER: Regex = Regex::new(r"[0-9]{2,9}").unwrap();
}
fn get_numeric_sort_key(filename: &str) -> String {
    match RE_NUMBER.find(filename) {
        Some(n) => format!("{:0>9}", n.as_str()),
        None => filename.to_string(),
    }
}

#[test]
fn test_get_numeric_sort_key() {
    assert_eq!(get_numeric_sort_key("ab123cd45ef"), "000000123");
    let mut a = vec!["b", "a", "c345", "d12345", "x001"];
    a.sort_by_key(|x| get_numeric_sort_key(x));
    assert_eq!(a, vec!["x001", "c345", "d12345", "a", "b"]);
}

fn format_duration(ms: u128) -> String {
    let mut s = ms / 1000;
    let h = s / 60 / 60;
    s -= h * 60 * 60;
    let m = s / 60;
    s -= m * 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
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

fn is_zip_file(entry: &DirEntry) -> bool {
    entry.metadata().map(|e| e.is_file()).unwrap_or(false)
        && file_extension(entry.file_name().to_str().unwrap_or("")) == ".zip"
}

fn process_file<F>(
    file: UnzippedFile,
    lang_filter: F,
    book_formats: &BookFormats,
    opts: &ParseOpts,
    debug: bool,
) -> ParsedBook
where
    F: Fn(&str) -> bool,
{
    let mut res = ParsedBook {
        zipfile: file.zipfile.clone(),
        filename: file.filename.clone(),
        total_files: file.total_files,
        ..Default::default()
    };
    let ext = file_extension(&file.filename);
    if let Some(book_format) = book_formats.get(&ext.as_ref()) {
        //filter eBook by extension
        res.readed_size = file.data.len(); //uncompressed book size with embedded images
        let mut buf_file = std::io::Cursor::new(file.data);
        let pt = Instant::now();
        let parsed_book = book_format.parse(
            &mut buf_file,
            opts.body || opts.xbody,
            opts.annotation,
            opts.cover,
        );
        res.time_to_parse = pt.elapsed();
        match parsed_book {
            Ok(mut b) => {
                res.warning_count += b.warning.len();
                if debug {
                    println!("  {}/{} -> {}", file.zipfile, file.filename, &b)
                }
                let lang = if !b.lang.is_empty() { &b.lang[0] } else { "" };
                if lang_filter(&lang) {
                    if let Some(img) = b.cover_image {
                        let it = Instant::now();
                        match crate::img_resizer::resize(
                            &img.as_slice(),
                            COVER_IMAGE_WIDTH,
                            COVER_IMAGE_HEIGHT,
                        ) {
                            Ok(resized) => b.cover_image = Some(resized),
                            Err(e) => {
                                eprintln!(
                                    "{}/{} -> {} {}",
                                    file.zipfile,
                                    file.filename,
                                    tr!["image resize error", "ошибка изображения"],
                                    e
                                );
                                res.warning_count += 1;
                                b.cover_image = None;
                            }
                        }
                        res.time_to_image = it.elapsed();
                    }
                    res.parsed_size = b.size_of(); //metadata + plain text + cover image
                    if !opts.body && !opts.xbody {
                        b.length = res.parsed_size as u64;
                    }
                    res.content_size = match b.body {
                        Some(ref x) => x.len(),
                        None => 0,
                    };
                    res.state = BookState::Valid(b);
                } else {
                    res.state = BookState::Ignored;
                    println!(
                        "{}/{} -> {} {}",
                        file.zipfile,
                        file.filename,
                        tr!["ignore lang", "игнорируем язык"],
                        lang
                    );
                }
            }
            Err(e) => {
                res.state = BookState::Invalid;
                res.error_count += 1;
                eprintln!(
                    "{}/{} -> {} {}",
                    file.zipfile,
                    file.filename,
                    tr!["parse error", "ошибка разбора"],
                    e
                );
                //and continue
            }
        }
    }
    res
}
