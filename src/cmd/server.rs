use atom_syndication::{
    Category, ContentBuilder, Entry, EntryBuilder, FeedBuilder, LinkBuilder, Person,
};
use clap::ArgMatches;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use rouille::{Request, Response};
use std::collections::HashMap;
use std::io::prelude::*;
use std::path::Path;
use std::str;

use crate::cmd::*;
use crate::sort::LocalString;
use crate::tr;

const CACHE_IMMUTABLE: u64 = 31_536_000;
const CACHE_STATIC_ASSET: u64 = 86_400;
const OPDS_PAGE_ENTRIES: usize = 20;

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
        "{}: http://{}/porcula/home.html",
        tr!["Application", "Приложение"],
        &listen_addr
    );

    #[allow(clippy::cognitive_complexity)]
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
                let res = Response::from_data(asset.content_type, asset.content)
                    .with_public_cache(CACHE_STATIC_ASSET);
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
            (GET) (/opds) => { opds_root(&req, &fts) },
            (GET) (/opds/search/{query: String}) => { opds_search_where(&req, &query) },
            (GET) (/opds/search/{query: String}/) => { opds_search_where(&req, &query) },
            (GET) (/opds/search/{field: String}/{query: String}/{page: usize}) => {
                let query = format!("{}:{}", field, query);
                opds_search_books(&req, &query, "default", page, &app.genre_map.translation, &fts)
            },
            (GET) (/opds/author) => { opds_facet(&req, "author", None, "Авторы", None, &fts) },
            (GET) (/opds/author/{prefix: String}) => { opds_facet(&req, "author", Some(&prefix), "Авторы", None, &fts) },
            (GET) (/opds/author/{prefix: String}/{name: String}/{page: usize}) => {
                let query = format!("facet:/author/{}/{}", prefix, name);
                opds_search_books(&req, &query, "title", page, &app.genre_map.translation, &fts)
            },
            (GET) (/opds/genre) => { opds_facet(&req, "genre", None, "Жанры", Some(&app.genre_map.translation), &fts) },
            (GET) (/opds/genre/{prefix: String}) => { opds_facet(&req, "genre", Some(&prefix), "Жанры", Some(&app.genre_map.translation), &fts) },
            (GET) (/opds/genre/{cat: String}/{code: String}/{page: usize}) => {
                let query = format!("facet:/genre/{}/{}", cat, code);
                opds_search_books(&req, &query, "title", page, &app.genre_map.translation, &fts)
            },
            _ =>  Response::empty_404() ,
        )
    });
}

fn urlenc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn root_url(req: &Request) -> String {
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
    format!("{}://{}", proto, host)
}

// Request -> ("http://server:port", "/prefix/path")
fn split_request_url(req: &Request) -> (String, String) {
    (root_url(req), format!("/porcula{}", req.url()))
}

fn handler_count(_req: &Request, fts: &BookReader) -> Response {
    match &fts.count_all() {
        Ok(count) => Response::text(count.to_string()).with_no_cache(),
        Err(_) => Response::text("0".to_string()),
    }
}

