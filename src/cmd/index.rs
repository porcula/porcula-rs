use clap::ArgMatches;
use crossbeam_utils::thread;
use std::collections::HashSet;
use std::fs::DirEntry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use regex::Regex;

use crate::cmd::*;
use crate::fts::BookWriter;
use crate::tr;

const READ_BUFFER_SIZE: usize = 2 * 1024 * 1024;

#[derive(Default)]
struct ParsedFileStats {
    is_book: bool,
    skipped: bool,
    ignored: bool,
    indexed: bool,
    error_count: usize,
    warning_count: usize,
    readed_size: usize,
    parsed_size: usize,
    time_to_unzip: Duration,
    time_to_parse: Duration,
    time_to_image: Duration,
}

#[derive(Clone)]
struct ParseOpts<'a> {
    book_formats: &'a BookFormats,
    genre_map: &'a GenreMap,
    debug: bool,
    delta: bool,
    body: bool,
    annotation: bool,
    cover: bool,
}


#[allow(clippy::cognitive_complexity)]
pub fn run_index(matches: &ArgMatches, app: &mut Application) {
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
    let delta = match matches.value_of("INDEX-MODE") {
        Some("full") => false,
        _ => true,
    };
    for i in &["body", "annotation", "cover"] {
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
    let heap_mb_str = matches.value_of("memory").unwrap_or(DEFAULT_HEAP_SIZE_MB);
    let heap_size: usize = heap_mb_str.parse().unwrap_or_else(|_| {
        eprintln!(
            "{} {}",
            tr!["Invalid memory size", "Некорректный размер"],
            heap_mb_str
        );
        std::process::exit(4);
    });
    app.load_genre_map();
    //open index
    let book_writer = crate::fts::BookWriter::new(
        &app.index_path,
        &app.index_settings.stemmer,
        index_threads,
        heap_size * 1024 * 1024,
    )
    .unwrap();
    let book_writer_lock = Arc::new(Mutex::new(book_writer));
    let tt = std::time::Instant::now();
    let mut lang_set = HashSet::<&str>::new();
    let mut any_lang = false;
    for i in &app.index_settings.langs {
        lang_set.insert(i);
        if i == "ANY" {
            any_lang = true
        }
    }
    //index books with `undefined` language too
    let lang_filter = |lang: &str| any_lang || lang_set.contains(&lang) || lang.is_empty();
    let opts = ParseOpts {
        book_formats: &app.book_formats,
        genre_map: &app.genre_map,
        debug: app.debug,
        delta,
        body: !app.index_settings.disabled.contains(&"body".to_string()),
        annotation: !app
            .index_settings
            .disabled
            .contains(&"annotation".to_string()),
        cover: !app.index_settings.disabled.contains(&"cover".to_string())
    };

    //exit nicely if user press Ctrl+C
    let canceled = Arc::new(AtomicBool::new(false));
    let c = canceled.clone();
    ctrlc::set_handler(move || {
        eprintln!("Cancel indexing...");
        c.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    println!(
        "----{}----\ndir={} delta={} lang={:?} stemmer={} body={} annotation={} cover={} files={:?}",
        tr!["START INDEXING","НАЧИНАЕМ ИНДЕКСАЦИЮ"],
        &app.books_path.display(),
        opts.delta,
        &lang_set,
        &app.index_settings.stemmer,
        opts.body, opts.annotation, opts.cover,
        app.book_formats.keys()
    );
    if app.debug {
        println!(
            "index threads={:?} read threads={:?} heap={}",
            index_threads, read_threads, heap_size
        );
    }
    //save settings with index
    if app.debug {
        println!("store settings in {}", app.index_path.display());
    }
    app.index_settings
        .save(&app.index_path)
        .unwrap_or_else(|e| {
            eprintln!("{}", e);
            std::process::exit(2);
        });

    let mut zip_files: Vec<DirEntry> = std::fs::read_dir(&app.books_path)
        .expect("directory not readable")
        .map(|x| x.expect("invalid file"))
        .filter(is_zip_file)
        .filter(|x| files.is_empty() || files.contains(&x.file_name().as_os_str()))
        .collect();
    zip_files.sort_by_key(|e| get_numeric_sort_key(e.file_name().to_str().unwrap_or_default()));
    let zip_count = zip_files.len();
    let zip_total_size = zip_files.iter().fold(0, |acc, entry| {
        acc + entry.metadata().map(|m| m.len()).unwrap_or(0)
    });
    let mut zip_progress_size = 0;
    let mut zip_processed_size = 0;
    let mut zip_index = 0;
    let mut book_count = 0;
    let mut book_indexed = 0;
    let mut book_ignored = 0;
    let mut book_skipped = 0;
    let mut book_readed_size = 0;
    let mut book_parsed_size = 0;
    let mut error_count = 0;
    let mut warning_count = 0;
    let mut time_to_unzip = Duration::default();
    let mut time_to_parse = Duration::default();
    let mut time_to_image = Duration::default();
    let mut time_to_commit = Duration::default();

    if !delta {
        println!("{}", tr!["deleting index...", "очищаем индекс..."]);
        let book_writer_lock = book_writer_lock.clone();
        let mut book_writer = book_writer_lock.lock().unwrap();
        book_writer.delete_all_books().unwrap();
    }

    for entry in zip_files {
        if canceled.load(Ordering::SeqCst) {
            break;
        }
        let os_filename = &entry.file_name();
        let zip_size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let zip_progress_pct = zip_progress_size * 100 / zip_total_size;
        let zipfile = &os_filename.to_str().expect("invalid filename");
        if delta && files.is_empty() {
            let book_writer_lock = book_writer_lock.clone();
            let book_writer = book_writer_lock.lock().unwrap();
            if let Ok(true) = book_writer.is_book_indexed(&zipfile, "WHOLE") {
                println!(
                    "[{}/{}] {} {}",
                    zip_index,
                    zip_count,
                    tr!["skip archive", "пропускаем архив"],
                    &zipfile
                );
                zip_progress_size += zip_size;
                continue;
            }
        }
        println!(
            "[{}/{}={}%] {} {}",
            zip_index,
            zip_count,
            zip_progress_pct,
            tr!["read archive", "читаем архив"],
            &zipfile
        );
        let zt = Instant::now();
        let files_count = {
            let reader = std::fs::File::open(&entry.path()).unwrap();
            //let reader = std::io::BufReader::new(reader);
            let zip = zip::ZipArchive::new(reader).unwrap();
            zip.len()
        };
        time_to_unzip += zt.elapsed();

        //assume large zip-file with many books inside, 1-3 MB each
        //single-threaded zip decompressing chokes at ~60 MBps
        //reopen zip and read different parts in multiple threads
        let chunk_size = files_count / read_threads;
        let (stats_sender, stats_receiver) = channel();
        let mut first_file_num = 0;
        let mut last_file_num = chunk_size;
        let mut opts = opts.clone();
        //enforce reindex of books inside specified archive
        if !files.is_empty() {
            opts.delta = false;
        }
        thread::scope(|scope| {
            for reader_id in 0..read_threads {
                if (reader_id + 1) == read_threads {
                    last_file_num = files_count
                }
                if opts.debug {
                    println!(
                        "reader#{} batch: {}..{}",
                        reader_id, first_file_num, last_file_num
                    );
                }
                let canceled = canceled.clone();
                let stats_sender_clone = stats_sender.clone();
                let reader = std::fs::File::open(&entry.path()).unwrap();
                let reader = std::io::BufReader::with_capacity(READ_BUFFER_SIZE, reader);
                let mut zip = zip::ZipArchive::new(reader).unwrap();
                let book_writer_lock = book_writer_lock.clone();
                let opts = opts.clone();
                scope.spawn(move |_| {
                    let tid = format!("{:?}", std::thread::current().id());
                    for i in first_file_num..last_file_num {
                        if canceled.load(Ordering::SeqCst) {
                            break;
                        }
                        let mut file = zip.by_index(i).unwrap();
                        let filename: String = match decode_filename(file.name_raw()) {
                            Some(s) => s,
                            None => file.name().into(),
                        };
                        if opts.debug {
                            println!(
                                "[{}%] {} {}/{}",
                                zip_progress_pct, &tid, &zipfile, &filename
                            );
                        }
                        let zt = Instant::now();
                        let mut data = vec![];
                        let file = file.read_to_end(&mut data);
                        let ze = zt.elapsed();
                        let book_writer_lock = book_writer_lock.clone();
                        match file {
                            Ok(_) => {
                                let mut stats = process_file(
                                    &zipfile,
                                    &filename,
                                    data.as_ref(),
                                    book_writer_lock,
                                    lang_filter,
                                    &opts,
                                );
                                stats.time_to_unzip = ze;
                                stats_sender_clone.send(stats).unwrap();
                            }
                            Err(e) => {
                                eprintln!(
                                    "{} error reading {}/{}: {}",
                                    &tid, &zipfile, &filename, e
                                );
                                let mut stats = ParsedFileStats::default();
                                stats.error_count = 1;
                                stats.time_to_unzip = ze;
                                stats_sender_clone.send(stats).unwrap();
                            }
                        }
                    }
                });
                first_file_num += chunk_size;
                last_file_num += chunk_size;
            }
        })
        .unwrap(); //scope

        drop(stats_sender);
        //collect statistics
        let mut need_commit = false;
        for i in stats_receiver {
            if i.is_book {
                book_count += 1
            }
            if i.skipped {
                book_skipped += 1
            }
            if i.ignored {
                book_ignored += 1
            }
            if i.indexed {
                book_indexed += 1;
                need_commit = true;
            }
            error_count += i.error_count;
            warning_count += i.warning_count;
            book_readed_size += i.readed_size;
            book_parsed_size += i.parsed_size;
            time_to_unzip += i.time_to_unzip;
            time_to_parse += i.time_to_parse;
            time_to_image += i.time_to_image;
        }

        let book_writer_lock = book_writer_lock.clone();
        let mut book_writer = book_writer_lock.lock().unwrap();
        book_writer
            .add_file_record(&zipfile, "WHOLE", book_indexed)
            .unwrap_or(()); //mark whole archive as indexed
        if need_commit {
            if opts.debug {
                println!("Commit: start");
            }
            let ct = Instant::now();
            book_writer.commit().unwrap();
            time_to_commit += ct.elapsed();
            if opts.debug {
                println!("Commit: done");
            }
        }
        zip_index += 1;
        zip_progress_size += zip_size;
        zip_processed_size += zip_size;
    }
    if canceled.load(Ordering::SeqCst) {
        println!("{}", tr!["Indexing canceled", "Индексация прервана"]);
    } else {
        println!("{}", tr!["Indexing done", "Индексация завершена"]);
    }
    let total = tt.elapsed().as_millis() + 1;
    println!(
        "{}: {}/{} = {}/{} MB",
        tr!["Archives", "Архивов"],
        zip_index,
        zip_count,
        zip_processed_size / 1024 / 1024,
        zip_total_size / 1024 / 1024,
    );
    println!(
        "{}: {}/{} : {} {}, {} {} = {} {} / {} {}",
        tr!["Books", "Книг"],
        book_indexed,
        book_count,
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
        "{}: {} MB/s",
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
    if app.debug {
        println!(
            "Main thread: {} : commit {}%",
            format_duration(total),
            time_to_commit.as_millis() * 100 / total,
        );
        let ue = time_to_unzip.as_millis();
        let pe = time_to_parse.as_millis();
        let ie = time_to_image.as_millis();
        let total = ue + pe + ie + 1;
        println!(
            "Reader threads: {} : unpacking {}%, parse {}%, image resize {}%",
            format_duration(total / (read_threads as u128)),
            ue * 100 / total,
            pe * 100 / total,
            ie * 100 / total,
        );
    }
}


// extract number from string and left-pad it
lazy_static! {
    static ref RE_NUMBER: Regex = {
        Regex::new(r"[0-9]{2,9}").unwrap()
    };
}
fn get_numeric_sort_key(filename: &str) -> String {
    match RE_NUMBER.find(filename) {
        Some(n) => format!("{:0>9}", n.as_str()),
        None => filename.to_string()
    }
}

#[test] 
fn test_get_numeric_sort_key() {
    assert_eq!(get_numeric_sort_key("ab123cd45ef"),"000000123");
    let mut a = vec!["b","a", "c345","d12345","x001"];
    a.sort_by_key(|x| get_numeric_sort_key(x));
    assert_eq!(a, vec!["x001","c345","d12345","a","b"]);
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
    data: &[u8],
    book_writer_lock: Arc<Mutex<BookWriter>>,
    lang_filter: F,
    opts: &ParseOpts,
) -> ParsedFileStats
where
    F: Fn(&str) -> bool,
{
    let mut stats = ParsedFileStats::default();
    let ext = file_extension(&filename);
    if let Some(book_format) = opts.book_formats.get(&ext.as_ref()) {
        //filter eBook by extension
        stats.is_book = true;
        if opts.delta {
            let book_writer = book_writer_lock.lock().unwrap();
            if let Ok(true) = book_writer.is_book_indexed(&zipfile, &filename) {
                println!("  {} {}", &filename, tr!["indexed", "индексирован"]);
                stats.skipped = true;
                return stats;
            }
        }
        stats.readed_size = data.len(); //uncompressed book size with embedded images
        let mut buf_file = std::io::Cursor::new(data);
        let pt = Instant::now();
        let parsed_book = book_format.parse(
            &zipfile,
            &filename,
            &mut buf_file,
            opts.body,
            opts.annotation,
            opts.cover,
        );
        stats.time_to_parse = pt.elapsed();
        match parsed_book {
            Ok(mut b) => {
                stats.warning_count += b.warning.len();
                if opts.debug {
                    println!("    -> {}", &b)
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
                                stats.warning_count += 1;
                                b.cover_image = None;
                            }
                        }
                        stats.time_to_image = it.elapsed();
                    }

                    stats.parsed_size = b.size_of(); //metadata + plain text + cover image
                    if !opts.body {
                        b.length = stats.parsed_size as u64;
                    }

                    let mut book_writer = book_writer_lock.lock().unwrap();
                    match book_writer.add_book(b, opts.genre_map) {
                        Ok(_) => stats.indexed = true,
                        Err(e) => eprintln!(
                            "{}/{} -> {} {}",
                            zipfile,
                            filename,
                            tr!["indexing error", "ошибка индексации"],
                            e
                        ), //and continue
                    }
                } else {
                    stats.ignored = true;
                    println!(
                        "         -> {} {}",
                        tr!["ignore lang", "игнорируем язык"],
                        lang
                    );
                }
            }
            Err(e) => {
                stats.error_count += 1;
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
    stats
}

