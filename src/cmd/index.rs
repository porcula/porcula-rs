use clap::ArgMatches;
use std::collections::HashSet;
use std::fs::DirEntry;
use std::io::BufReader;
use std::time::Instant;

use crate::cmd::*;
use crate::tr;

pub fn run_index(matches: &ArgMatches, app: &mut Application) {
    if matches.occurrences_of("language") > 0 {
        if let Some(v) = matches.values_of_lossy("language") {
            app.index_settings.langs = v;
        }
    }
    assert!(
        app.index_settings.langs.len() > 0,
        "{} {}",
        tr![
            "No language specified nor on command line [--lang], nor in settings file",
            "Не указан язык ни в командной строке [--lang], ни в файле настроек"
        ],
        INDEX_SETTINGS_FILE
    );
    if matches.occurrences_of("stemmer") > 0 {
        if let Some(v) = matches.value_of("stemmer") {
            app.index_settings.stemmer = v.to_string();
        }
    }
    let delta = match matches.value_of("INDEX-MODE") {
        Some("full") => false,
        _ => true,
    };
    for i in vec!["body", "annotation", "cover"] {
        let s = i.to_string();
        if matches.is_present(format!("with-{}", i)) {
            app.index_settings.disabled.remove(&s); //enabling field
        }
        if matches.is_present(format!("without-{}", i)) {
            app.index_settings.disabled.insert(s); //disabling field
        }
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
    app.load_genre_map();
    //open index
    let mut book_writer = crate::fts::BookWriter::new(
        &app.index_path,
        &app.index_settings.stemmer,
        num_threads,
        heap_size * 1024 * 1024,
    )
    .unwrap();
    let tt = std::time::Instant::now();
    let mut lang_set = HashSet::<String>::new();
    let mut any_lang = false;
    for i in &app.index_settings.langs {
        lang_set.insert(i.clone());
        if i == "ANY" {
            any_lang = true
        }
    }
    let with_body = !app.index_settings.disabled.contains(&"body".to_string());
    let with_annotation = !app
        .index_settings
        .disabled
        .contains(&"annotation".to_string());
    let with_cover = !app.index_settings.disabled.contains(&"cover".to_string());
    //exit nicely if user press Ctrl+C
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    let canceled: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
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
        delta,
        &lang_set,
        &app.index_settings.stemmer,
        with_body, with_annotation, with_cover,
        app.book_formats.keys()
    );
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
        .collect();
    zip_files.sort_by_key(|e| e.file_name());
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
    let mut book_parsed_size = 0;
    let mut error_count = 0;
    let mut warning_count = 0;
    let mut time_to_open_zip = 0;
    let mut time_to_parse = 0;
    let mut time_to_image = 0;
    let mut time_to_doc = 0;
    let mut time_to_commit = 0;

    if !delta {
        println!("{}", tr!["deleting index...", "очищаем индекс..."]);
        book_writer.delete_all_books().unwrap();
    }

    for entry in zip_files {
        if canceled.load(Ordering::SeqCst) {
            break;
        }
        let zt = Instant::now();
        let os_filename = &entry.file_name();
        let zip_size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let zip_progress_pct = zip_progress_size * 100 / zip_total_size;
        let zipfile = &os_filename.to_str().expect("invalid filename");
        if delta {
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
            "[{}/{}] {} {}",
            zip_index,
            zip_count,
            tr!["read archive", "читаем архив"],
            &zipfile
        );
        let reader = std::fs::File::open(&entry.path()).unwrap();
        let buffered = std::io::BufReader::new(reader);
        let mut zip = zip::ZipArchive::new(buffered).unwrap();
        let files_count = zip.len();
        time_to_open_zip += zt.elapsed().as_millis();
        let mut book_indexed_size = 0;
        for file_index in 0..files_count {
            if canceled.load(Ordering::SeqCst) {
                break;
            }
            let file = zip.by_index(file_index).unwrap();
            let filename: String = match decode_filename(file.name_raw()) {
                Some(s) => s,
                None => file.name().into(),
            };
            let ext = file_extension(&filename);
            if let Some(book_format) = app.book_formats.get(&ext.as_ref()) {
                //filter eBook by extension
                book_count += 1;
                if delta {
                    if let Ok(true) = book_writer.is_book_indexed(&zipfile, &filename) {
                        println!("  {} {}", &filename, tr!["indexed", "индексирован"]);
                        book_skipped += 1;
                        continue;
                    }
                }
                println!(
                    "[{}%/{}%] {}/{}",
                    file_index * 100 / files_count,
                    zip_progress_pct,
                    &zipfile,
                    &filename
                );
                book_parsed_size += file.size(); //uncompressed book size
                let mut buf_file = BufReader::new(file);
                let pt = Instant::now();
                let parsed_book = book_format.parse(
                    &zipfile,
                    &filename,
                    &mut buf_file,
                    with_body,
                    with_annotation,
                    with_cover,
                );
                time_to_parse += pt.elapsed().as_millis();
                match parsed_book {
                    Ok(mut b) => {
                        warning_count += b.warning.len();
                        if app.debug {
                            println!("    -> {}", &b)
                        }
                        if any_lang || (b.lang.len() > 0 && lang_set.get(&b.lang[0]).is_some()) {
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
                                        warning_count += 1;
                                        b.cover_image = None;
                                    }
                                }
                                time_to_image += it.elapsed().as_millis();
                            }

                            let book_size = b.size_of();
                            if !with_body {
                                b.length = book_size as u64;
                            }

                            let at = Instant::now();
                            match book_writer.add_book(b, &app.genre_map) {
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
                            book_indexed_size += book_size;
                            if book_indexed_size > batch_size {
                                if app.debug {
                                    println!("Commit: start");
                                }
                                let ct = Instant::now();
                                book_writer.commit().unwrap();
                                time_to_commit += ct.elapsed().as_millis();
                                if app.debug {
                                    println!("Commit: done");
                                }
                                book_indexed_size = 0;
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
        if app.debug {
            println!("Commit: start");
        }
        let ct = Instant::now();
        book_writer.commit().unwrap();
        time_to_commit += ct.elapsed().as_millis();
        if app.debug {
            println!("Commit: done");
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
    println!(
        "{}: {}/{} = {}/{} MB",
        tr!["Archives", "Архивов"],
        zip_index,
        zip_count,
        zip_processed_size / 1024 / 1024,
        zip_total_size / 1024 / 1024,
    );
    println!(
        "{}: {}/{} = {} MB, {} {}, {} {}",
        tr!["Books", "Книг"],
        book_indexed,
        book_count,
        book_parsed_size / 1024 / 1024,
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
    if app.debug {
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

fn is_zip_file(entry: &DirEntry) -> bool {
    entry.metadata().map(|e| e.is_file()).unwrap_or(false)
        && file_extension(entry.file_name().to_str().unwrap_or("")) == ".zip"
}