fn handler_search(req: &Request, fts: &BookReader, debug: bool) -> Response {
    match req.get_param("query") {
        Some(query) => {
            let limit: usize = req
                .get_param("limit")
                .unwrap_or_default()
                .parse()
                .unwrap_or(DEFAULT_QUERY_HITS);
            let offset: usize = req
                .get_param("offset")
                .unwrap_or_default()
                .parse()
                .unwrap_or(0);
            let order: String = req
                .get_param("order")
                .unwrap_or_else(|| String::from("default"));
            match fts.search_as_json(&query, &order, limit, offset, debug) {
                Ok(json) => Response::from_data("application/json", json).with_no_cache(),
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
        _ => None,
    };
    match req.get_param("path") {
        Some(path) => match fts.get_facet(&path, opt_query, hits, debug) {
            Ok(ref data) => Response::json(data).with_no_cache(),
            Err(e) => Response::text(e.to_string()).with_status_code(500),
        },
        None => Response::empty_404(),
    }
}

fn handler_cover(_req: &Request, fts: &BookReader, zipfile: &str, filename: &str) -> Response {
    match fts.get_cover(zipfile, filename) {
        Ok(Some(img)) if !img.is_empty() => {
            Response::from_data("image/jpeg", img).with_public_cache(CACHE_IMMUTABLE)
        }
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
            const TEMPLATE: &str = "render.html";
            const TEMPLATE_SIZE: usize = 1000; //approximate
            let template = Path::new(DEFAULT_ASSETS_DIR).join(TEMPLATE);
            let mut html = Vec::<u8>::with_capacity(content.len() + TEMPLATE_SIZE);
            let mut buf: String = String::new();
            //read template from static file or load internal asset
            let tmpl: &str = if let Ok(mut f) = std::fs::File::open(template) {
                f.read_to_string(&mut buf).unwrap();
                &buf
            } else {
                let raw = assets::get(TEMPLATE)
                    .expect("render template not found")
                    .content;
                str::from_utf8(raw).unwrap()
            };
            let mut start = 0;
            let substr = "{title}";
            if let Some(found) = tmpl.find(substr) {
                html.extend_from_slice(tmpl[start..found].as_bytes());
                html.extend_from_slice(title.as_bytes());
                start = found + substr.len();
            }
            let substr = "{content}";
            if let Some(found) = tmpl.find(substr) {
                html.extend_from_slice(tmpl[start..found].as_bytes());
                html.extend_from_slice(&content);
                start = found + substr.len();
            }
            html.extend_from_slice(tmpl[start..].as_bytes());
            Response::from_data("text/html", html).with_public_cache(CACHE_IMMUTABLE)
        }
        None => Response::empty_404(),
    }
}

fn handler_file(_req: &Request, app: &Application, zipfile: &str, filename: &str) -> Response {
    match app.book_formats.get(&file_extension(filename).as_ref()) {
        Some(book_format) => {
            let content = read_zipped_file(&app.books_path, zipfile, filename);
            Response::from_data(book_format.content_type(), content)
                .with_public_cache(CACHE_IMMUTABLE)
        }
        None => Response::empty_404(),
    }
}

fn handler_opensearch_xml(req: &Request) -> Response {
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/">
      <ShortName>Porcula</ShortName>
      <Description>Library search</Description>
      <Url type="text/html" template="{}/porcula/home.html?query={{searchTerms}}"/>  
      <Language>ru-RU</Language>
      <OutputEncoding>UTF-8</OutputEncoding>
      <InputEncoding>UTF-8</InputEncoding>
    </OpenSearchDescription>"#,
        root_url(req)
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

fn atom_mime_type() -> Option<String> {
    Some("application/atom+xml".to_string())
}
fn atom_cat_mime_type() -> Option<String> {
    Some("application/atom+xml;profile=opds-catalog".to_string())
}
fn atom_nav_mime_type() -> Option<String> {
    Some("application/atom+xml;profile=opds-catalog;kind=navigation".to_string())
}

fn opds_response(
    title: &str,
    root: &str,
    path: &str,
    entries: Vec<Entry>,
    prev_url: Option<String>,
    next_url: Option<String>,
) -> Response {
    let abs_url = format!("{}/porcula{}", root, path);
    let mut ns = HashMap::<String, String>::new();
    ns.insert("dcterms".into(), "http://purl.org/dc/terms".into());

    let mut links = Vec::new();

    links.push(
        LinkBuilder::default()
            .href(&abs_url)
            .rel("self")
            .mime_type(atom_nav_mime_type())
            .build()
            .unwrap(),
    );
    links.push(
        LinkBuilder::default()
            .href("/porcula/opds")
            .rel("start")
            .mime_type(atom_nav_mime_type())
            .build()
            .unwrap(),
    );
    links.push(
        LinkBuilder::default()
            .href("/porcula/opds/search/{searchTerms}")
            .rel("search")
            .mime_type(atom_mime_type())
            .build()
            .unwrap(),
    );
    if let Some(url) = prev_url {
        links.push(
            LinkBuilder::default()
                .href(url)
                .rel("prev")
                .title(Some(
                    tr!["Previous Page", "Предыдущая страница"].to_string(),
                ))
                .mime_type(atom_cat_mime_type())
                .build()
                .unwrap(),
        );
    }
    if let Some(url) = next_url {
        links.push(
            LinkBuilder::default()
                .href(url)
                .rel("next")
                .title(Some(tr!["Next Page", "Следующая страница"].to_string()))
                .mime_type(atom_cat_mime_type())
                .build()
                .unwrap(),
        );
    }
    let f = FeedBuilder::default()
        .title(title)
        .subtitle(Some("Porcula library OPDS catalog".into()))
        .id(abs_url)
        .icon(Some("favicon.ico".into()))
        .namespaces(ns)
        .updated(chrono::Utc::now())
        .links(links)
        .entries(entries)
        .build()
        .unwrap();
    Response::from_data("application/xml", f.to_string())
}

fn opds_root(req: &Request, fts: &BookReader) -> Response {
    let (root_url, req_path) = split_request_url(req);
    let book_count = match fts.count_all() {
        Ok(c) => c,
        Err(_) => 0,
    };
    let mut e = Vec::new();

    let mut links = Vec::new();
    links.push(
        LinkBuilder::default()
            .href(format!("{}/porcula/opds/author", root_url))
            .rel("alternate")
            .build()
            .unwrap(),
    );
    links.push(
        LinkBuilder::default()
            .href("/porcula/opds/author")
            .rel("subsection")
            .mime_type(atom_nav_mime_type())
            .build()
            .unwrap(),
    );

    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("m:1")
            .title(tr!["By author", "По авторам"])
            .links(links)
            .content(
                ContentBuilder::default()
                    .value(format!("{}: {}", tr!["Books", "Книг"], book_count))
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap(),
    );

    let mut links = Vec::new();
    links.push(
        LinkBuilder::default()
            .href(format!("{}/porcula/opds/genre", root_url))
            .rel("alternate")
            .build()
            .unwrap(),
    );
    links.push(
        LinkBuilder::default()
            .href("/porcula/opds/genre")
            .rel("subsection")
            .mime_type(atom_nav_mime_type())
            .build()
            .unwrap(),
    );

    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("m:2")
            .title(tr!["By genre", "По жанрам"])
            .links(links)
            .content(
                ContentBuilder::default()
                    .value(format!("{}: {}", tr!["Books", "Книг"], book_count))
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap(),
    );

    opds_response("Porcula", &root_url, &req_path, e, None, None)
}

fn opds_search_where(req: &Request, query: &str) -> Response {
    let (root_url, req_path) = split_request_url(req);
    let mut e = Vec::new();

    let mut links = Vec::new();
    let rel_url = format!("/porcula/opds/search/title/{}/0", urlenc(query));
    let abs_url = format!("{}{}", &root_url, &rel_url);
    links.push(
        LinkBuilder::default()
            .href(abs_url)
            .rel("alternate")
            .build()
            .unwrap(),
    );
    links.push(
        LinkBuilder::default()
            .href(rel_url)
            .rel("subsection")
            .mime_type(atom_nav_mime_type())
            .build()
            .unwrap(),
    );
    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("st:1")
            .title(tr!["Search by title", "Поиск по наименованию"])
            .links(links)
            .build()
            .unwrap(),
    );

    let mut links = Vec::new();
    let rel_url = format!("/porcula/opds/search/author/{}/0", urlenc(query));
    let abs_url = format!("{}{}", &root_url, &rel_url);
    links.push(
        LinkBuilder::default()
            .href(abs_url)
            .rel("alternate")
            .build()
            .unwrap(),
    );
    links.push(
        LinkBuilder::default()
            .href(rel_url)
            .rel("subsection")
            .mime_type(atom_nav_mime_type())
            .build()
            .unwrap(),
    );
    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("st:2")
            .title(tr!["Search by author", "Поиск по автору"])
            .links(links)
            .build()
            .unwrap(),
    );

    let mut links = Vec::new();
    let rel_url = format!("/porcula/opds/search/body/{}/0", urlenc(query));
    let abs_url = format!("{}{}", &root_url, &rel_url);
    links.push(
        LinkBuilder::default()
            .href(abs_url)
            .rel("alternate")
            .build()
            .unwrap(),
    );
    links.push(
        LinkBuilder::default()
            .href(rel_url)
            .rel("subsection")
            .mime_type(atom_nav_mime_type())
            .build()
            .unwrap(),
    );
    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("st:3")
            .title(tr!["Search in book text", "Поиск по тексту книги"])
            .links(links)
            .build()
            .unwrap(),
    );

    opds_response(
        tr!["Porcula - search", "Porcula - поиск"],
        &root_url,
        &req_path,
        e,
        None,
        None,
    )
}

