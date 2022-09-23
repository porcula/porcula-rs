use std::cmp::{Ordering, PartialOrd};
use std::collections::HashMap;

lazy_static! {
    static ref ORDER: HashMap::<char, usize> = {
        const ORDERED: &str = "АаБбВвГгДдЕеЁёЖжЗзИиЙйКкЛлМмНнОоПпРрСсТтУуФфХхЦцЧчШшЩщЪъЫыЬьЭэЮюЯяAaBbCcDdEeFfGgHhIiJjKkLlMmNnOoPpQqRrSsTtUuVvWwXxYyZz0123456789";
        ORDERED.chars().enumerate().map(|i| (i.1, i.0)).collect()
    };
}

/// wrapper type for &str for custom collation
#[derive(Eq, PartialEq, Debug)]
pub struct LocalStr<'a>(pub &'a str);

/// wrapper type for String for custom collation
#[derive(Eq, PartialEq, Debug)]
pub struct LocalString(pub String);

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
                    (false, false) => self.0.cmp(other.0),
                    (false, true) => Ordering::Greater,
                    (true, false) => Ordering::Less,
                    (true, true) => {
                        for (a, b) in ai.zip(bi) {
                            if a == b {
                                continue;
                            }
                            return match (ORDER.get(&a), ORDER.get(&b)) {
                                (Some(a2), Some(b2)) => a2.cmp(b2),
                                (Some(_), None) => Ordering::Less,
                                (None, Some(_)) => Ordering::Greater,
                                (None, None) => self.0.cmp(other.0),
                            };
                        }
                        let a = self.0.len();
                        let b = other.0.len();
                        a.cmp(&b)
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
    let a = LocalStr("aaa");
    let b = LocalStr("aa");
    assert_eq!(a.cmp(&b), Ordering::Greater);
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
        "Мегрэ путешествует",
        "Мегрэ расставляет ловушку",
        "Мегрэ",
        "Мегрэ путешествует",
        "Мегрэ расставляет ловушку",
    ];
    let b = vec![
        "«ела»",
        "ёлка",
        "кашу",
        "Мама",
        "Маша",
        "Мегрэ",
        "Мегрэ путешествует",
        "Мегрэ путешествует",
        "Мегрэ расставляет ловушку",
        "Мегрэ расставляет ловушку",
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

 /// hashing first chars of string for sorting in ascending order using custom collation
 pub fn hash_asc(s: &str) -> u64 {
    assert!(ORDER.len()<=253, "ORDER charset too big"); //reserve two values
    const WHITESPACE: u8 = 0x00;
    let next_index = (ORDER.len() as u8) + 2;
    let mut hash: u64 = 0;
    let mut offset = 64;
    let mut prev = 0x00;
    for i in s.chars() {
        offset -= 8;
        if offset < 0 {
            break;
        }
        let octet: u8 = match ORDER.get(&i) {
            Some(x) => ((x + 1) & 0xff) as u8, //main characters [1..last_index)
            None if i.is_whitespace() => WHITESPACE,
            None => {
                //squeeze other in (last_index..255] range
                let x: u8 = (((i as u64) & 0xfe) >> 1) as u8 | 0x80; //bits 7..1 of first byte
                if x < next_index {
                    next_index
                } else {
                    x
                }
            }
        };
        if octet==WHITESPACE && prev==WHITESPACE { continue } //treat sequental spaces as one
        hash |= (octet as u64) << offset;
        prev = octet;
    }
    if hash == 0 && offset == 64 {
        hash = 0xffffffffffffffff; //empty string -> to end
    }
    hash
}

/// hashing first chars of string for sorting in descending order using custom collation
 pub fn hash_desc(s: &str) -> u64 {
    ! hash_asc(s)
 }
 

#[test]
#[rustfmt::skip]
fn test_hash() {
    assert_eq!(hash_asc(""),         0xffffffffffffffff, "empty str");
    assert_eq!(hash_asc("А"),        0x0100000000000000, "А");
    assert_eq!(hash_asc("АААААААА"), 0x0101010101010101, "АААААААА");
    assert_eq!(hash_asc("А АА"),     0x0100010100000000, "А АА");
    assert_eq!(hash_asc("А?"),       0x019f000000000000, "А?" );
    assert_eq!(hash_asc("z"),        0x7600000000000000, "z");
    assert_eq!(hash_asc("foo"),      0x4e60600000000000, "foo");
    assert_eq!(hash_asc("bar"),      0x4644660000000000, "bar");
    assert_eq!(hash_asc("Иллюзия выбора"), 0x131a1a4012144200, "Иллюзия выбора");
    assert_eq!(hash_desc(""),        0x0000000000000000, "desc empty str");
    assert_eq!(hash_desc("А"),       0xfeffffffffffffff, "desc А");
    assert_eq!(hash_desc("z"),       0x89ffffffffffffff, "desc z");
    let mut a = vec!["foo", "bar", "мама", "Мама", "2020", "мамаша", "мама мыла раму"];
    a.sort_by_cached_key(|s| hash_asc(s));
    assert_eq!(a, vec!["Мама", "мама", "мама мыла раму", "мамаша", "bar", "foo", "2020"]);
}
