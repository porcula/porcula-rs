use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::io::BufRead;

#[derive(Debug, Serialize)]
pub struct GenreMap {
    pub category: HashMap<String, String>, //genre code to category
    pub translation: HashMap<String, String>, //genre code to translation
}

impl GenreMap {
    pub fn load(reader: &mut dyn BufRead) -> Result<Self, std::io::Error> {
        let mut gc = HashMap::new();
        let mut gt = HashMap::new();
        let mut category: String = "misc".to_string();
        let re = Regex::new(r"([#/]?)([^=]+)=(.+)").unwrap();
        for i in reader.lines() {
            match i {
                Ok(line) => {
                    if let Some(m) = re.captures(&line) {
                        let flag = m.get(1).map_or("", |m| m.as_str());
                        let code = m.get(2).map_or("", |m| m.as_str());
                        let desc = m.get(3).map_or("", |m| m.as_str());
                        if flag == "#" {
                            continue;
                        } //commented line
                        if flag == "/" {
                            category = code.to_string();
                        } else {
                            gc.insert(code.to_string(), category.clone());
                        }
                        gt.insert(code.to_string(), desc.to_string());
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(GenreMap {
            category: gc,
            translation: gt,
        })
    }
}
