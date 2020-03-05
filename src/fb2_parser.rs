use crate::types::*;
use quick_xml::events::attributes::{Attribute, Attributes};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::BufRead;
use std::str;

pub struct FB2BookFormat;

enum ParentNode {
    TitleInfo,
    SrcTitleInfo,
}

enum XMode<'a> {
    Start,
    Body,
    TitleInfo,
    SrcTitleInfo,
    DocInfo,
    Author(ParentNode),
    Translator,
    Annotation,
    Binary(Cow<'a, [u8]>, Cow<'a, [u8]>), // (id,content-type)
}

fn get_attr_raw<'a>(name: &[u8], attrs: &'a mut Attributes) -> Option<Attribute<'a>> {
    attrs.filter_map(|x| x.ok()).find(|a| {
        if a.key.eq(name) {
            return true;
        } //attr <tag attr="val">
        if a.key.len() > name.len() + 1 {
            //<tag x:attr="val"> - ignore namespace
            let s = a.key.len() - name.len();
            a.key[s..].eq(name) && a.key[s - 1] == b':'
        } else {
            false
        }
    })
}

fn get_attr_string<B: BufRead>(
    name: &str,
    attrs: &mut Attributes,
    xml: &quick_xml::Reader<B>,
) -> Option<String> {
    match get_attr_raw(name.as_bytes(), attrs) {
        Some(a) => a.unescape_and_decode_value(&xml).ok(),
        None => None,
    }
}

