use clap::ArgMatches;
use rayon::prelude::*;
use regex::Regex;
use std::collections::HashSet;
use std::fs::DirEntry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cmd::*;
use crate::tr;
use crate::types::Book;

//const READ_BUFFER_SIZE: usize = 8 * 1024 * 1024;

#[derive(Default, Clone)]
struct ProcessStats {
    error_count: usize,
    warning_count: usize,
    packed_size: usize,
    unpacked_size: usize,
    parsed_size: usize,
    book_total: usize,
    book_skipped: usize,
    book_ignored: usize,
    time_to_unzip: Duration,
    time_to_parse: Duration,
    time_to_image: Duration,
}

impl std::ops::Add for ProcessStats {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self {
            error_count: self.error_count + other.error_count,
            warning_count: self.warning_count + other.warning_count,
            packed_size: self.packed_size + other.packed_size,
            unpacked_size: self.unpacked_size + other.unpacked_size,
            parsed_size: self.parsed_size + other.parsed_size,
            book_total: self.book_total + other.book_total,
            book_skipped: self.book_skipped + other.book_skipped,
            book_ignored: self.book_ignored + other.book_ignored,
            time_to_unzip: self.time_to_unzip + other.time_to_unzip,
            time_to_parse: self.time_to_parse + other.time_to_parse,
            time_to_image: self.time_to_image + other.time_to_image,
        }
    }
}

enum BookState {
    Invalid,
    Ignored,
    WholeZip,
    Valid(Box<Book>),
}

impl Default for BookState {
    fn default() -> Self {
        BookState::Ignored
    }
}

#[derive(Default)]
struct ParsedBook {
    state: BookState,
    zipfile: String,
    filename: String,
    warning_count: usize,
    parsed_size: usize,
    time_to_parse: Duration,
    time_to_image: Duration,
}

#[derive(Default)]
struct CommitStats {
    book_indexed: usize,
    error_count: usize,
    time_to_commit: Duration,
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
            let book_reader = app.open_book_reader().unwrap();
            Some(book_reader.get_indexed_books(true).unwrap()) //read ALL indexed file names as two-level hash: zipfile->{filenames}
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

