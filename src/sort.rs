use std::cmp::{Ordering, PartialOrd};
use std::collections::HashMap;
use std::iter::FromIterator;

lazy_static! {
    static ref ORDER: HashMap::<char, usize> = {
        const ORDERED: &str = "АаБбВвГгДдЕеЁёЖжЗзИиЙйКкЛлМмНнОоПпРрСсТтУуФфХхЦцЧчШшЩщЪъЫыЬьЭэЮюЯяAaBbCcDdEeFfGgHhIiJjKkLlMmNnOoPpQqRrSsTtUuVvWwXxYyZz0123456789";
        HashMap::from_iter(ORDERED.chars().enumerate().map(|i| (i.1, i.0)))
    };
}

#[derive(Eq, PartialEq, Debug)]
pub struct LocalStr<'a> {
    pub v: &'a str,
}

#[derive(Eq, PartialEq, Debug)]
pub struct LocalString {
    pub v: String,
}

impl Ord for LocalStr<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        //empty string -> to end
        //non-alphanumeric -> to end
        //ignore non-alphanumerics
        match (self.v.is_empty(), other.v.is_empty()) {
            (true, true) => Ordering::Equal,
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
            (false, false) => {
                let mut ai = self
                    .v
                    .chars()
                    .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                    .peekable();
                let mut bi = other
                    .v
                    .chars()
                    .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                    .peekable();
                match (ai.peek().is_some(), bi.peek().is_some()) {
                    (false, false) => self.v.cmp(&other.v),
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
                                (None, None) => self.v.cmp(&other.v),
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
        LocalStr { v: &self.v }.cmp(&LocalStr { v: &other.v })
    }
}

impl PartialOrd for LocalString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[test]
fn test_sort() {
    let a = LocalStr { v: "Фыва" };
    let b = LocalStr { v: "Asdf" };
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
    a.sort_by_cached_key(|x| LocalStr { v: x });
    assert_eq!(a, b);
}