impl BookFormat for FB2BookFormat {
    fn file_extension(&self) -> &'static str {
        ".fb2"
    }
    fn content_type(&self) -> &'static str {
        "application/fb2"
    }

    fn parse(
        &self,
        zipfile: &str,
        filename: &str,
        reader: &mut dyn BufRead,
        with_body: bool,
        with_annotation: bool,
        with_cover: bool,
    ) -> Result<Book, ParserError> {
        let mut xml = quick_xml::Reader::from_reader(reader);
        xml.trim_text(true);
        let mut buf: Vec<u8> = Vec::new();
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
        let mut person = Person::new();
        let mut genre = Vec::<String>::new();
        let mut keyword = Vec::<String>::new();
        let mut title = Vec::<String>::new();
        let mut sequence = Vec::<String>::new();
        let mut seqnum = Vec::<i64>::new();
        let mut lang = Vec::<String>::new();
        let mut date = Vec::<String>::new();
        let mut annotation = Vec::<String>::new();
        let mut body = Vec::<String>::new();
        let mut warning = Vec::<String>::new();
        loop {
            let event = xml.read_event(&mut buf);
            match event {
                //continue processing non-valid XML
                Err(ref e) => warning.push(format!(
                    "Error at position {}: {:?}",
                    &xml.buffer_position(),
                    e
                )),
                Ok(Event::Eof) => break,
                _ => (),
            }
            match mode {
                XMode::Start => match event {
                    Ok(Event::Start(ref e)) => {
                        match e.name() {
                            b"body" => {
                                if with_body {
                                    mode = XMode::Body;
                                } else {
                                    xml.read_to_end(b"body", &mut buf).unwrap_or_else(|e| {
                                        warning.push(format!(
                                            "Error at position {}: {:?}",
                                            &xml.buffer_position(),
                                            e
                                        ))
                                    });
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
                                            mode = XMode::Binary(Cow::Owned(id), Cow::Owned(ct));
                                        }
                                        _ => (),
                                    }
                                } else {
                                    xml.read_to_end(b"binary", &mut buf).unwrap_or_else(|e| {
                                        warning.push(format!(
                                            "Error at position {}: {:?}",
                                            &xml.buffer_position(),
                                            e
                                        ))
                                    });
                                }
                            }
                            _ => tag = e.name().to_vec(),
                        }
                    }
                    _ => (),
                },
                XMode::Binary(ref _id, ref _ct) => match event {
                    Ok(Event::End(_)) => mode = XMode::Start,
                    Ok(Event::Text(e)) if cover_prob > cover_load => {
                        cover_b64 = Some(e.into_owned());
                        cover_load = cover_prob;
                    }
                    _ => (),
                },
                XMode::TitleInfo => match event {
                    Ok(Event::Start(ref e)) => match e.name() {
                        b"author" => mode = XMode::Author(ParentNode::TitleInfo),
                        b"translator" => mode = XMode::Translator,
                        b"annotation" => {
                            if with_annotation {
                                mode = XMode::Annotation;
                            } else {
                                xml.read_to_end(b"annotation", &mut buf)
                                    .unwrap_or_else(|e| {
                                        warning.push(format!(
                                            "Error at position {}: {:?}",
                                            &xml.buffer_position(),
                                            e
                                        ))
                                    });
                            }
                        }
                        _ => tag = e.name().to_vec(),
                    },
                    Ok(Event::Empty(ref e)) => match e.name() {
                        b"sequence" => {
                            let mut attrs = e.attributes();
                            if let Some(name) = get_attr_string("name", &mut attrs, &xml) {
                                let mut num: i64 = 0;
                                if let Some(n) = get_attr_string("number", &mut attrs, &xml) {
                                    if let Ok(i) = n.parse::<i64>() {
                                        num = i;
                                    }
                                }
                                sequence.push(name);
                                seqnum.push(num);
                            }
                        }
                        b"image" => {
                            if let Some(v) = get_attr_string("href", &mut e.attributes(), &xml) {
                                coverpage_href = v.trim_start_matches("#").to_string();
                                // "#link" -> "link"
                            }
                        }
                        _ => (),
                    },
                    Ok(Event::Text(e)) => match tag.as_slice() {
                        b"book-title" => {
                            if let Ok(v) = e.unescape_and_decode(&xml) {
                                title.push(v);
                            }
                        }
                        b"lang" => {
                            if let Ok(mut v) = e.unescape_and_decode(&xml) {
                                if v.len() > 2 {
                                    v = v[0..2].to_string()
                                } //2-letter ISO 639-1
                                v = v.to_lowercase();
                                lang.push(v);
                            }
                        }
                        b"genre" => {
                            if let Ok(v) = e.unescape_and_decode(&xml) {
                                genre.push(v);
                            }
                        }
                        b"date" => {
                            if let Ok(v) = e.unescape_and_decode(&xml) {
                                date.push(v);
                            }
                        }
                        b"keywords" => {
                            if let Ok(v) = e.unescape_and_decode(&xml) {
                                for i in v.split(',') {
                                    keyword.push(i.trim().to_lowercase());
                                }
                            }
                        }
                        _ => (),
                    },
                    Ok(Event::End(ref e)) if e.name() == b"title-info" => mode = XMode::Start,
                    _ => (),
                },
                XMode::SrcTitleInfo => match event {
                    Ok(Event::Start(ref e)) => match e.name() {
                        b"author" => mode = XMode::Author(ParentNode::SrcTitleInfo),
                        _ => tag = e.name().to_vec(),
                    },
                    Ok(Event::Text(e)) => match tag.as_slice() {
                        //single field for translation / source
                        b"book-title" => {
                            if let Ok(v) = e.unescape_and_decode(&xml) {
                                title.push(v);
                            }
                        }
                        b"lang" => {
                            if let Ok(v) = e.unescape_and_decode(&xml) {
                                lang.push(v);
                            }
                        }
                        b"date" => {
                            if let Ok(v) = e.unescape_and_decode(&xml) {
                                date.push(v);
                            }
                        }
                        _ => (),
                    },
                    Ok(Event::End(ref e)) if e.name() == b"src-title-info" => mode = XMode::Start,
                    _ => (),
                },
                XMode::Author(ref parent_node) => match event {
                    Ok(Event::Start(ref e)) => tag = e.name().to_vec(),
                    Ok(Event::Text(e)) => match tag.as_slice() {
                        b"first-name" => person.first_name = e.unescape_and_decode(&xml).ok(),
                        b"middle-name" => person.middle_name = e.unescape_and_decode(&xml).ok(),
                        b"last-name" => person.last_name = e.unescape_and_decode(&xml).ok(),
                        b"nickname" => person.nick_name = e.unescape_and_decode(&xml).ok(),
                        _ => (),
                    },
                    Ok(Event::End(ref e)) if e.name() == b"author" => {
                        match parent_node {
                            ParentNode::TitleInfo => {
                                mode = XMode::TitleInfo;
                                author.push(person);
                            }
                            ParentNode::SrcTitleInfo => {
                                mode = XMode::SrcTitleInfo;
                                src_author.push(person);
                            }
                        }
                        person = Person::new();
                    }
                    _ => (),
                },
                XMode::Translator => match event {
                    Ok(Event::Start(ref e)) => tag = e.name().to_vec(),
                    Ok(Event::Text(e)) => match tag.as_slice() {
                        b"first-name" => person.first_name = e.unescape_and_decode(&xml).ok(),
                        b"middle-name" => person.middle_name = e.unescape_and_decode(&xml).ok(),
                        b"last-name" => person.last_name = e.unescape_and_decode(&xml).ok(),
                        b"nickname" => person.nick_name = e.unescape_and_decode(&xml).ok(),
                        _ => (),
                    },
                    Ok(Event::End(ref e)) if e.name() == b"translator" => {
                        mode = XMode::TitleInfo;
                        translator.push(person);
                        person = Person::new();
                    }
                    _ => (),
                },
                XMode::DocInfo => match event {
                    Ok(Event::Start(ref e)) => tag = e.name().to_vec(),
                    Ok(Event::Text(e)) => match tag.as_slice() {
                        b"id" => {
                            if let Ok(v) = e.unescape_and_decode(&xml) {
                                id = Some(v);
                            }
                        }
                        b"date" => {
                            if let Ok(v) = e.unescape_and_decode(&xml) {
                                date.push(v);
                            }
                        }
                        _ => (),
                    },
                    Ok(Event::End(ref e)) if e.name() == b"document-info" => mode = XMode::Start,
                    _ => (),
                },
                XMode::Annotation => match event {
                    Ok(Event::Text(e)) => {
                        if let Ok(u) = e.unescaped() {
                            annotation.push(String::from(xml.decode(&u)));
                        }
                    }
                    Ok(Event::End(ref e)) if e.name() == b"annotation" => mode = XMode::TitleInfo,
                    _ => (),
                },
                XMode::Body => match event {
                    Ok(Event::Text(e)) => {
                        if let Ok(u) = e.unescaped() {
                            body.push(String::from(xml.decode(&u)));
                        }
                    }
                    Ok(Event::End(ref e)) if e.name() == b"body" => mode = XMode::Start,
                    _ => (),
                },
            }
            buf.clear();
        }

        let mut cover_image = None;
        if with_cover {
            if let Some(bt) = cover_b64 {
                match try_decode_base64(bt.escaped()) {
                    Ok((raw, warn)) => {
                        cover_image = Some(raw);
                        if !warning.is_empty() {
                            warning.push(warn)
                        }
                    }
                    Err(e) => warning.push(e.to_string()),
                }
            }
        }

        if with_body && body.is_empty() {
            return Err(ParserError::EmptyBody);
        }
        if title.is_empty() {
            return Err(ParserError::EmptyTitle);
        }

        let length = body
            .iter()
            .map(|x| x.len() as u64)
            .fold(0, |acc, x| acc + x); //total body length

        //fix common error: comma-delimited list of genres in one <genre> tag
        genre = genre
            .iter()
            .flat_map(|c| c.split(","))
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .map(|c| c.to_lowercase())
            .collect();

        Ok(Book {
            id: id,
            zipfile: zipfile.into(),
            filename: filename.into(),
            encoding: xml.encoding().name().to_string(),
            length: length,
            title: title,
            lang: lang,
            date: date,
            genre: genre,
            keyword: keyword,
            author: author,
            src_author: src_author,
            translator: translator,
            sequence: sequence,
            seqnum: seqnum,
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
            cover_image: cover_image,
            warning: warning,
        })
    }

    fn str_to_html(&self, decoded_xml: &str) -> RenderResult {
        let mut res = Vec::<Event>::new();
        let mut xml = quick_xml::Reader::from_str(decoded_xml);
        xml.expand_empty_elements(true); //for compatibility with HTML4 <tag/> -> <tag></tag>
        let mut buf = Vec::new();
        let mut mode = XMode::Start;
        let mut img = HashMap::<Cow<[u8]>, (Cow<[u8]>, Cow<[u8]>)>::new(); //image-id -> (content-type,base64-data)

        //phase 1: collect XML events from [ title-info (annotation+cover), bodies, binaries ]
        //doing mapping to HTML tags
        loop {
            let event = xml.read_event(&mut buf);
            match event {
                Err(_) => (), //ignore xml error
                Ok(Event::Eof) => break,
                _ => (),
            }
            match mode {
                XMode::Start => match event {
                    Ok(Event::Start(ref e)) => {
                        match e.name() {
                            b"body" => {
                                // -> <div class="body" id="..">
                                let mut attrs = vec![Attribute {
                                    key: b"class",
                                    value: Cow::Borrowed(b"body"),
                                }];
                                if let Some(id) = get_attr_raw(b"id", &mut e.attributes()) {
                                    let id = id.value.to_vec();
                                    attrs.push(Attribute {
                                        key: b"id",
                                        value: Cow::Owned(id),
                                    });
                                }
                                let tag = Event::Start(
                                    BytesStart::borrowed_name(b"div").with_attributes(attrs),
                                );
                                res.push(tag);
                                mode = XMode::Body;
                            }
                            b"title-info" => mode = XMode::TitleInfo,
                            b"binary" => {
                                if let Some(id) = get_attr_raw(b"id", &mut e.attributes()) {
                                    let id = id.value.to_vec();
                                    let ct = get_attr_raw(b"content-type", &mut e.attributes())
                                        .map(|a| a.value.to_vec())
                                        .unwrap_or(b"".to_vec());
                                    mode = XMode::Binary(Cow::Owned(id), Cow::Owned(ct));
                                }
                            }
                            _ => (),
                        }
                    }
                    _ => (),
                },
                XMode::Binary(ref id, ref ct) => match event {
                    Ok(Event::End(_)) => mode = XMode::Start,
                    Ok(Event::Text(e)) => {
                        let b64 = e.escaped().to_vec();
                        img.insert(id.clone(), (ct.clone(), Cow::Owned(b64)));
                    }
                    _ => (),
                },
                XMode::TitleInfo => match event {
                    Ok(Event::Start(ref e)) => match e.name() {
                        b"annotation" => mode = XMode::Annotation,
                        b"image" => {
                            if let Some(a) = get_attr_raw(b"href", &mut e.attributes()) {
                                let mut href = a.value.to_vec();
                                if href.len() > 0 && href[0] == b'#' {
                                    href.remove(0); // "#link" -> "link"
                                }
                                let attrs = vec![Attribute {
                                    key: b"href".as_ref(),
                                    value: Cow::Owned(href),
                                }];
                                let tag = Event::Start(
                                    BytesStart::borrowed_name(b"img").with_attributes(attrs),
                                );
                                res.push(tag);
                            }
                        }
                        _ => (),
                    },
                    Ok(Event::End(ref e)) if e.name() == b"title-info" => mode = XMode::Start,
                    _ => (),
                },
                XMode::Annotation | XMode::Body => match &event {
                    Ok(Event::Start(ref e)) => {
                        match e.name() {
                            b"p" | b"strong" | b"sup" | b"sub" | b"table" | b"tr" | b"th"
                            | b"td" => res.push(Event::Start(e.to_owned())), //keep as is
                            b"emphasis" => res.push(Event::Start(BytesStart::borrowed_name(b"em"))),
                            b"a" | b"image" => {
                                //remove namespace from href="ns:xxx"
                                if let Some(a) = get_attr_raw(b"href", &mut e.attributes()) {
                                    let mut href = a.value.to_vec();
                                    if e.name() == b"image" && href.len() > 0 && href[0] == b'#' {
                                        href.remove(0); // "#link" -> "link"
                                    }
                                    let attrs = vec![Attribute {
                                        key: b"href".as_ref(),
                                        value: Cow::Owned(href),
                                    }];
                                    let tag = Event::Start(
                                        BytesStart::owned_name(e.name().to_vec())
                                            .with_attributes(attrs),
                                    );
                                    res.push(tag);
                                }
                            }
                            b"empty-line" => {
                                res.push(Event::Start(BytesStart::borrowed_name(b"br")))
                            }
                            tag => {
                                let mut attrs = vec![Attribute {
                                    key: b"class",
                                    value: Cow::Owned(tag.to_vec()),
                                }];
                                if let Some(a) = get_attr_raw(b"id", &mut e.attributes()) {
                                    let id = a.value.to_vec();
                                    attrs.push(Attribute {
                                        key: b"id".as_ref(),
                                        value: Cow::Owned(id),
                                    });
                                }
                                let b = BytesStart::borrowed_name(b"div").with_attributes(attrs);
                                res.push(Event::Start(b));
                            }
                        }
                    }
                    Ok(Event::Text(_)) => res.push(event.unwrap().into_owned()),
                    Ok(Event::End(ref e)) => match e.name() {
                        b"annotation" => mode = XMode::TitleInfo,
                        b"body" => {
                            res.push(Event::End(BytesEnd::borrowed(b"div")));
                            mode = XMode::Start;
                        }
                        b"a" | b"p" | b"strong" | b"sup" | b"sub" | b"table" | b"tr" | b"th"
                        | b"td" => res.push(event.unwrap().into_owned()),
                        b"emphasis" => res.push(Event::End(BytesEnd::borrowed(b"em"))),
                        b"empty-line" | b"image" => (),
                        _ => res.push(Event::End(BytesEnd::borrowed(b"div"))),
                    },
                    _ => (),
                },
                _ => (),
            }
            buf.clear();
        }

        //phase 2: construct HTML, inline image content
        let mut writer = quick_xml::Writer::new(std::io::Cursor::new(Vec::new()));
        for event in res {
            match event {
                Event::Start(ref e) => {
                    if e.name() == b"image" {
                        if let Some(href) = get_attr_raw(b"href", &mut e.attributes()) {
                            if let Some(i) = img.get(&href.value) {
                                let mut src = b"data:".to_vec();
                                src.extend_from_slice(&*i.0); //content-type
                                src.extend_from_slice(b" ;base64, ");
                                src.extend_from_slice(&*i.1); //image data
                                let mut attrs = vec![];
                                attrs.push(Attribute {
                                    key: b"src",
                                    value: Cow::Owned(src),
                                });
                                let b = BytesStart::borrowed_name(b"img").with_attributes(attrs);
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
        let result = writer.into_inner().into_inner();
        Ok(result)
    }
}

fn is_base64(x: u8) -> bool {
    //standard base64 chars: + / 0-9 A-Z a-z
    x == 43 || (x >= 47 && x <= 57) || (x >= 65 && x <= 90) || (x >= 97 && x <= 122)
}

/// base64 raw string -> (decoded raw, warning) | error
pub fn try_decode_base64(b64: &[u8]) -> Result<(Vec<u8>, String), String> {
    let mut buf: Vec<u8>;
    let mut warning = String::new();
    let mut b64_ref = b64;
    let config = base64::STANDARD.decode_allow_trailing_bits(true);
    //remove non-base64 chars
    if let Some(_) = b64.iter().find(|&&x| is_base64(x)) {
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