fn opds_facet(
    req: &Request,
    facet: &str,
    prefix: Option<&str>,
    title: &str,
    translation: Option<&HashMap<String, String>>,
    fts: &BookReader,
) -> Response {
    let (root_url, req_path) = split_request_url(req);
    let path = match prefix {
        Some(x) => format!("/{}/{}", facet, x),
        None => format!("/{}", facet),
    };
    match fts.get_facet(&path, None, None, false) {
        Ok(data) => {
            let mut arr: Vec<(String, u64, String)> = data
                .into_iter()
                .map(|(path, count)| {
                    let code = path.rsplitn(2, '/').next().unwrap_or("?");
                    let title = match translation {
                        Some(t) => match t.get(code) {
                            Some(tr) => tr.to_owned(),
                            None => code.to_owned(),
                        },
                        None => code.to_owned(),
                    };
                    (path, count, title)
                })
                .collect::<Vec<(String, u64, String)>>();
            arr.sort_by_cached_key(|(_p, _c, t)| LocalString { v: t.to_owned() });
            let mut e = Vec::new();
            let updated = chrono::Utc::now();
            for (path, count, title) in arr {
                let mut path = path
                    .split('/')
                    .map(|x| urlenc(x))
                    .collect::<Vec<String>>()
                    .join("/");
                //append page to final path, i.e. "/author/A/Abcd" -> "/author/A/Abcd/0"
                if prefix.is_some() {
                    path.push_str("/0");
                }
                let rel_url = format!("/porcula/opds{}", &path);
                let abs_url = format!("{}{}", &root_url, &rel_url);
                let mut links = Vec::new();
                links.push(
                    LinkBuilder::default()
                        .href(&abs_url)
                        .rel("alternate")
                        .build()
                        .unwrap(),
                );
                links.push(
                    LinkBuilder::default()
                        .href(&rel_url)
                        .rel("subsection")
                        .mime_type(atom_nav_mime_type())
                        .build()
                        .unwrap(),
                );
                e.push(
                    EntryBuilder::default()
                        .updated(updated)
                        .id(&abs_url)
                        .title(title)
                        .content(
                            ContentBuilder::default()
                                .value(format!("{}: {}", tr!["Books", "Книг"], count))
                                .build()
                                .unwrap(),
                        )
                        .links(links)
                        .build()
                        .unwrap(),
                );
            }
            opds_response(title, &root_url, &req_path, e, None, None)
        }
        Err(e) => Response::text(e.to_string()).with_status_code(500),
    }
}

