use crate::types::*;
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};
use quick_xml::events::attributes::{Attribute, Attributes};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::name::QName;
use quick_xml::Reader;
use std::borrow::{Borrow, Cow};
use std::collections::HashMap;
use std::str;

pub struct Fb2BookFormat;

enum ParentNode {
    Start,
    TitleInfo,
    SrcTitleInfo,
}

enum XMode {
    Start,
    Body(ParentNode),
    TitleInfo,
    SrcTitleInfo,
    DocInfo,
    Author(ParentNode),
    Translator,
    Annotation(ParentNode),
    Binary(Vec<u8>, Vec<u8>), // (id,content-type)
}

fn get_attr_raw<'a>(name: &[u8], attrs: &'a mut Attributes) -> Option<Attribute<'a>> {
    attrs.filter_map(|x| x.ok()).find(|a|
        //<tag x:attr="val"> - ignore namespace, equals <tag attr="val">
        a.key.local_name().as_ref().eq(name))
}

fn get_attr_string(
    name: &str,
    attrs: &mut Attributes,
    xml: &quick_xml::Reader<&[u8]>,
) -> Option<String> {
    match get_attr_raw(name.as_bytes(), attrs) {
        Some(a) => match a.decode_and_unescape_value(xml) {
            Ok(x) => Some(x.to_string()),
            _ => None,
        },
        None => None,
    }
}

