use atom_syndication::{
    Category, ContentBuilder, Entry, EntryBuilder, FeedBuilder, LinkBuilder, Person,
};
use log::{debug, info};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use rouille::{Request, Response};
use std::collections::{BTreeMap, HashMap};
use std::io::prelude::*;
use std::path::Path;
use std::str;
use std::str::FromStr;

use crate::cmd::*;
use crate::sort::LocalString;
use crate::tr;

const CACHE_IMMUTABLE: u64 = 31_536_000;
const CACHE_STATIC_ASSET: u64 = 86_400;
const OPDS_PAGE_ENTRIES: usize = 20;

pub fn run_server(args: &ServerArgs, app: Application) -> ProcessResult {
    let fts = match app.open_book_reader() {
        Ok(x) => x,
        Err(e) => return ProcessResult::IndexError(e),
    };
    info!(
        "{}: {}",
        tr!["Index dir", "Индекс"],
        &app.index_path.display()
    );
    info!(
        "{}: {}",
        tr!["Books dir", "Книги "],
        &app.books_path.display()
    );
    info!(
        "{}: {:?}",
        tr!["Language", "Язык"],
        &app.index_settings.langs
    );
    info!(
        "{}: {:?}",
        tr!["Stemmer", "Стеммер"],
        &app.index_settings.stemmer
    );
    info!("{:?}", &app.index_settings.options);
    info!(
        "{}: http://{}/porcula/home.html",
        tr!["Application", "Приложение"],
        &args.listen
    );
    let genre_map = match app.load_genre_map() {
        Ok(x) => x,
        Err(e) => return ProcessResult::ConfigError(e),
    };

    #[allow(clippy::cognitive_complexity, clippy::manual_strip)]
    rouille::start_server(&args.listen, move |req| {
        debug!("req {}", req.raw_url());
        let mut req = req;
        let req_no_prefix;

        // map: /home.html -> home.html -> ./static/home.html
        // map: /porcula/home.html -> home.html -> ./static/home.html
        if let Some(r) = req.remove_prefix(DEFAULT_BASE_URL) {
            req_no_prefix = r;
            req = &req_no_prefix;
        }
        let res = rouille::match_assets(req, DEFAULT_ASSETS_DIR);
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
            (GET) (/index/info) => { handler_index_info(req, &app, &fts) },
            (GET) (/search) => { handler_search(req, &fts) },
            (GET) (/facet) => { handler_facet(req, &fts) },
            (GET) (/genre/translation) => { Response::json(&genre_map.translation) },
            (GET) (/book/{zipfile: String}/{filename: String}/render) => { handler_render(req, &fts, &app, &zipfile, &filename) },
            (GET) (/book/{zipfile: String}/{filename: String}) => { handler_file(req, &app, &zipfile, &filename) },
            (GET) (/book/{zipfile: String}/{filename: String}/{_saveas: String}) => { handler_file(req, &app, &zipfile, &filename) },
            (GET) (/opensearch) => { handler_opensearch_xml(req) },
            (GET) (/file_list) => { handler_file_list(req, &fts) },
            (GET) (/opds) => { opds_root(req, &fts) },
            (GET) (/opds/search/{query: String}) => { opds_search_where(req, &query) },
            (GET) (/opds/search/{query: String}/) => { opds_search_where(req, &query) },
            (GET) (/opds/search/{field: String}/{query: String}/{page: usize}) => {
                let query = format!("{}:{}", field, query);
                let order = match field.as_str() {
                    "sequence" => "sequence",
                    _ => "default"
                };
                opds_search_books(req, &query, order, page, &genre_map.translation, &fts)
            },
            (GET) (/opds/author) => { opds_facet(req, "author", None, "Авторы", None, &fts) },
            (GET) (/opds/author/{prefix: String}) => { opds_facet(req, "author", Some(&prefix), "Авторы", None, &fts) },
            (GET) (/opds/author/{prefix: String}/{name: String}/{page: usize}) => {
                let query = format!("facet:/author/{}/{}", prefix, name);
                opds_search_books(req, &query, "title", page, &genre_map.translation, &fts)
            },
            (GET) (/opds/genre) => { opds_facet(req, "genre", None, "Жанры", Some(&genre_map.translation), &fts) },
            (GET) (/opds/genre/{prefix: String}) => { opds_facet(req, "genre", Some(&prefix), "Жанры", Some(&genre_map.translation), &fts) },
            (GET) (/opds/genre/{cat: String}/{code: String}/{page: usize}) => {
                let query = format!("facet:/genre/{}/{}", cat, code);
                opds_search_books(req, &query, "title", page, &genre_map.translation, &fts)
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

fn handler_index_info(_req: &Request, app: &Application, fts: &BookReader) -> Response {
    match &fts.count_all() {
        Ok(count) => {
            let info = IndexInfo {
                count: *count,
                settings: app.index_settings.clone(),
            };
            Response::json(&info).with_no_cache()
        }
        Err(_) => Response::empty_404(),
    }
}

fn handler_search(req: &Request, fts: &BookReader) -> Response {
    match req.get_param("query") {
        Some(query) => {
            let stemming = req.get_param("stemming").unwrap_or_default() == "1";
            let disjunction = req.get_param("disjunction").unwrap_or_default() == "1";
            let limit: usize = req
                .get_param("page_size")
                .unwrap_or_default()
                .parse()
                .unwrap_or(DEFAULT_QUERY_HITS);
            let page: usize = req
                .get_param("page")
                .unwrap_or_default()
                .parse()
                .unwrap_or(0);
            let offset: usize = page * limit;
            let orderby = match req.get_param("order") {
                Some(s) => crate::fts::OrderBy::from_str(&s).unwrap_or_default(),
                None => crate::fts::OrderBy::default(),
            };
            match fts.search_as_json(&query, stemming, disjunction, orderby, limit, offset) {
                Ok(json) => Response::from_data("application/json", json).with_no_cache(),
                Err(e) => Response::text(e.to_string()).with_status_code(500),
            }
        }
        None => Response::empty_404(),
    }
}

fn handler_facet(req: &Request, fts: &BookReader) -> Response {
    let hits: Option<usize> = req
        .get_param("hits")
        .map(|x| x.parse().unwrap_or(DEFAULT_QUERY_HITS));
    let req_query = req.get_param("query");
    let opt_query = match req_query {
        Some(ref s) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    };
    let stemming = req.get_param("stemming").unwrap_or_default() == "1";
    let disjunction = req.get_param("disjunction").unwrap_or_default() == "1";
    match req.get_param("path") {
        Some(path) => match fts.get_facet(&path, opt_query, stemming, disjunction, hits) {
            Ok(ref data) => Response::json(data).with_no_cache(),
            Err(e) => Response::text(e.to_string()).with_status_code(500),
        },
        None => Response::empty_404(),
    }
}

fn handler_file_list(_req: &Request, fts: &BookReader) -> Response {
    if let Ok(hash) = fts.get_indexed_books(crate::fts::IndexListDetails::Full) {
        Response::json(&hash)
    } else {
        Response::empty_404()
    }
}

fn handler_render(
    _req: &Request,
    _fts: &BookReader,
    app: &Application,
    zipfile: &str,
    filename: &str,
) -> Response {
    let ext = file_extension(filename);
    match app.book_formats.get(&ext.as_ref()) {
        Some(book_format) => {
            let raw = read_zipped_file(&app.books_path, zipfile, filename);
            let (title, content) = book_format.render_to_html(&raw).unwrap(); //result is Vec<u8> but valid UTF-8
            const TEMPLATE: &str = "render.html";
            const TEMPLATE_SIZE: usize = 1000; //approximate
            let template = Path::new(DEFAULT_ASSETS_DIR).join(TEMPLATE);
            let mut html = String::with_capacity(content.len() + title.len() + TEMPLATE_SIZE);
            let mut buf = String::new();
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
                html.push_str(&tmpl[start..found]);
                html.push_str(&title);
                start = found + substr.len();
            }
            let substr = "{content}";
            if let Some(found) = tmpl.find(substr) {
                html.push_str(&tmpl[start..found]);
                html.push_str(&content);
                start = found + substr.len();
            }
            html.push_str(&tmpl[start..]);
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
    let mut ns = BTreeMap::<String, String>::new();
    ns.insert("dcterms".into(), "http://purl.org/dc/terms".into());

    let mut links = vec![
        LinkBuilder::default()
            .href(&abs_url)
            .rel("self".to_string())
            .mime_type(atom_nav_mime_type())
            .build(),
        LinkBuilder::default()
            .href("/porcula/opds".to_string())
            .rel("start".to_string())
            .mime_type(atom_nav_mime_type())
            .build(),
        LinkBuilder::default()
            .href("/porcula/opds/search/{searchTerms}".to_string())
            .rel("search".to_string())
            .mime_type(atom_mime_type())
            .build(),
    ];
    if let Some(url) = prev_url {
        links.push(
            LinkBuilder::default()
                .href(url)
                .rel("prev".to_string())
                .title(Some(
                    tr!["Previous Page", "Предыдущая страница"].to_string(),
                ))
                .mime_type(atom_cat_mime_type())
                .build(),
        );
    }
    if let Some(url) = next_url {
        links.push(
            LinkBuilder::default()
                .href(url)
                .rel("next".to_string())
                .title(Some(tr!["Next Page", "Следующая страница"].to_string()))
                .mime_type(atom_cat_mime_type())
                .build(),
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
        .build();
    Response::from_data("application/xml", f.to_string())
}

fn opds_root(req: &Request, fts: &BookReader) -> Response {
    let (root_url, req_path) = split_request_url(req);
    let book_count = fts.count_all().unwrap_or(0);
    let mut e = Vec::new();

    let links = vec![
        LinkBuilder::default()
            .href(format!("{}/porcula/opds/author", root_url))
            .rel("alternate".to_string())
            .build(),
        LinkBuilder::default()
            .href("/porcula/opds/author".to_string())
            .rel("subsection".to_string())
            .mime_type(atom_nav_mime_type())
            .build(),
    ];
    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("m:1".to_string())
            .title(tr!["By author", "По авторам"])
            .links(links)
            .content(Some(
                ContentBuilder::default()
                    .value(Some(format!("{}: {}", tr!["Books", "Книг"], book_count)))
                    .build(),
            ))
            .build(),
    );

    let links = vec![
        LinkBuilder::default()
            .href(format!("{}/porcula/opds/genre", root_url))
            .rel("alternate".to_string())
            .build(),
        LinkBuilder::default()
            .href("/porcula/opds/genre".to_string())
            .rel("subsection".to_string())
            .mime_type(atom_nav_mime_type())
            .build(),
    ];
    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("m:2".to_string())
            .title(tr!["By genre", "По жанрам"])
            .links(links)
            .content(Some(
                ContentBuilder::default()
                    .value(Some(format!("{}: {}", tr!["Books", "Книг"], book_count)))
                    .build(),
            ))
            .build(),
    );

    opds_response("Porcula", &root_url, &req_path, e, None, None)
}

fn opds_search_where(req: &Request, query: &str) -> Response {
    let (root_url, req_path) = split_request_url(req);
    let mut e = Vec::new();

    let rel_url = format!("/porcula/opds/search/title/{}/0", urlenc(query));
    let abs_url = format!("{}{}", &root_url, &rel_url);
    let links = vec![
        LinkBuilder::default()
            .href(abs_url)
            .rel("alternate".to_string())
            .build(),
        LinkBuilder::default()
            .href(rel_url)
            .rel("subsection".to_string())
            .mime_type(atom_nav_mime_type())
            .build(),
    ];
    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("st:1".to_string())
            .title(tr!["Search by title", "Поиск по наименованию"])
            .links(links)
            .build(),
    );

    let rel_url = format!("/porcula/opds/search/author/{}/0", urlenc(query));
    let abs_url = format!("{}{}", &root_url, &rel_url);
    let links = vec![
        LinkBuilder::default()
            .href(abs_url)
            .rel("alternate".to_string())
            .build(),
        LinkBuilder::default()
            .href(rel_url)
            .rel("subsection".to_string())
            .mime_type(atom_nav_mime_type())
            .build(),
    ];
    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("st:2".to_string())
            .title(tr!["Search by author", "Поиск по автору"])
            .links(links)
            .build(),
    );

    let rel_url = format!("/porcula/opds/search/body/{}/0", urlenc(query));
    let abs_url = format!("{}{}", &root_url, &rel_url);
    let links = vec![
        LinkBuilder::default()
            .href(abs_url)
            .rel("alternate".to_string())
            .build(),
        LinkBuilder::default()
            .href(rel_url)
            .rel("subsection".to_string())
            .mime_type(atom_nav_mime_type())
            .build(),
    ];
    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("st:3".to_string())
            .title(tr!["Search in book text", "Поиск по тексту книги"])
            .links(links)
            .build(),
    );

    let rel_url = format!("/porcula/opds/search/sequence/{}/0", urlenc(query));
    let abs_url = format!("{}{}", &root_url, &rel_url);
    let links = vec![
        LinkBuilder::default()
            .href(abs_url)
            .rel("alternate".to_string())
            .build(),
        LinkBuilder::default()
            .href(rel_url)
            .rel("subsection".to_string())
            .mime_type(atom_nav_mime_type())
            .build(),
    ];
    e.push(
        EntryBuilder::default()
            .updated(chrono::Utc::now())
            .id("st:3".to_string())
            .title(tr!["Search in series", "Поиск по серии книг"])
            .links(links)
            .build(),
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
    match fts.get_facet(&path, None, false, false, None) {
        Ok(data) => {
            let mut arr: Vec<(String, u64, String)> = data
                .into_iter()
                .map(|(path, count)| {
                    let code = path.rsplit_once('/').map(|x| x.1).unwrap_or("?");
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
            arr.sort_by_cached_key(|(_p, _c, t)| LocalString(t.to_owned()));
            let mut e = Vec::new();
            let updated = chrono::Utc::now();
            for (path, count, title) in arr {
                let mut path = path
                    .split('/')
                    .map(urlenc)
                    .collect::<Vec<String>>()
                    .join("/");
                //append page to final path, i.e. "/author/A/Abcd" -> "/author/A/Abcd/0"
                if prefix.is_some() {
                    path.push_str("/0");
                }
                let rel_url = format!("/porcula/opds{}", &path);
                let abs_url = format!("{}{}", &root_url, &rel_url);
                let links = vec![
                    LinkBuilder::default()
                        .href(&abs_url)
                        .rel("alternate".to_string())
                        .build(),
                    LinkBuilder::default()
                        .href(&rel_url)
                        .rel("subsection".to_string())
                        .mime_type(atom_nav_mime_type())
                        .build(),
                ];
                e.push(
                    EntryBuilder::default()
                        .updated(updated)
                        .id(&abs_url)
                        .title(title)
                        .content(Some(
                            ContentBuilder::default()
                                .value(Some(format!("{}: {}", tr!["Books", "Книг"], count)))
                                .build(),
                        ))
                        .links(links)
                        .build(),
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
    orderby: &str,
    page: usize,
    translation: &HashMap<String, String>,
    fts: &BookReader,
) -> Response {
    let (root_url, req_path) = split_request_url(req);
    let orderby = crate::fts::OrderBy::from_str(orderby).unwrap_or_default();
    let stemming = true; //TODO: url parameter
    let disjunction = false; //TODO: url parameter
    let limit = OPDS_PAGE_ENTRIES;
    let offset = page * OPDS_PAGE_ENTRIES;
    //split path to base and page
    let mut path_parts = req_path.split('/').map(urlenc).collect::<Vec<String>>();
    let prev_url = if page == 0 || path_parts.len() < 2 {
        None
    } else {
        let n = path_parts.len() - 1;
        path_parts[n] = format!("{}", page - 1);
        Some(path_parts.join("/"))
    };
    match fts.search_as_meta(query, stemming, disjunction, orderby, limit, offset) {
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
                let links = vec![
                    LinkBuilder::default()
                        .href(&abs_url)
                        .rel("alternate".to_string())
                        .build(),
                    LinkBuilder::default()
                        .href(&rel_url)
                        .rel("http://opds-spec.org/acquisition/open-access".to_string())
                        .mime_type(Some("application/fb2+xml".to_string()))
                        .build(),
                    LinkBuilder::default()
                        .href(&cover_url)
                        .rel("http://opds-spec.org/image".to_string())
                        .mime_type(Some("image/jpeg".to_string()))
                        .build(),
                ];
                let mut b = EntryBuilder::default()
                    .id(format!("b:{}/{}", i.zipfile, i.filename))
                    .title(i.title)
                    .links(links)
                    .build();
                if let Some(x) = i.annotation {
                    b.set_content(Some(ContentBuilder::default().value(Some(x)).build()));
                }
                if let Some(sequence) = i.sequence {
                    let text = format!(
                        "{}: {} {}",
                        tr!["Sequence", "Серия"],
                        sequence,
                        i.seqnum.unwrap_or(0)
                    );
                    b.set_summary(Some(text.into()));
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
