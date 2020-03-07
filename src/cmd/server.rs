use clap::ArgMatches;
use rouille::{Request, Response};
use std::io::prelude::*;
use std::path::Path;

use crate::cmd::*;
use crate::tr;

pub fn run_server(matches: &ArgMatches, app: Application) -> Result<(), String> {
    let fts = app.open_book_reader().unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(4);
    });
    let listen_addr = matches.value_of("listen").unwrap_or(DEFAULT_LISTEN_ADDR);
    println!(
        "{}: {}",
        tr!["Index dir", "Индекс"],
        &app.index_path.display()
    );
    println!(
        "{}: {}",
        tr!["Books dir", "Книги "],
        &app.books_path.display()
    );
    println!(
        "{}: {:?}",
        tr!["Language", "Язык"],
        &app.index_settings.langs
    );
    println!(
        "{}: http://{}/home.html",
        tr!["Application", "Приложение"],
        &listen_addr
    );

    rouille::start_server(&listen_addr, move |req| {
        if app.debug {
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
            (GET) (/search) => { handler_search(&req, &fts, app.debug) },
            (GET) (/facet) => { handler_facet(&req, &fts, app.debug) },
            (GET) (/genre/translation) => { Response::json(&app.genre_map.translation) },
            (GET) (/book/{zipfile: String}/{filename: String}/cover) => { handler_cover(&req, &fts, &zipfile, &filename) },
            (GET) (/book/{zipfile: String}/{filename: String}/render) => { handler_render(&req, &fts, &app, &zipfile, &filename) },
            (GET) (/book/{zipfile: String}/{filename: String}) => { handler_file(&req, &app, &zipfile, &filename) },
            (GET) (/book/{zipfile: String}/{filename: String}/{_saveas: String}) => { handler_file(&req, &app, &zipfile, &filename) },
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
                .unwrap_or(DEFAULT_QUERY_HITS);
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

fn handler_facet(req: &Request, fts: &BookReader, debug: bool) -> Response {
    let hits: Option<usize> = match req.get_param("hits") {
        Some(x) => Some(x.parse().unwrap_or(DEFAULT_QUERY_HITS)),
        None => None,
    };
    let req_query = req.get_param("query");
    let opt_query = match req_query {
        Some(ref s) if s != "" => Some(s.as_str()),
        _ => None
    };
    match req.get_param("path") {
        Some(path) => match fts.get_facet(&path, opt_query, hits, debug) {
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
    app: &Application,
    zipfile: &str,
    filename: &str,
) -> Response {
    let ext = file_extension(&filename);
    match app.book_formats.get(&ext.as_ref()) {
        Some(book_format) => {
            let (title, enc) = match fts.get_book_info(zipfile, filename).unwrap() {
                Some(x) => x,
                None => (filename.to_string(), "UTF-8".to_string()), //book not indexed yet, try defaults
            };
            let content = read_zipped_file(&app.books_path, zipfile, filename);
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

fn handler_file(_req: &Request, app: &Application, zipfile: &str, filename: &str) -> Response {
    match app.book_formats.get(&file_extension(filename).as_ref()) {
        Some(book_format) => {
            let content = read_zipped_file(&app.books_path, zipfile, filename);
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

pub fn read_zipped_file(books_path: &Path, zipfile: &str, filename: &str) -> Vec<u8> {
    let zip_path = books_path.join(zipfile);
    let reader = std::fs::File::open(zip_path).unwrap();
    let buffered = std::io::BufReader::new(reader);
    let mut zip = zip::ZipArchive::new(buffered).unwrap();
    let mut file = zip.by_name(filename).unwrap();
    let mut content = vec![];
    file.read_to_end(&mut content).unwrap();
    content
}