fn format_xml_error<T: std::fmt::Debug>(error: T, reader: &Reader<&[u8]>) -> String {
    let end_pos = reader.buffer_position();
    let buf: &[u8] = reader.get_ref();
    let end_pos = if end_pos >= buf.len() { 0 } else { end_pos };
    let mut line = 1;
    let mut column = 0;
    for c in buf[0..end_pos].iter() {
        if *c == 0x10 {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    format!("Error at position {},{}: {:?}", line, column, error)
}

impl BookFormat for Fb2BookFormat {
    fn file_extension(&self) -> &'static str {
        ".fb2"
    }
    fn content_type(&self) -> &'static str {
        "application/fb2"
    }

    #[allow(clippy::cognitive_complexity, clippy::single_match)]
    fn parse(
        &self,
        raw: &[u8],
        with_body: bool,
        with_annotation: bool,
        with_cover: bool,
    ) -> Result<Book, ParserError> {
        let mut warning = Vec::<String>::new();
        let encoding = detect_xml_encoding(raw);
        let (xml_str, _enc, malformed) = encoding.decode(raw);
        if malformed {
            warning.push("malformed characters replaced".to_string());
        }
        let mut xml = quick_xml::Reader::from_reader(xml_str.as_ref().as_bytes());
        xml.trim_text(true);
        let mut mode = XMode::Start;
        let mut tag: Vec<u8> = vec![];
        let mut id: Option<String> = None;
        let mut cover_b64: Option<BytesText> = None;
        let mut coverpage_href: String = String::new();
        let mut cover_prob = 0;
        let mut cover_load = -1;
        let mut author = Vec::<Person>::new();
        let mut src_author = Vec::<Person>::new();
        let mut translator = Vec::<Person>::new();
        let mut person = Person::default();
        let mut genre = Vec::<String>::new();
        let mut keyword = Vec::<String>::new();
        let mut title = Vec::<String>::new();
        let mut sequence = Vec::<String>::new();
        let mut seqnum = Vec::<i64>::new();
        let mut lang = Vec::<String>::new();
        let mut date = Vec::<String>::new();
        let mut annotation = Vec::<String>::new();
        let mut body = Vec::<String>::new();
        loop {
            match xml.read_event() {
                //continue processing non-valid XML
                Err(e) => warning.push(format!(
                    "Error at position {}: {:?}",
                    &xml.buffer_position(),
                    e
                )),
                Ok(Event::Eof) => break,
                Ok(event) => match mode {
                    XMode::Start => match event {
                        Event::Start(ref e) => {
                            match e.local_name().as_ref() {
                                b"body" => {
                                    if with_body {
                                        mode = XMode::Body(ParentNode::Start);
                                    } else {
                                        match xml.read_to_end(QName(b"body")) {
                                            Ok(_span) => (),
                                            Err(e) => {
                                                warning.push(format_xml_error(e, &xml));
                                            }
                                        }
                                    }
                                }
                                b"title-info" => mode = XMode::TitleInfo,
                                b"src-title-info" => mode = XMode::SrcTitleInfo,
                                b"document-info" => mode = XMode::DocInfo,
                                b"binary" => {
                                    if with_cover {
                                        match get_attr_raw(b"id", &mut e.attributes()) {
                                            Some(a) => {
                                                let id = a.value.to_vec();
                                                if *coverpage_href.as_bytes() == *id {
                                                    cover_prob = 3; //exact match with coverpage/image/href
                                                }
                                                if cover_prob < 3 {
                                                    //search word 'cover' in id
                                                    if let Ok(s) = str::from_utf8(&*id) {
                                                        if s.to_lowercase().contains("cover") {
                                                            cover_prob = 2;
                                                        }
                                                    }
                                                }
                                                if cover_prob < 1
                                                //just first occurence of binary tag
                                                {
                                                    cover_prob = 1;
                                                }
                                                let ct = match get_attr_raw(
                                                    b"content-type",
                                                    &mut e.attributes(),
                                                ) {
                                                    Some(a) => a.value.to_vec(),
                                                    None => b"image/jpeg".to_vec(),
                                                };
                                                mode =
                                                    XMode::Binary(id, ct);
                                            }
                                            _ => (),
                                        }
                                    } else {
                                        match xml.read_to_end(QName(b"binary")) {
                                            Ok(_span) => (),
                                            Err(e) => {
                                                warning.push(format_xml_error(e, &xml));
                                            }
                                        }
                                    }
                                }
                                _ => tag = e.local_name().as_ref().into(),
                            }
                        }
                        _ => (),
                    },
                    XMode::Binary(ref _id, ref _ct) => match event {
                        Event::End(_) => mode = XMode::Start,
                        Event::Text(e) if cover_prob > cover_load => {
                            cover_b64 = Some(e.into_owned());
                            cover_load = cover_prob;
                        }
                        _ => (),
                    },
                    XMode::TitleInfo => match event {
                        Event::Start(e) => {
                            tag = e.local_name().as_ref().into();
                            match e.local_name().as_ref() {
                                b"author" => mode = XMode::Author(ParentNode::TitleInfo),
                                b"translator" => mode = XMode::Translator,
                                b"annotation" => {
                                    if with_annotation {
                                        mode = XMode::Annotation(ParentNode::TitleInfo);
                                    } else {
                                        match xml.read_to_end(QName(b"annotation")) {
                                            Ok(_span) => (),
                                            Err(e) => {
                                                warning.push(format_xml_error(e, &xml));
                                            }
                                        }
                                    }
                                }
                                b"date" => {
                                    tag = e.local_name().as_ref().into();
                                    if let Some(a) =
                                        get_attr_string("value", &mut e.attributes(), &xml)
                                    {
                                        date.push(a);
                                    }
                                }
                                _ => (),
                            }
                        }
                        Event::Empty(ref e) => match e.local_name().as_ref() {
                            b"sequence" => {
                                for i in e.attributes().filter_map(|x| x.ok()) {
                                    match i.key.as_ref() {
                                        b"name" => {
                                            if let Ok(name) = i.decode_and_unescape_value(&xml) {
                                                sequence.push(name.to_string());
                                            }
                                        }
                                        b"number" => {
                                            if let Ok(number) = i.decode_and_unescape_value(&xml) {
                                                seqnum.push(
                                                    number.parse::<i64>().unwrap_or_default(),
                                                );
                                            }
                                        }
                                        _ => (),
                                    }
                                }
                            }
                            b"image" => {
                                if let Some(v) = get_attr_string("href", &mut e.attributes(), &xml)
                                {
                                    // "#link" -> "link"
                                    coverpage_href = v.trim_start_matches('#').to_string();
                                }
                            }
                            _ => (),
                        },
                        Event::Text(e) => match tag.as_slice() {
                            b"book-title" => {
                                if let Ok(v) = e.unescape() {
                                    title.push(v.to_string());
                                }
                            }
                            b"lang" => {
                                if let Ok(v) = e.unescape() {
                                    let mut v = v.to_string();
                                    if v.len() > 2 {
                                        v = v[0..2].to_string()
                                    } //2-letter ISO 639-1
                                    v = v.to_lowercase();
                                    lang.push(v);
                                }
                            }
                            b"genre" => {
                                if let Ok(v) = e.unescape() {
                                    genre.push(v.to_string());
                                }
                            }
                            b"date" => {
                                if let Ok(v) = e.unescape() {
                                    date.push(v.to_string());
                                }
                            }
                            b"keywords" => {
                                if let Ok(v) = e.unescape() {
                                    for i in v.split(',') {
                                        keyword.push(i.trim().to_lowercase());
                                    }
                                }
                            }
                            _ => (),
                        },
                        Event::End(ref e) if e.local_name().as_ref() == b"title-info" => {
                            mode = XMode::Start
                        }
                        _ => (),
                    },
                    XMode::SrcTitleInfo => match event {
                        Event::Start(e) => {
                            tag = e.local_name().as_ref().into();
                            match e.local_name().as_ref() {
                                b"author" => mode = XMode::Author(ParentNode::SrcTitleInfo),
                                b"date" => {
                                    if let Some(a) =
                                        get_attr_string("value", &mut e.attributes(), &xml)
                                    {
                                        date.push(a);
                                    }
                                }
                                _ => (),
                            }
                        }
                        Event::Text(e) => match tag.as_slice() {
                            //single field for translation / source
                            b"book-title" => {
                                if let Ok(v) = e.unescape() {
                                    title.push(v.to_string());
                                }
                            }
                            b"lang" => {
                                if let Ok(v) = e.unescape() {
                                    lang.push(v.to_string());
                                }
                            }
                            b"date" => {
                                if let Ok(v) = e.unescape() {
                                    date.push(v.to_string());
                                }
                            }
                            _ => (),
                        },
                        Event::End(ref e) if e.local_name().as_ref() == b"src-title-info" => {
                            mode = XMode::Start
                        }
                        _ => (),
                    },
                    XMode::Author(ref parent_node) => match event {
                        Event::Start(e) => tag = e.local_name().as_ref().into(),
                        Event::Text(e) => match tag.as_slice() {
                            b"first-name" => {
                                person.first_name = e.unescape().map(|s| s.to_string()).ok()
                            }
                            b"middle-name" => {
                                person.middle_name = e.unescape().map(|s| s.to_string()).ok()
                            }
                            b"last-name" => {
                                person.last_name = e.unescape().map(|s| s.to_string()).ok()
                            }
                            b"nickname" => {
                                person.nick_name = e.unescape().map(|s| s.to_string()).ok()
                            }
                            _ => (),
                        },
                        Event::End(ref e) if e.local_name().as_ref() == b"author" => {
                            match parent_node {
                                ParentNode::TitleInfo => {
                                    mode = XMode::TitleInfo;
                                    author.push(person);
                                }
                                ParentNode::SrcTitleInfo => {
                                    mode = XMode::SrcTitleInfo;
                                    src_author.push(person);
                                }
                                _ => (),
                            }
                            person = Person::default();
                        }
                        _ => (),
                    },
                    XMode::Translator => match event {
                        Event::Start(e) => tag = e.local_name().as_ref().into(),
                        Event::Text(e) => match tag.as_slice() {
                            b"first-name" => {
                                person.first_name = e.unescape().map(|s| s.to_string()).ok()
                            }
                            b"middle-name" => {
                                person.middle_name = e.unescape().map(|s| s.to_string()).ok()
                            }
                            b"last-name" => {
                                person.last_name = e.unescape().map(|s| s.to_string()).ok()
                            }
                            b"nickname" => {
                                person.nick_name = e.unescape().map(|s| s.to_string()).ok()
                            }
                            _ => (),
                        },
                        Event::End(ref e) if e.local_name().as_ref() == b"translator" => {
                            mode = XMode::TitleInfo;
                            translator.push(person);
                            person = Person::default();
                        }
                        _ => (),
                    },
                    XMode::DocInfo => match event {
                        Event::Start(e) => {
                            tag = e.local_name().as_ref().into();
                            match tag.as_slice() {
                                b"date" => {
                                    if let Some(a) =
                                        get_attr_string("value", &mut e.attributes(), &xml)
                                    {
                                        date.push(a);
                                    }
                                }
                                _ => (),
                            }
                        }
                        Event::Text(e) => match tag.as_slice() {
                            b"id" => {
                                if let Ok(v) = e.unescape() {
                                    id = Some(v.to_string());
                                }
                            }
                            b"date" => {
                                if let Ok(v) = e.unescape() {
                                    date.push(v.to_string());
                                }
                            }
                            _ => (),
                        },
                        Event::End(ref e) if e.local_name().as_ref() == b"document-info" => {
                            mode = XMode::Start
                        }
                        _ => (),
                    },
                    XMode::Annotation(ref parent) => match event {
                        Event::Text(e) => {
                            if let Ok(u) = e.unescape() {
                                annotation.push(u.to_string());
                            }
                        }
                        Event::End(ref e) if e.local_name().as_ref() == b"annotation" => {
                            mode = match parent {
                                ParentNode::TitleInfo => XMode::TitleInfo,
                                _ => XMode::Body(ParentNode::Start),
                            }
                        }
                        _ => (),
                    },
                    XMode::Body(_) => match event {
                        Event::Text(e) => {
                            if let Ok(u) = e.unescape() {
                                body.push(u.to_string());
                            }
                        }
                        Event::End(ref e) if e.local_name().as_ref() == b"body" => {
                            mode = XMode::Start
                        }
                        _ => (),
                    },
                },
            }
        }

        let mut cover_image = None;
        if with_cover {
            if let Some(bt) = cover_b64 {
                match try_decode_base64(bt.into_inner().borrow()) {
                    Ok((raw, warn)) => {
                        cover_image = Some(raw);
                        if !warning.is_empty() {
                            warning.push(warn)
                        }
                    }
                    Err(e) => warning.push(e),
                }
            }
        }

        if with_body && body.is_empty() {
            return Err(ParserError::EmptyBody);
        }
        if title.is_empty() {
            return Err(ParserError::EmptyTitle);
        }

        let length = body.iter().map(|x| x.len() as u64).sum(); //total body length

        //fix common error: comma-delimited list of genres in one <genre> tag
        genre = genre
            .iter()
            .flat_map(|c| c.split(','))
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .filter(|c| *c != "antique") //assigned by default
            .map(|c| c.to_lowercase())
            .collect();

        Ok(Book {
            id,
            encoding: encoding.name().to_string(),
            length,
            title,
            lang,
            date,
            genre,
            keyword,
            author,
            src_author,
            translator,
            sequence,
            seqnum,
            annotation: if with_annotation && !annotation.is_empty() {
                Some(annotation.join(" "))
            } else {
                None
            },
            body: if with_body {
                Some(body.join(" "))
            } else {
                None
            },
            cover_image,
            warning,
        })
    }

    #[allow(clippy::cognitive_complexity, clippy::single_match)]
    fn render_to_html(&self, raw: &[u8]) -> RenderResult {
        let encoding = detect_xml_encoding(raw);
        let (xml_str, _enc, _malformed) = encoding.decode(raw);
        let mut xml = quick_xml::Reader::from_reader(xml_str.as_ref().as_bytes());
        xml.expand_empty_elements(true); //for compatibility with HTML4 <tag/> -> <tag></tag>
        let mut res = Vec::<Event>::new(); //generaged sequence of xhtml events
        let mut mode = XMode::Start;
        let mut in_book_title: bool = false;
        let mut title = String::new();
        let mut img = HashMap::<Vec<u8>, (Vec<u8>, Vec<u8>)>::new(); //image-id -> (content-type,base64-data)
        let mut description_start: usize = 0;
        let mut description_end: usize = 0;

        //phase 1: collect XML events from [ title-info (annotation+cover), bodies, binaries ]
        //map XML tag to HTML tag
        //extract book title
        loop {
            mode = match xml.read_event() {
                Err(_) => mode, //ignore xml error
                Ok(Event::Eof) => break,
                Ok(Event::Start(e)) => {
                    let this_tag = e.local_name();
                    match mode {
                        XMode::Start => match this_tag.as_ref() {
                            b"body" => {
                                // -> <div class="body" id="..">
                                let mut attrs = vec![Attribute::from(("class", "body"))];
                                if let Some(id) = get_attr_raw(b"id", &mut e.attributes()) {
                                    let id = id.value.to_vec();
                                    attrs.push(Attribute {
                                        key: QName(b"id"),
                                        value: Cow::Owned(id),
                                    });
                                }
                                let tag =
                                    Event::Start(BytesStart::new("div").with_attributes(attrs));
                                res.push(tag);
                                XMode::Body(ParentNode::Start)
                            }
                            b"description" => {
                                description_start = xml.buffer_position();
                                mode
                            }
                            b"title-info" => XMode::TitleInfo,
                            b"binary" => {
                                if let Some(id) = get_attr_raw(b"id", &mut e.attributes()) {
                                    let id = id.value.to_vec();
                                    let ct = get_attr_raw(b"content-type", &mut e.attributes())
                                        .map(|a| a.value.to_vec())
                                        .unwrap_or_else(|| b"".to_vec());
                                    XMode::Binary(id, ct)
                                } else {
                                    mode
                                }
                            }
                            _ => mode,
                        },
                        XMode::TitleInfo => match this_tag.as_ref() {
                            b"annotation" => XMode::Annotation(ParentNode::TitleInfo),
                            b"image" => {
                                if let Some(a) = get_attr_raw(b"href", &mut e.attributes()) {
                                    let mut href = a.value.to_vec();
                                    if !href.is_empty() && href[0] == b'#' {
                                        href.remove(0); // "#link" -> "link"
                                    }
                                    let attrs = vec![Attribute {
                                        key: QName(b"href"),
                                        value: Cow::Owned(href),
                                    }];
                                    let tag = Event::Start(
                                        BytesStart::new("image").with_attributes(attrs),
                                    );
                                    res.push(tag);
                                    mode
                                } else {
                                    mode
                                }
                            }
                            b"book-title" => { 
                                in_book_title = true; 
                                mode 
                            }
                            _ => mode,
                        },
                        XMode::Annotation(_) | XMode::Body(_) => match this_tag.as_ref() {
                            b"p" | b"strong" | b"sup" | b"sub" | b"table" | b"tr" | b"th"
                            | b"td" => {
                                res.push(Event::Start(e.to_owned())); //keep as is
                                mode
                            }
                            b"emphasis" => {
                                res.push(Event::Start(BytesStart::new("em")));
                                mode
                            }
                            b"a" | b"image" => {
                                //remove namespace from href="ns:xxx"
                                if let Some(a) = get_attr_raw(b"href", &mut e.attributes()) {
                                    let mut href = a.value.to_vec();
                                    if this_tag.as_ref() == b"image"
                                        && !href.is_empty()
                                        && href[0] == b'#'
                                    {
                                        href.remove(0); // "#link" -> "link"
                                    }
                                    let attrs = vec![Attribute {
                                        key: QName(b"href"),
                                        value: Cow::Owned(href),
                                    }];
                                    let new_tag = Event::Start(
                                        BytesStart::new(
                                            String::from_utf8_lossy(this_tag.as_ref()).into_owned(),
                                        )
                                        .with_attributes(attrs),
                                    );
                                    res.push(new_tag);
                                }
                                mode
                            }
                            b"empty-line" => {
                                res.push(Event::Start(BytesStart::new("br")));
                                mode
                            }
                            tag => {
                                let mut attrs = vec![Attribute {
                                    key: QName(b"class"),
                                    value: Cow::Owned(tag.to_vec()),
                                }];
                                if let Some(a) = get_attr_raw(b"id", &mut e.attributes()) {
                                    let id = a.value.to_vec();
                                    attrs.push(Attribute {
                                        key: QName(b"id"),
                                        value: Cow::Owned(id),
                                    });
                                }
                                let b = BytesStart::new("div").with_attributes(attrs);
                                res.push(Event::Start(b));
                                mode
                            }
                        },
                        _ => mode,
                    }
                }
                Ok(Event::End(e)) => {
                    let tag = e.local_name();
                    in_book_title = false;
                    match mode {
                        XMode::Start if tag.as_ref() == b"description" => {
                            description_end = xml.buffer_position();
                            mode
                        }
                        XMode::Binary(_, _) => XMode::Start,
                        XMode::TitleInfo if tag.as_ref() == b"title-info" => XMode::Start,
                        XMode::Annotation(ref parent) | XMode::Body(ref parent) => {
                            match tag.as_ref() {
                                b"annotation" => {
                                    if let ParentNode::TitleInfo = parent {
                                        XMode::TitleInfo
                                    } else {
                                        //<annotation> inside <body>
                                        res.push(Event::End(BytesEnd::new("div")));
                                        mode
                                    }
                                }
                                b"body" => {
                                    res.push(Event::End(BytesEnd::new("div")));
                                    XMode::Start
                                }
                                b"a" | b"p" | b"strong" | b"sup" | b"sub" | b"table" | b"tr"
                                | b"th" | b"td" => {
                                    res.push(Event::End(e.to_owned())); //keep as is
                                    mode
                                }
                                b"emphasis" => {
                                    res.push(Event::End(BytesEnd::new("em")));
                                    mode
                                }
                                b"empty-line" | b"image" => mode,
                                _ => {
                                    res.push(Event::End(BytesEnd::new("div")));
                                    mode
                                }
                            }
                        }
                        _ => mode,
                    }
                }
                Ok(Event::Text(e)) => match mode {
                    XMode::TitleInfo if in_book_title => {
                        title = e.unescape().unwrap().into_owned();
                        mode
                    }
                    XMode::Binary(id, ct) => {
                        let b64 = e.into_inner().into_owned();
                        img.insert(id, (ct, b64));
                        XMode::Start
                    }
                    XMode::Annotation(_) | XMode::Body(_) => {
                        res.push(Event::Text(e.to_owned()));
                        mode
                    }
                    _ => mode,
                },
                _ => mode,
            }
        }

        //phase 2: parse <description> tag again, construct HTML tree with all technical information ("book imprint")
        // <tag aaa="bbb">xxx</tag> -> <div><span class="name">tag</span><span class="value">aaa=bbb xxx</span><div>
        let attrs = vec![Attribute::from(("class", "description"))];
        res.push(Event::Start(BytesStart::new("div").with_attributes(attrs)));
        let mut xml = quick_xml::Reader::from_str(&xml_str[description_start..description_end]);
        xml.expand_empty_elements(true);
        loop {
            match xml.read_event() {
                Err(_) => (), //ignore xml error
                Ok(Event::Eof) => break,
                Ok(Event::Start(ref e)) => {
                    res.push(Event::Start(BytesStart::new("div")));
                    res.push(Event::Start(
                        BytesStart::new("span")
                            .with_attributes(vec![Attribute::from(("class", "name"))]),
                    ));
                    let tag_name =
                        String::from_utf8(e.name().as_ref().to_vec()).unwrap_or_default();
                    res.push(Event::Text(BytesText::from_escaped(tag_name)));
                    res.push(Event::End(BytesEnd::new("span")));
                    res.push(Event::Start(
                        BytesStart::new("span")
                            .with_attributes(vec![Attribute::from(("class", "value"))]),
                    ));
                    for a in e.attributes().flatten() {
                        let v = a.decode_and_unescape_value(&xml).unwrap_or_default();
                        let txt = format!(
                            "{}={} ",
                            String::from_utf8_lossy(a.key.as_ref()).to_owned(),
                            v,
                        );
                        res.push(Event::Text(BytesText::from_escaped(Cow::Owned(txt))));
                    }
                }
                Ok(Event::Text(text)) => res.push(Event::Text(text.to_owned())),
                Ok(Event::End(_)) => {
                    res.push(Event::End(BytesEnd::new("span")));
                    res.push(Event::End(BytesEnd::new("div")));
                }
                _ => (),
            }
        }
        res.push(Event::End(BytesEnd::new("div"))); //</description>

        //phase 3: construct HTML, inline image content
        let mut writer = quick_xml::Writer::new(std::io::Cursor::new(Vec::new()));
        for event in res {
            match event {
                Event::Start(ref e) => {
                    if e.local_name().as_ref() == b"image" {
                        if let Some(href) = get_attr_raw(b"href", &mut e.attributes()) {
                            let id = href.value.to_vec();
                            if let Some((ct, data)) = img.get(&id) {
                                let mut src = b"data:".to_vec();
                                src.extend_from_slice(ct); //content-type
                                src.extend_from_slice(b" ;base64, ");
                                src.extend_from_slice(data); //image data
                                let attrs = vec![Attribute {
                                    key: QName(b"src"),
                                    value: Cow::Owned(src),
                                }];
                                let b = BytesStart::new("img").with_attributes(attrs);
                                writer.write_event(Event::Start(b)).unwrap();
                            }
                        }
                    } else {
                        writer.write_event(event).unwrap();
                    }
                }
                Event::End(_) => {
                    writer.write_event(event).unwrap();
                }
                Event::Text(_) => {
                    writer.write_event(event).unwrap();
                }
                _ => (),
            }
        }

        let content = writer.into_inner().into_inner();
        let content = String::from_utf8(content).unwrap();
        Ok((title,content))
    }
}