fn opds_search_books(
    req: &Request,
    query: &str,
    order: &str,
    page: usize,
    translation: &HashMap<String, String>,
    fts: &BookReader,
) -> Response {
    let (root_url, req_path) = split_request_url(req);
    let limit = OPDS_PAGE_ENTRIES;
    let offset = page * OPDS_PAGE_ENTRIES;
    //split path to base and page
    let mut path_parts = req_path
        .split('/')
        .map(|x| urlenc(x))
        .collect::<Vec<String>>();
    let prev_url = if page == 0 || path_parts.len() < 2 {
        None
    } else {
        let n = path_parts.len() - 1;
        path_parts[n] = format!("{}", page - 1);
        Some(path_parts.join("/"))
    };
    //println!("DEBUG page={} limit={} offset={} root={} path={} -> prev={:?}", page, limit, offset, root_url, req_path, prev_url);
    match fts.search_as_meta(query, order, limit, offset, false) {
        Ok(data) => {
            let next_url = if data.len() < limit {
                None
            } else {
                let n = path_parts.len() - 1;
                path_parts[n] = format!("{}", page + 1);
                Some(path_parts.join("/"))
            };
            let mut e = Vec::new();
            for i in data {
                let rel_url = format!(
                    "/porcula/book/{}/{}",
                    urlenc(&i.zipfile),
                    urlenc(&i.filename)
                );
                let cover_url = format!(
                    "/porcula/book/{}/{}/cover",
                    urlenc(&i.zipfile),
                    urlenc(&i.filename)
                );
                let abs_url = format!("{}{}", &root_url, &rel_url);
                let mut links = Vec::new();
                links.push(
                    LinkBuilder::default()
                        .href(&abs_url)
                        .rel("alternate")
                        .build()
                        .unwrap(),
                );
                links.push(
                    LinkBuilder::default()
                        .href(&rel_url)
                        .rel("http://opds-spec.org/acquisition/open-access")
                        .mime_type(Some("application/fb2+xml".into()))
                        .build()
                        .unwrap(),
                );
                links.push(
                    LinkBuilder::default()
                        .href(&cover_url)
                        .rel("http://opds-spec.org/image")
                        .mime_type(Some("image/jpeg".into()))
                        .build()
                        .unwrap(),
                );
                let mut b = EntryBuilder::default()
                    .id(format!("b:{}/{}", i.zipfile, i.filename))
                    .title(i.title)
                    .links(links)
                    .build()
                    .unwrap();
                if let Some(x) = i.annotation {
                    b.set_content(Some(ContentBuilder::default().value(x).build().unwrap()));
                }
                b.set_authors(
                    i.author
                        .iter()
                        .map(|a| Person {
                            name: a.to_owned(),
                            email: None,
                            uri: None,
                        })
                        .collect::<Vec<Person>>(),
                );
                b.set_categories(
                    i.genre
                        .iter()
                        .map(|c| Category {
                            term: c.to_owned(),
                            scheme: None,
                            label: translation.get(c).map(|x| x.to_owned()),
                        })
                        .collect::<Vec<Category>>(),
                );
                e.push(b);
            }
            opds_response(
                tr!["Porcula - books", "Porcula - книги"],
                &root_url,
                &req_path,
                e,
                prev_url,
                next_url,
            )
        }
        Err(e) => Response::text(e.to_string()).with_status_code(500),
    }
}