    //exit nicely if user press Ctrl+C
    let canceled = Arc::new(AtomicBool::new(false));
    let c = canceled.clone();
    ctrlc::set_handler(move || {
        eprintln!("Cancel indexing...");
        c.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let (send_book, recv_book) = crossbeam_channel::bounded::<ParsedBook>(read_queue);

    //single commit-thread
    crossbeam_utils::thread::scope(|scope| {
        let tt = Instant::now();

        let commit_canceled = canceled.clone();
        let opts_body = opts.body;
        let opts_xbody = opts.xbody;
        let commit_thread = scope.spawn(move |_| {
            let mut stats = CommitStats::default();
            let mut uncommited_size = 0;
            for entry in recv_book.iter() {
                if commit_canceled.load(Ordering::SeqCst) {
                    break;
                }
                match entry.state {
                    BookState::Valid(book) => {
                        match book_writer.add_book(
                            &entry.zipfile,
                            &entry.filename,
                            *book,
                            &genre_map,
                            opts_body,
                            opts_xbody,
                        ) {
                            Ok(_) => {
                                stats.book_indexed += 1;
                                uncommited_size += entry.parsed_size;
                            }
                            Err(e) => {
                                stats.error_count += 1;
                                eprintln!(
                                    "{}/{} -> {} {}",
                                    entry.zipfile,
                                    entry.filename,
                                    tr!["indexing error", "ошибка индексации"],
                                    e
                                );
                                //and continue
                            }
                        }
                    }
                    BookState::WholeZip => {
                        book_writer
                            .mark_zipfile_as_indexed(&entry.zipfile, entry.parsed_size as u64)
                            .unwrap();
                    }
                    _ => (),
                }
                if uncommited_size > batch_size {
                    uncommited_size = 0;
                    if debug {
                        println!("--------------Commit: start");
                    }
                    let ct = Instant::now();
                    book_writer.commit().unwrap();
                    stats.time_to_commit += ct.elapsed();
                    if debug {
                        println!("--------------Commit: done");
                    }
                }
            }
            //final commit
            if debug {
                println!("Final commit: start");
            }
            let ct = Instant::now();
            book_writer.commit().unwrap();
            if debug {
                println!("Waiting for merging threads");
            }
            book_writer.wait_merging_threads().unwrap();
            stats.time_to_commit += ct.elapsed();
            if debug {
                println!("Final commit: done");
            }
            stats
        });

        let mut gstats = ProcessStats::default();
        let mut zip_processed = 0;
        let mut zip_skipped = 0;
        let mut zip_progress_size = 0;
        for (zip_index, entry) in zip_files.iter().enumerate() {
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
                            zip_index + 1,
                            zip_total_count,
                            tr!["skip archive", "пропускаем архив"],
                            &zipfile
                        );
                        zip_skipped += 1;
                        zip_progress_size += zip_size;
                        continue;
                    }
                }
            }
            zip_processed += 1;
            zip_progress_size += zip_size;
            println!(
                "[{}/{}={}%] {} {}",
                zip_index + 1,
                zip_total_count,
                zip_progress_pct,
                tr!["read archive", "читаем архив"],
                &zipfile
            );
            let reader = std::fs::File::open(&entry.path()).unwrap();
            //let reader = std::io::BufReader::with_capacity(READ_BUFFER_SIZE, reader);
            let zip = zip::ZipArchive::new(reader).unwrap();
            let file_count = zip.len();
            let part_size = file_count / read_threads;
            let partitions: Vec<(usize, usize)> = (0..read_threads)
                .map(|i| {
                    let first = i * part_size;
                    let last = if i == read_threads - 1 {
                        file_count
                    } else {
                        first + part_size
                    };
                    (first, last)
                })
                .collect();
            let zip_stats: ProcessStats = partitions
                .into_par_iter()
                .map(|(first, last)| {
                    let reader = std::fs::File::open(&entry.path()).unwrap();
                    let mut zip = zip::ZipArchive::new(reader).unwrap();
                    let mut stats = ProcessStats::default();
                    for i in first..last {
                        if canceled.load(Ordering::SeqCst) {
                            break;
                        }
                        let mut file = zip.by_index(i).unwrap();
                        stats.book_total += 1;
                        stats.packed_size += file.compressed_size() as usize;
                        stats.unpacked_size += file.size() as usize;
                        let filename: String = match decode_filename(file.name_raw()) {
                            Some(s) => s,
                            None => file.name().into(),
                        };
                        if debug {
                            println!("[{}%] {}/{}", zip_progress_pct, &zipfile, &filename);
                        }
                        let mut process_book = true;
                        if let Some(indexed) = &indexed_books {
                            if let Some(files) = indexed.get(zipfile) {
                                if files.contains(&filename) {
                                    println!("  {} {}", &filename, tr!["indexed", "индексирован"]);
                                    stats.book_skipped += 1;
                                    process_book = false;
                                }
                            }
                        }
                        if process_book {
                            let zt = Instant::now();
                            let mut data = Vec::with_capacity(file.size() as usize);
                            file.read_to_end(&mut data).unwrap();
                            stats.time_to_unzip += zt.elapsed();
                            let parsed_book = process_file(
                                zipfile,
                                &filename,
                                data,
                                lang_filter,
                                &book_formats,
                                &opts,
                                debug,
                            );
                            stats.parsed_size += parsed_book.parsed_size;
                            stats.time_to_parse += parsed_book.time_to_parse;
                            stats.time_to_image += parsed_book.time_to_image;
                            match parsed_book.state {
                                BookState::Invalid => stats.error_count += 1,
                                BookState::Ignored => stats.book_ignored += 1,
                                BookState::Valid(_) => {
                                    send_book.send(parsed_book).unwrap_or_else(|e| {
                                        if !canceled.load(Ordering::SeqCst) {
                                            panic!("Error queueing book to index: {}", e);
                                        }
                                    })
                                }
                                _ => (),
                            }
                        }
                    }
                    stats
                })
                .reduce(ProcessStats::default, |a, b| a + b);
            if !canceled.load(Ordering::SeqCst) {
                send_book
                    .send(ParsedBook {
                        state: BookState::WholeZip,
                        zipfile: zipfile.to_string(),
                        parsed_size: zip_stats.book_total,
                        ..Default::default()
                    })
                    .unwrap();
            }
            gstats = gstats + zip_stats;
        }
        drop(send_book);
        let cstats = commit_thread.join().unwrap();

