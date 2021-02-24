use clap::ArgMatches;
use regex::Regex;
use std::collections::HashSet;
use std::fs::DirEntry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc};
use std::time::{Duration, Instant};

use crate::cmd::*;
use crate::fts::BookWriter;
use crate::tr;

//const READ_BUFFER_SIZE: usize = 2 * 1024 * 1024;

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
    indexed_size: usize,
    time_to_parse: Duration,
    time_to_image: Duration,
    time_to_index: Duration,
}

struct ParseOpts<'a> {
    book_formats: &'a BookFormats,
    genre_map: &'a GenreMap,
    debug: bool,
    body: bool,
    xbody: bool,
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
    let delta = ! matches!( matches.value_of("INDEX-MODE"), Some("full") );
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
    //open index
    let mut book_writer = crate::fts::BookWriter::new(
        &app.index_path,
        &app.index_settings.stemmer,
        index_threads,
        heap_size,
    )
    .unwrap();
    let tt = Instant::now();
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
        body: !app.index_settings.disabled.contains(&"body".to_string()),
        xbody: !app.index_settings.disabled.contains(&"xbody".to_string()),
        annotation: !app
            .index_settings
            .disabled
            .contains(&"annotation".to_string()),
        cover: !app.index_settings.disabled.contains(&"cover".to_string()),
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
        "----{}----\ndir={} delta={} lang={:?} stemmer={} body={} xbody={} annotation={} cover={} files={:?}",
        tr!["START INDEXING","НАЧИНАЕМ ИНДЕКСАЦИЮ"],
        &app.books_path.display(),
        delta,
        &lang_set,
        &app.index_settings.stemmer,
        opts.body, opts.xbody, opts.annotation, opts.cover,
        app.book_formats.keys()
    );
    if app.debug {
        println!(
            "index threads={:?} heap={} batch={}",
            index_threads, heap_size, batch_size,
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
    let mut need_commit = false;
    let mut time_to_unzip = Duration::default();
    let mut time_to_parse = Duration::default();
    let mut time_to_image = Duration::default();
    let mut time_to_index = Duration::default();
    let mut time_to_commit = Duration::default();

    if !delta {
        println!("{}", tr!["deleting index...", "очищаем индекс..."]);
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
        //enforce reindex of books inside specified archive
        let skip_indexed = if files.is_empty() { delta } else { false };
        let mut running_indexed_size: usize = 0;
        let zt = Instant::now();
        let reader = std::fs::File::open(&entry.path()).unwrap();
        //let reader = std::io::BufReader::with_capacity(READ_BUFFER_SIZE, reader);
        let mut zip = zip::ZipArchive::new(reader).unwrap();
        let files_count = zip.len();
        time_to_unzip += zt.elapsed();
        for i in 0..files_count {
            if canceled.load(Ordering::SeqCst) {
                break;
            }
            let zt = Instant::now();
            let mut file = zip.by_index(i).unwrap();
            let filename: String = match decode_filename(file.name_raw()) {
                Some(s) => s,
                None => file.name().into(),
            };
            if opts.debug {
                println!(
                    "[{}%] {}/{}",
                    zip_progress_pct, &zipfile, &filename
                );
            }
            let mut data = Vec::with_capacity(file.size() as usize);
            let file = file.read_to_end(&mut data);
            time_to_unzip += zt.elapsed();
            match file {
                Ok(_) => {
                    let stats = process_file(
                        &zipfile,
                        &filename,
                        data.as_ref(),
                        &mut book_writer,
                        lang_filter,
                        &opts,
                        skip_indexed,
                    );
                    if stats.is_book {
                        book_count += 1
                    }
                    if stats.skipped {
                        book_skipped += 1
                    }
                    if stats.ignored {
                        book_ignored += 1
                    }
                    if stats.indexed {
                        book_indexed += 1;
                        need_commit = true;
                    }
                    error_count += stats.error_count;
                    warning_count += stats.warning_count;
                    book_readed_size += stats.readed_size;
                    book_parsed_size += stats.parsed_size;
                    time_to_parse += stats.time_to_parse;
                    time_to_image += stats.time_to_image;
                    time_to_index += stats.time_to_index;
                    running_indexed_size += stats.indexed_size;
                    if running_indexed_size > batch_size {
                        running_indexed_size = 0;
                        if opts.debug {
                            println!("Batch commit: start");
                        }
                        let ct = Instant::now();
                        book_writer.commit().unwrap();
                        time_to_commit += ct.elapsed();
                        if opts.debug {
                            println!("Batch commit: done");
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "error reading {}/{}: {}",
                        &zipfile, &filename, e
                    );
                    error_count += 1;
                }
            }
        }
        //mark whole archive as indexed
        if !canceled.load(Ordering::SeqCst) {
            book_writer
                .add_file_record(&zipfile, "WHOLE", book_indexed)
                .unwrap(); 
        }       
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
    if app.debug {
        let ue = time_to_unzip.as_millis();
        let pe = time_to_parse.as_millis();
        let ie = time_to_image.as_millis();
        let xe = time_to_index.as_millis();
        let ce = time_to_commit.as_millis();
        println!(
            "unpacking {}%, parse {}%, image resize {}%,index {}%, commit {}%",
            ue * 100 / total,
            pe * 100 / total,
            ie * 100 / total,
            xe * 100 / total,
            ce * 100 / total,
        );
    }
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
    data: &[u8],
    book_writer: &mut BookWriter,
    lang_filter: F,
    opts: &ParseOpts,
    skip_indexed: bool,
) -> ParsedFileStats
where
    F: Fn(&str) -> bool,
{
    let mut stats = ParsedFileStats::default();
    let ext = file_extension(&filename);
    if let Some(book_format) = opts.book_formats.get(&ext.as_ref()) {
        //filter eBook by extension
        stats.is_book = true;
        if skip_indexed {
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
            opts.body || opts.xbody,
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
                    if !opts.body && !opts.xbody {
                        b.length = stats.parsed_size as u64;
                    }
                    stats.indexed_size = match b.body {
                        Some(ref x) => x.len(),
                        None => 0,
                    };
                    let it = Instant::now();
                    match book_writer.add_book(b, opts.genre_map, opts.body, opts.xbody) {
                        Ok(_) => stats.indexed = true,
                        Err(e) => {
                            stats.error_count += 1;
                            eprintln!(
                                "{}/{} -> {} {}",
                                zipfile,
                                filename,
                                tr!["indexing error", "ошибка индексации"],
                                e
                            )
                            //and continue
                        }
                    }
                    stats.time_to_index = it.elapsed();
                } else {
                    stats.ignored = true;
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