fn is_base64(x: u8) -> bool {
    //standard base64 chars: + / 0-9 A-Z a-z
    x == 43 || (47..=57).contains(&x) || (65..=90).contains(&x) || (97..=122).contains(&x)
}

/// base64 raw string -> (decoded raw, warning) | error
pub fn try_decode_base64(b64: &[u8]) -> Result<(Vec<u8>, String), String> {
    let mut buf: Vec<u8>;
    let mut warning = String::new();
    let mut b64_ref = b64;
    let config = base64::STANDARD.decode_allow_trailing_bits(true);
    //remove non-base64 chars
    if b64.iter().any(|&x| is_base64(x)) {
        buf = b64.iter().filter(|&&x| is_base64(x)).copied().collect();
        b64_ref = &buf[..];
    }
    match base64::decode_config(b64_ref, config) {
        Ok(raw) => buf = raw,
        Err(base64::DecodeError::InvalidByte(offset, _)) => {
            let upto = offset - offset % 4; //align to 4-byte chunk and retry decoding
            match base64::decode_config(&b64_ref[0..upto], config) {
                Ok(raw) => {
                    warning = format!("Image truncated, invalid byte at {}", offset);
                    buf = raw;
                }
                Err(e) => return Err(format!("Invalid image: {}", e)),
            }
        }
        Err(e) => return Err(format!("Invalid image: {}", e)),
    }
    Ok((buf, warning))
}

fn find_raw(needle: &[u8], haystack: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn detect_xml_encoding<'a>(head: &'a [u8]) -> &'static Encoding {
    let mut enc: &Encoding = UTF_8;
    if head.len() > 3 {
        match (head[0], head[1], head[2]) {
            (0xEF, 0xBB, 0xBF) => enc = UTF_8, //BOM
            (0xFF, 0xFE, _) => enc = UTF_16LE, //BOM
            (0xFE, 0xFF, _) => enc = UTF_16BE, //BOM
            (0x3C, 0x00, _) => enc = UTF_16LE, //<
            (0x00, 0x3C, _) => enc = UTF_16BE, //<
            _ => {
                let upto = if head.len() < 128 { head.len() } else { 128 };
                if let Some(s) = find_raw(b"encoding=\"", &head[0..upto]) {
                    if let Some(e) = find_raw(b"\"", &head[s + 10..upto]) {
                        let name = &head[s + 10..s + 10 + e];
                        enc = Encoding::for_label(name).unwrap_or(UTF_8);
                    }
                }
            }
        }
    }
    enc
}