        let total = tt.elapsed().as_millis() + 1;
        let canceled = canceled.load(Ordering::SeqCst);
        if canceled {
            println!("{}", tr!["Indexing canceled", "Индексация прервана"]);
        } else {
            println!("{}", tr!["Indexing done", "Индексация завершена"]);
        }

        println!(
            "{}: {}/{}, {} {} = {}/{} MB",
            tr!["Archives", "Архивов"],
            zip_processed,
            zip_total_count,
            zip_skipped,
            tr!["skipped", "пропущено"],
            gstats.packed_size / 1024 / 1024,
            zip_total_size / 1024 / 1024,
        );
        println!(
            "{}: {} {}: {} {}, {} {} = {} {} / {} {}",
            tr!["Books", "Книг"],
            cstats.book_indexed,
            tr!["added", "добавлено"],
            gstats.book_ignored,
            tr!["ignored", "проигнорировано"],
            gstats.book_skipped,
            tr!["skipped", "пропущено"],
            gstats.parsed_size / 1024 / 1024,
            tr!["MB indexed", "МБ проиндексировано"],
            gstats.unpacked_size / 1024 / 1024,
            tr!["MB readed", "МБ прочитано"],
        );
        println!(
            "{}: {} {}: {} MB/s",
            tr!["Duration", "Длительность"],
            format_duration(total),
            tr!["Average speed", "Средняя скорость"],
            (gstats.unpacked_size as u128) / total * 1000 / 1024 / 1024,
        );
        println!(
            "{}: {}, {}: {}",
            tr!["Errors", "Ошибок"],
            gstats.error_count + cstats.error_count,
            tr!["Warnings", "Предупреждений"],
            gstats.warning_count,
        );
        if debug {
            let ue = gstats.time_to_unzip.as_millis() / read_threads as u128;
            let pe = gstats.time_to_parse.as_millis() / read_threads as u128;
            let ie = gstats.time_to_image.as_millis() / read_threads as u128;
            let ce = cstats.time_to_commit.as_millis();
            println!(
                "unpacking {}%, parse {}%, image resize {}%, commit {}%",
                ue * 100 / total,
                pe * 100 / total,
                ie * 100 / total,
                ce * 100 / total,
            );
        }
    })
    .unwrap();
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
    zipfile: &str,
    filename: &str,
    data: Vec<u8>,
    lang_filter: F,
    book_formats: &BookFormats,
    opts: &ParseOpts,
    debug: bool,
) -> ParsedBook
where
    F: Fn(&str) -> bool,
{
    let mut res = ParsedBook {
        zipfile: zipfile.to_string(),
        filename: filename.to_string(),
        ..Default::default()
    };
    let ext = file_extension(filename);
    if let Some(book_format) = book_formats.get(&ext.as_ref()) {
        //filter eBook by extension
        let mut buf_file = std::io::Cursor::new(data);
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
                    println!("  {}/{} -> {}", zipfile, filename, &b)
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
                                    zipfile,
                                    filename,
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
                    res.state = BookState::Valid(Box::new(b));
                } else {
                    res.state = BookState::Ignored;
                    println!(
                        "{}/{} -> {} {}",
                        zipfile,
                        filename,
                        tr!["ignore lang", "игнорируем язык"],
                        lang
                    );
                }
            }
            Err(e) => {
                res.state = BookState::Invalid;
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
    res
}
