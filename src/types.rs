use deepsize::DeepSizeOf;
use std::collections::HashMap;
use std::io::BufRead;

#[derive(Debug, DeepSizeOf)]
pub struct Person {
    pub first_name: Option<String>,
    pub middle_name: Option<String>,
    pub last_name: Option<String>,
    pub nick_name: Option<String>,
}

#[derive(Debug, DeepSizeOf)]
pub struct Book {
    pub id: Option<String>,
    pub zipfile: String,
    pub filename: String,
    pub encoding: String,
    pub length: u64,
    pub title: Vec<String>, // title | translated-title,source-title
    pub lang: Vec<String>,  // lang | translated-lang,source-lang
    pub date: Vec<String>,
    pub genre: Vec<String>,
    pub keyword: Vec<String>,
    pub author: Vec<Person>,
    pub src_author: Vec<Person>,
    pub translator: Vec<Person>,
    pub cover_image: Option<Vec<u8>>,
    pub sequence: Vec<String>,
    pub seqnum: Vec<i64>,
    pub annotation: Option<String>,
    pub body: Option<String>,
    pub warning: Vec<String>,
}

#[derive(Debug)]
pub enum ParserError {
    EmptyBody,
    EmptyTitle,
}

impl std::fmt::Display for ParserError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ParserError::EmptyBody => write!(f, "Empty body"),
            ParserError::EmptyTitle => write!(f, "Empty title"),
        }
    }
}

pub type ParserResult = std::result::Result<Book, ParserError>;
pub type RenderResult = std::result::Result<Vec<u8>, String>;

pub trait BookFormat: Send + Sync {
    fn file_extension(&self) -> &'static str;
    fn content_type(&self) -> &'static str;

    fn parse(
        &self,
        zipfile: &str,
        filename: &str,
        reader: &mut dyn BufRead,
        with_body: bool,
        with_annotation: bool,
        with_cover: bool,
    ) -> ParserResult;

    fn str_to_html(&self, decoded_xml: &str) -> RenderResult;
}

pub type BookFormats = HashMap<&'static str, Box<dyn BookFormat + Send + Sync>>;

use std::fmt::{Display, Formatter, Result};

pub fn person_to_string(p: &Vec<Person>) -> String {
    p.into_iter()
        .map(|x| x.to_string())
        .collect::<Vec<String>>()
        .join(", ")
}

impl Display for Book {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let seq: Vec<String> = self
            .sequence
            .iter()
            .zip(self.seqnum.iter())
            .map(|(name, num)| format!("{}-{}", name, num))
            .collect();
        write!(f, "zip={} file={} enc={} lang={} len={} title={} date={} genre={} author={} src.author={} trans={} seq={} keyword={} ann.len={} img.len={} warn={}", 
           &self.zipfile, &self.filename, &self.encoding, &self.lang.join(" / "), self.length, &self.title.join(" / "), 
           self.date.join(" / "),
           self.genre.join(", "),
           person_to_string(&self.author),
           person_to_string(&self.src_author),
           person_to_string(&self.translator),
           seq.join(", "),
           self.keyword.join(", "),
           self.annotation.as_ref().map_or(String::new(), |x| x.len().to_string()),
           self.cover_image.as_ref().map(|x| x.len()).unwrap_or(0),
           self.warning.join(", "),
        )
    }
}

impl Book {
    pub fn size_of(&self) -> usize {
        self.deep_size_of()
    }
}

impl std::fmt::Display for Person {
    // -> Last First Middle [nick]
    fn fmt(&self, f: &mut Formatter) -> Result {
        let mut r = "".to_string();
        if let Some(ref x) = self.last_name {
            if x.len() > 0 {
                if r.len() > 0 {
                    r.push(' ');
                }
                r.push_str(x);
            }
        }
        if let Some(ref x) = self.first_name {
            if x.len() > 0 {
                if r.len() > 0 {
                    r.push(' ');
                }
                r.push_str(x);
            }
        }
        if let Some(ref x) = self.middle_name {
            if x.len() > 0 {
                if r.len() > 0 {
                    r.push(' ');
                }
                r.push_str(x);
            }
        }
        if let Some(ref x) = self.nick_name {
            if x.len() > 0 {
                if r.len() > 0 {
                    r.push(' ');
                }
                r.push('[');
                r.push_str(x);
                r.push(']');
            }
        }
        write!(f, "{}", r)
    }
}

fn is_name_delimiter(x: char) -> bool {
    x == '\u{002D}'
        || x == '\u{2010}'
        || x == '\u{2011}'
        || x == '\u{2012}'
        || x == '\u{2013}'
        || x == '\u{2014}'
        || x == '\u{FE58}'
        || x == '\u{FE63}'
        || x == '\u{FF0D}'
}

impl Person {
    pub fn new() -> Person {
        Person {
            first_name: None,
            middle_name: None,
            last_name: None,
            nick_name: None,
        }
    }

    /// takes first word from last name, convert it to Proper-Case
    pub fn last_name_normalized(&self) -> Option<String> {
        if let Some(n) = &self.last_name {
            let name: String = n
                .chars()
                .take_while(|&x| x.is_alphabetic() || is_name_delimiter(x))
                .collect(); //Name | Name-name
            if name.len() > 0 {
                Some(
                    name.split('-')
                        .filter(|x| x.len() > 0)
                        .map(|x| {
                            let mut s = x.chars().take(1).collect::<String>().to_uppercase();
                            s.extend(x.chars().skip(1).collect::<String>().to_lowercase().chars());
                            s
                        })
                        .collect::<Vec<String>>()
                        .join("-"),
                )
            } else {
                None
            }
        } else {
            None
        }
    }
}
