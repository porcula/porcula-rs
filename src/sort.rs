use std::cmp::{Ordering, PartialOrd};
use std::collections::HashMap;

lazy_static! {
    static ref ORDER: HashMap::<char, usize> = {
        const ORDERED: &str = "АаБбВвГгДдЕеЁёЖжЗзИиЙйКкЛлМмНнОоПпРрСсТтУуФфХхЦцЧчШшЩщЪъЫыЬьЭэЮюЯяAaBbCcDdEeFfGgHhIiJjKkLlMmNnOoPpQqRrSsTtUuVvWwXxYyZz0123456789";
        ORDERED.chars().enumerate().map(|i| (i.1, i.0)).collect()
    };
}

#[derive(Eq, PartialEq, Debug)]
pub struct LocalStr <'a> (pub &'a str);

#[derive(Eq, PartialEq, Debug)]
pub struct LocalString (pub String);

impl Ord for LocalStr<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        //empty string -> to end
        //non-alphanumeric -> to end
        //ignore non-alphanumerics
        match (self.0.is_empty(), other.0.is_empty()) {
            (true, true) => Ordering::Equal,
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            (false, false) => {
                let mut ai = self
                    .0
                    .chars()
                    .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                    .peekable();
                let mut bi = other
                    .0
                    .chars()
                    .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                    .peekable();
                match (ai.peek().is_some(), bi.peek().is_some()) {
                    (false, false) => self.0.cmp(&other.0),
                    (false, true) => Ordering::Greater,
                    (true, false) => Ordering::Less,
                    (true, true) => {
                        for (a, b) in ai.zip(bi) {
                            if a == b {
                                continue;
                            }
                            return match (ORDER.get(&a), ORDER.get(&b)) {
                                (Some(a2), Some(b2)) => a2.cmp(&b2),
                                (Some(_), None) => Ordering::Less,
                                (None, Some(_)) => Ordering::Greater,
                                (None, None) => self.0.cmp(&other.0),
                            };
                        }
                        Ordering::Equal
                    }
                }
            }
        }
    }
}

impl PartialOrd for LocalStr<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LocalString {
    fn cmp(&self, other: &Self) -> Ordering {
        LocalStr(&self.0).cmp(&LocalStr(&other.0))
    }
}

impl PartialOrd for LocalString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[test]
fn test_sort() {
    let a = LocalStr("Фыва");
    let b = LocalStr("Asdf");
    assert_eq!(a.cmp(&b), Ordering::Less);

    let mut a = vec![
        "",
        "123",
        "*",
        "Eeny",
        "meeny",
        "miny",
        "moe",
        "Мама",
        "...мыла",
        "раму",
        "Маша",
        "«ела»",
        "кашу",
        "ёлка",
    ];
    let b = vec![
        "«ела»",
        "ёлка",
        "кашу",
        "Мама",
        "Маша",
        "...мыла",
        "раму",
        "Eeny",
        "meeny",
        "miny",
        "moe",
        "123",
        "*",
        "",
    ];
    a.sort_by_cached_key(|x| LocalStr(x));
    assert_eq!(a, b);
}
