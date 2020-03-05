use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::io::BufRead;

#[derive(Debug, Serialize)]
pub struct GenreMap {
    code_to_prim: HashMap<String, String>, //genre code to primary code
    prim_to_cat: HashMap<String, String>,  //primary genre code to category code
    pub translation: HashMap<String, String>, //(primary genre code|category code) to translation
}

impl GenreMap {
    pub fn load(reader: &mut dyn BufRead) -> Result<Self, std::io::Error> {
        let mut cp = HashMap::new();
        let mut pc = HashMap::new();
        let mut tr = HashMap::new();
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
                            tr.insert(category.clone(), desc.to_string());
                        } else {
                            let mut codes = code.split("+");
                            //first code is primary
                            if let Some(primary) = codes.next() {
                                pc.insert(primary.to_string(), category.clone());
                                tr.insert(primary.to_string(), desc.to_string());
                                //next code is alias
                                for c in codes {
                                    cp.insert(c.to_string(), primary.to_string());
                                }
                                //use translation as extra alias
                                cp.insert(desc.to_lowercase(), primary.to_string());
                            }
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(GenreMap {
            code_to_prim: cp,
            prim_to_cat: pc,
            translation: tr,
        })
    }

    pub fn path_for<'a>(&'a self, code: &str) -> String {
        //normalize code
        let code = code
            .chars()
            .filter(|&c| c.is_alphanumeric() || c.is_whitespace() || c == '-' || c == '_')
            .collect::<String>()
            .to_lowercase();
        //map to primary code
        let primary = self.code_to_prim.get(&code).unwrap_or(&code);
        //map to category
        let cat = self
            .prim_to_cat
            .get(primary)
            .map(|x| x.as_str())
            .unwrap_or("misc");
        format!("{}/{}", cat, primary)
    }
}
