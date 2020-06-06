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
        if !self.v.is_empty() && other.v.is_empty() {
            return Ordering::Less;
        } else if self.v.is_empty() && !other.v.is_empty() {
            return Ordering::Greater;
        }
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
        let a1 = ai.peek().is_some();
        let b1 = bi.peek().is_some();
        //non-alphanumeric -> to end
        if !a1 && b1 {
            return Ordering::Greater;
        } else if a1 && !b1 {
            return Ordering::Less;
        } else if !a1 && !b1 {
            return self.v.cmp(&other.v);
        }
        for (a, b) in ai.zip(bi) {
            if a == b {
                continue;
            }
            let ac = ORDER.get(&a);
            let bc = ORDER.get(&b);
            if let Some(a2) = ac {
                if let Some(b2) = bc {
                    return a2.cmp(&b2);
                } else {
                    return Ordering::Less;
                }
            } else if bc.is_some() {
                return Ordering::Greater;
            }
            return a.cmp(&b);
        }
        self.v.cmp(&other.v)
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
