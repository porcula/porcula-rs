/// resources compiled in executable

use std::collections::HashMap;

pub struct Asset {
    pub content_type: &'static str,
    pub content: &'static [u8],
}

// filename -> (content-type,content)
lazy_static!{
    static ref ASSETS: HashMap::<&'static str, Asset> = {
        let mut m = HashMap::new();
        m.insert("authors.html", Asset{ content_type: "text/html; charset=utf8", content: include_bytes!("../static/authors.html") });
        m.insert("genres.html", Asset{ content_type: "text/html; charset=utf8", content: include_bytes!("../static/genres.html") });
        m.insert("home.html", Asset{ content_type: "text/html; charset=utf8", content: include_bytes!("../static/home.html") });
        m.insert("render.html", Asset{ content_type: "text/html; charset=utf8", content: include_bytes!("../static/render.html") });
        m.insert("favicon.ico", Asset{ content_type: "image/x-icon", content: include_bytes!("../static/favicon.ico") });
        m.insert("render.css", Asset{ content_type: "text/css; charset=utf8", content: include_bytes!("../static/render.css") });
        m.insert("site.css", Asset{ content_type: "text/css; charset=utf8", content: include_bytes!("../static/site.css") });
        m.insert("jquery-1.12.4.min.js", Asset{ content_type: "application/javascript", content: include_bytes!("../static/jquery-1.12.4.min.js") });
        m.insert("common.js", Asset{ content_type: "application/javascript", content: include_bytes!("../static/common.js") });
        m.insert("render.js", Asset{ content_type: "application/javascript", content: include_bytes!("../static/render.js") });
        m.insert("defcover.png", Asset{ content_type: "image/png", content: include_bytes!("../static/defcover.png") });
        m.insert("genre-map.txt", Asset{ content_type: "text/plain; charset=utf8", content: include_bytes!("../static/genre-map.txt") });
        m
    };
}

pub fn get(filename: &str) -> Option<&Asset> {
    ASSETS.get(filename)
}
