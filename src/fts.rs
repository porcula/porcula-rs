use rand::Rng;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tantivy::collector::{Count, FacetCollector, TopDocs};
use tantivy::query::{
    AllQuery, BooleanQuery, Occur, Query, QueryParser, QueryParserError, RegexQuery, TermQuery, FuzzyTermQuery,
};
use tantivy::schema::*;
use tantivy::tokenizer::*;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, Searcher, TantivyError};

use crate::letter_replacer::LetterReplacer;
use crate::sort::LocalString;

const MAX_MATCHES_BEFORE_ORDERING: usize = 10000;
const SIMPLE_TOKENIZER_NAME: &str = "p_simple";
const STEMMED_TOKENIZER_NAME: &str = "p_stemmed";
pub const WHOLE_MARKER: &str = "WHOLE";

type Result<T> = tantivy::Result<T>;

#[allow(dead_code)]
struct Fields {
    facet: Field, // contains: path to file + genre classification + authors catalog
    id: Field,
    encoding: Field,
    length: Field,
    lang: Field,
    keyword: Field,
    date: Field,
    title: Field,
    author: Field,
    src_author: Field,
    translator: Field,
    sequence: Field,
    seqnum: Field,
    annotation: Field,
    body: Field,        //simple tokenizer
    xbody: Field,       //stemmed tokenizer
    cover_image: Field, //jpeg in base64
}

#[derive(Debug)]
pub struct BookMeta {
    pub zipfile: String,
    pub filename: String,
    pub length: u64,
    pub title: String,
    pub lang: String,
    pub date: Option<String>,
    pub genre: Vec<String>,
    pub keyword: Vec<String>,
    pub author: Vec<String>,
    pub translator: Vec<String>,
    pub sequence: Option<String>,
    pub seqnum: Option<i64>,
    pub annotation: Option<String>,
}

pub type IndexedBooks = HashMap<String, HashSet<String>>; //zipfile->{filenames}

#[allow(dead_code)]
pub struct BookWriter {
    schema: Schema,
    index: Index,
    writer: IndexWriter,
    fields: Fields,
    use_stemmer: bool,
}

pub struct BookReader {
    reader: IndexReader,
    schema: Schema,
    query_parser: QueryParser,
    fields: Fields,
    default_fields: Vec<Field>,
}

impl Fields {
    fn build(schema_builder: &mut SchemaBuilder) -> Self {
        let simple_indexing_opts = TextFieldIndexing::default()
            .set_tokenizer(SIMPLE_TOKENIZER_NAME)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions);
        let stemmed_indexing_opts = TextFieldIndexing::default()
            .set_tokenizer(STEMMED_TOKENIZER_NAME)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions);
        let stored_text_opts = TextOptions::default()
            .set_indexing_options(simple_indexing_opts.clone())
            .set_stored();
        let nonstored_simple_text_opts =
            TextOptions::default().set_indexing_options(simple_indexing_opts);
        let nonstored_stemmed_text_opts =
            TextOptions::default().set_indexing_options(stemmed_indexing_opts);
        Fields {
            facet: schema_builder.add_facet_field("facet", INDEXED | STORED),
            id: schema_builder.add_text_field("id", STORED | STRING),
            encoding: schema_builder.add_text_field("encoding", STORED),
            length: schema_builder.add_u64_field("length", STORED),
            lang: schema_builder.add_text_field("lang", STORED | STRING),
            keyword: schema_builder.add_text_field("keyword", STORED | STRING),
            date: schema_builder.add_text_field("date", STORED | STRING),
            title: schema_builder.add_text_field("title", stored_text_opts.clone()),
            author: schema_builder.add_text_field("author", stored_text_opts.clone()),
            src_author: schema_builder.add_text_field("src_author", stored_text_opts.clone()),
            translator: schema_builder.add_text_field("translator", stored_text_opts.clone()),
            sequence: schema_builder.add_text_field("sequence", stored_text_opts.clone()),
            seqnum: schema_builder.add_i64_field("seqnum", STORED),
            annotation: schema_builder.add_text_field("annotation", stored_text_opts),
            body: schema_builder.add_text_field("body", nonstored_simple_text_opts),
            xbody: schema_builder.add_text_field("xbody", nonstored_stemmed_text_opts),
            cover_image: schema_builder.add_text_field("cover_image", STORED),
        }
    }

    fn load(schema: &Schema) -> Result<Self> {
        let load_field = |name: &str| {
            schema
                .get_field(name)
                .ok_or_else(|| TantivyError::SchemaError(format!("field not found: {}", name)))
        };
        Ok(Fields {
            facet: load_field("facet")?,
            id: load_field("id")?,
            encoding: load_field("encoding")?,
            length: load_field("length")?,
            lang: load_field("lang")?,
            keyword: load_field("keyword")?,
            date: load_field("date")?,
            title: load_field("title")?,
            author: load_field("author")?,
            src_author: load_field("src_author")?,
            translator: load_field("translator")?,
            sequence: load_field("sequence")?,
            seqnum: load_field("seqnum")?,
            annotation: load_field("annotation")?,
            body: load_field("body")?,
            xbody: load_field("xbody")?,
            cover_image: load_field("cover_image")?,
        })
    }
}

fn file_facet(zipfile: &str, filename: &str) -> Facet {
    let path: String = format!("/file/{}/{}", zipfile, filename);
    Facet::from_text(&path).unwrap()
}

fn get_simple_tokenizer() -> TextAnalyzer {
    TextAnalyzer::from(SimpleTokenizer)
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(LetterReplacer)
}

fn get_stemmed_tokenizer(stemmer: &str) -> TextAnalyzer {
    let language = match stemmer {
        "ar" => Language::Arabic,
        "da" => Language::Danish,
        "nl" => Language::Dutch,
        "en" => Language::English,
        "fi" => Language::Finnish,
        "fr" => Language::French,
        "de" => Language::German,
        "el" => Language::Greek,
        "hu" => Language::Hungarian,
        "it" => Language::Italian,
        "no" => Language::Norwegian,
        "pt" => Language::Portuguese,
        "ro" => Language::Romanian,
        "ru" => Language::Russian,
        "es" => Language::Spanish,
        "sv" => Language::Swedish,
        "ta" => Language::Tamil,
        "tr" => Language::Turkish,
        _ => return TokenizerManager::default().get("default").unwrap(),
    };
    TextAnalyzer::from(SimpleTokenizer)
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(LetterReplacer)
        .filter(Stemmer::new(language))
}

impl BookWriter {
    pub fn new<P: AsRef<Path>>(
        index_dir: P,
        stemmer: &str,
        num_threads: Option<usize>,
        heap_size: usize,
    ) -> Result<BookWriter> {
        let (index, schema, fields) = match Index::open_in_dir(&index_dir) {
            Ok(index) => {
                let schema = index.schema();
                let fields = Fields::load(&schema)?; //check all fields
                (index, schema, fields)
            }
            Err(_) => {
                //assume empty dir
                let mut schema_builder = SchemaBuilder::default();
                let fields = Fields::build(&mut schema_builder);
                let schema = schema_builder.build();
                let index = Index::create_in_dir(&index_dir, schema.clone())?;
                (index, schema, fields)
            }
        };
        let tokenizers = index.tokenizers();
        tokenizers.register(SIMPLE_TOKENIZER_NAME, get_simple_tokenizer());
        tokenizers.register(STEMMED_TOKENIZER_NAME, get_stemmed_tokenizer(stemmer));

        let writer = match num_threads {
            Some(n) if n > 0 => index.writer_with_num_threads(n, heap_size)?,
            _ => index.writer(heap_size)?,
        };

        Ok(BookWriter {
            writer,
            index,
            schema,
            fields,
            use_stemmer: stemmer != "OFF",
        })
    }

    pub fn delete_all_books(&mut self) -> Result<()> {
        self.writer.delete_all_documents()?;
        self.writer.commit()?;
        Ok(())
    }

    pub fn mark_zipfile_as_indexed(&self, zipfile: &str, count: u64) -> Result<()> {
        let mut doc = Document::default();
        let facet = Facet::from_path(vec![WHOLE_MARKER, zipfile]);
        doc.add_facet(self.fields.facet, facet);
        doc.add_u64(self.fields.length, count); //books count
        self.writer.add_document(doc)?;
        Ok(())
    }

    #[allow(clippy::cognitive_complexity)]
    pub fn add_book(
        &self,
        zipfile: &str,
        filename: &str,
        book: crate::types::Book,
        genre_map: &crate::genre_map::GenreMap,
        body: bool,
        xbody: bool,
    ) -> Result<()> {
        let mut doc = Document::default();
        doc.add_facet(self.fields.facet, file_facet(zipfile, filename)); //facet field is mandatory
        doc.add_text(self.fields.encoding, &book.encoding);
        doc.add_u64(self.fields.length, book.length);
        if let Some(id) = &book.id {
            doc.add_text(self.fields.id, &id);
        }
        for i in &book.lang {
            if !i.is_empty() {
                doc.add_text(self.fields.lang, &i)
            }
        }
        for i in &book.title {
            if !i.is_empty() {
                doc.add_text(self.fields.title, &i)
            }
        }
        for i in &book.date {
            if !i.is_empty() {
                doc.add_text(self.fields.date, &i)
            }
        }
        let mut genre_facet = vec![];
        let mut keyword = book.keyword.clone();
        for i in &book.genre {
            if !i.is_empty() {
                genre_facet.push(format!("/genre/{}", genre_map.path_for(i)));
                //if genre looks like word -> add it to keywords
                if !i.contains('_') {
                    keyword.push(i.to_lowercase());
                }
                //duplicate genre translation as keyword
                keyword.push(genre_map.translate(i).to_lowercase());
            }
        }
        if genre_facet.is_empty() {
            genre_facet.push("/genre/misc/unknown".to_string());
        }
        genre_facet.sort();
        genre_facet.dedup();
        for i in genre_facet {
            doc.add_facet(self.fields.facet, &i);
        }
        keyword.sort();
        keyword.dedup();
        for i in keyword {
            if !i.is_empty() {
                let path = format!("/kw/{}", i);
                doc.add_facet(self.fields.facet, &path);
                doc.add_text(self.fields.keyword, &i);
            }
        }
        for i in &book.author {
            let t = &i.to_string();
            if !t.is_empty() {
                doc.add_text(self.fields.author, &t);
                if let Some(name) = &i.last_name_normalized() {
                    let first = name.chars().take(1).collect::<String>();
                    let path = format!("/author/{}/{}", &first, name); //first letter/last name in proper case
                    doc.add_facet(self.fields.facet, &path);
                }
            }
        }
        for i in &book.src_author {
            let t = &i.to_string();
            if !t.is_empty() {
                doc.add_text(self.fields.src_author, &t);
                if let Some(name) = &i.last_name_normalized() {
                    let first = name.chars().next().unwrap_or('?');
                    let path = format!("/author/{}/{}", &first, name); //first letter/last name in proper case
                    doc.add_facet(self.fields.facet, &path);
                }
            }
        }
        for i in &book.translator {
            let i = i.to_string();
            if !i.is_empty() {
                doc.add_text(self.fields.translator, &i)
            }
        }
        for i in &book.sequence {
            doc.add_text(self.fields.sequence, &i);
        }
        for i in &book.seqnum {
            doc.add_i64(self.fields.seqnum, *i);
        }
        if let Some(v) = &book.annotation {
            doc.add_text(self.fields.annotation, &v);
        }
        if let Some(text) = &book.body {
            if body {
                doc.add_text(self.fields.body, &text); //simple tokenizer
            }
            if xbody && self.use_stemmer {
                doc.add_text(self.fields.xbody, &text); //stemmed tokenizer
            }
        }
        //consume book with image
        if let Some(raw) = book.cover_image {
            doc.add_text(self.fields.cover_image, base64::encode(raw));
        }
        self.writer.add_document(doc)?;
        Ok(())
    }

    pub fn commit(&mut self) -> Result<()> {
        let res = self.writer.commit().map(|_| ());
        #[cfg(not(target_os = "windows"))]
        return res;
        //windows: some spurious IO error can be fixed by retrying
        #[cfg(target_os = "windows")]
        return match res {
            Err(TantivyError::OpenWriteError(
                tantivy::directory::error::OpenWriteError::IoError { io_error, filepath },
            )) if io_error.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!(
                    "retry after error: {} at {}",
                    io_error,
                    filepath.to_string_lossy()
                );
                self.writer.commit().map(|_| ())
            }
            any => any,
        };
    }

    pub fn wait_merging_threads(self) -> Result<()> {
        self.writer.wait_merging_threads().map(|_| ())
    }
}

fn first_string(doc: &Document, field: Field) -> Option<String> {
    match doc.get_first(field) {
        Some(Value::Str(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn first_str(doc: &Document, field: Field) -> Option<&str> {
    match doc.get_first(field) {
        Some(x) => x.as_text(),
        _ => None,
    }
}

fn joined_values(doc: &Document, field: Field) -> String {
    let v: Vec<&str> = doc.get_all(field).filter_map(|x| x.as_text()).collect();
    v.join(", ")
}

fn vec_string(doc: &Document, field: Field) -> Vec<String> {
    doc.get_all(field)
        .filter_map(|x| x.as_text())
        .map(|s| s.to_string())
        .collect()
}

fn first_i64_value(doc: &Document, field: Field) -> i64 {
    doc.get_first(field)
        .map(|x| if let Value::I64(i) = x { *i } else { 0 })
        .unwrap_or(0)
}

fn first_u64_value(doc: &Document, field: Field) -> u64 {
    doc.get_first(field)
        .map(|x| if let Value::U64(i) = x { *i } else { 0 })
        .unwrap_or(0)
}

fn parse_fuzzy_pattern(pat: &str) -> (String, u8) {
    let distance = pat.matches('~').count();
    let word = pat.replace('~',"");
    (word, distance as u8)
}

impl BookReader {
    pub fn new<P: AsRef<Path>>(index_dir: P, lang: &str) -> Result<BookReader> {
        let index = Index::open_in_dir(index_dir)?;
        let tokenizers = index.tokenizers();
        tokenizers.register(SIMPLE_TOKENIZER_NAME, get_simple_tokenizer());
        tokenizers.register(STEMMED_TOKENIZER_NAME, get_stemmed_tokenizer(lang));
        let schema = index.schema();
        let fields = Fields::load(&schema)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommit)
            .try_into()?;
        let default_fields: Vec<Field> = vec![
            "title",
            "author",
            "src_author",
            "translator",
            "annotation",
            "keyword",
            "body",
        ]
        .iter()
        .filter_map(|n| schema.get_field(n))
        .collect();
        let mut query_parser = QueryParser::for_index(&index, default_fields.clone());
        query_parser.set_conjunction_by_default();
        Ok(BookReader {
            reader,
            schema,
            query_parser,
            fields,
            default_fields,
        })
    }

    /// Extract list of indexed files
    /// compact==true: Get complete zipfiles plus books of incomplete zipfiles
    /// compact==false: Get all books
    pub fn get_indexed_books(&self, compact: bool) -> Result<IndexedBooks> {
        let mut res = HashMap::new();
        let searcher = self.reader.searcher();
        if compact {
            //collect whole zipfiles
            let mut facet_collector = FacetCollector::for_field(self.fields.facet);
            let whole_facet = Facet::from_path(vec![WHOLE_MARKER]);
            facet_collector.add_facet(whole_facet.clone());
            let facet_counts = searcher.search(&AllQuery, &facet_collector)?;
            for (zip_facet, _) in facet_counts.get(whole_facet) {
                let path = zip_facet.to_path(); //0='WHOLE',1=zipfile
                if path.len() < 2 {
                    continue;
                }
                let zipfile = path[1].to_string();
                let mut hs = HashSet::new();
                hs.insert(WHOLE_MARKER.to_owned());
                res.insert(zipfile, hs);
            }
        }
        //collect files
        let mut facet_collector = FacetCollector::for_field(self.fields.facet);
        let root_facet = Facet::from_path(vec!["file"]);
        facet_collector.add_facet(root_facet.clone());
        let facet_counts = searcher.search(&AllQuery, &facet_collector)?;
        for (zip_facet, _) in facet_counts.get(root_facet) {
            let path = zip_facet.to_path(); //0='file',1=zipfile
            if path.len() < 2 {
                continue;
            }
            let zipfile = path[1].to_string();
            if res.contains_key(&zipfile) {
                continue; //skip WHOLE marked zipfiles
            }
            let mut hs = HashSet::new();
            let term = Term::from_facet(self.fields.facet, zip_facet);
            let query = TermQuery::new(term, IndexRecordOption::Basic);
            let mut facet_collector = FacetCollector::for_field(self.fields.facet);
            facet_collector.add_facet(zip_facet.clone());
            let facet_counts = searcher.search(&query, &facet_collector)?;
            for (file_facet, _) in facet_counts.get(zip_facet.clone()) {
                let path = file_facet.to_path(); //0='file',1=zipfile,2=filename
                if path.len() < 3 {
                    continue;
                }
                hs.insert(path[2].to_owned());
            }
            res.insert(zipfile, hs);
        }
        Ok(res)
    }

    pub fn count_all(&self) -> Result<usize> {
        let searcher = self.reader.searcher();
        let cnt = searcher.search(&AllQuery, &Count)?;
        Ok(cnt)
    }

    pub fn search_as_docs(
        &self,
        query: &dyn Query,
        order: &str,
        limit: usize,
        offset: usize,
        debug: bool,
    ) -> Result<Vec<Document>> {
        let searcher = self.reader.searcher();
        if debug {
            println!("debug: query={:?} order={}", query, order);
        }
        let mut docs = Vec::new();
        if order == "default" {
            let top_docs = searcher.search(query, &TopDocs::with_limit(limit + offset))?;
            for (_score, doc_address) in top_docs.iter().skip(offset) {
                let retrieved_doc = searcher.doc(*doc_address)?;
                docs.push(retrieved_doc);
            }
        } else {
            //dummy sort: get top-N relevant docs, sort them and apply offset+limit
            let collector = &TopDocs::with_limit(MAX_MATCHES_BEFORE_ORDERING);
            let mut all_docs: Vec<Document> = searcher
                .search(query, collector)?
                .iter()
                .map(|(_score, doc_address)| searcher.doc(*doc_address))
                .filter_map(|x| x.ok())
                .collect();
            let mut offset = offset;
            match order {
                "title" => all_docs.sort_by_cached_key(|d| {
                    LocalString(first_str(d, self.fields.title).unwrap_or_default().to_lowercase())
                }),
                "author" => all_docs.sort_by_cached_key(|d| {
                    LocalString(joined_values(d, self.fields.author).to_lowercase())
                }),
                "translator" => all_docs.sort_by_cached_key(|d| {
                    LocalString(joined_values(d, self.fields.translator).to_lowercase())
                }),
                "sequence" => all_docs.sort_by_cached_key(|d| {
                    (
                        LocalString(first_str(d, self.fields.sequence).unwrap_or_default().to_lowercase()),
                        first_i64_value(d, self.fields.seqnum),
                    )
                }),
                "random" => {
                    let mut rnd = rand::thread_rng();
                    all_docs.sort_by_cached_key(|_| rnd.gen::<i64>());
                    offset = 0;
                }
                x => return Err(tantivy::TantivyError::InvalidArgument(x.to_string())),
            }
            for doc in all_docs.into_iter().skip(offset).take(limit) {
                docs.push(doc);
            }
        }
        Ok(docs)
    }

    pub fn search_as_json(
        &self,
        query: &str,
        order: &str,
        limit: usize,
        offset: usize,
        debug: bool,
    ) -> Result<String> {
        let query = self.parse_query(query, debug)?;
        let docs = self.search_as_docs(&query, order, limit, offset, debug)?;
        let matches: Vec<String> = docs.iter().map(|doc| self.schema.to_json(doc)).collect();
        let total = self.reader.searcher().search(&query, &Count)?;
        Ok(format!(
            "{{\"total\":{},\"matches\":[{}]}}",
            total,
            matches.join(",\n")
        ))
    }

    pub fn search_as_meta(
        &self,
        query: &str,
        order: &str,
        limit: usize,
        offset: usize,
        debug: bool,
    ) -> Result<Vec<BookMeta>> {
        let query = self.parse_query(query, debug)?;
        let docs = self.search_as_docs(&query, order, limit, offset, debug)?;
        let mut matches = Vec::new();
        for doc in docs {
            let mut zipfile = "".to_string();
            let mut filename = "".to_string();
            let mut genre = Vec::new();
            for i in doc.get_all(self.fields.facet) {
                if let Value::Facet(f) = i {
                    let mut path = f.to_path().into_iter();
                    let p0 = path.next();
                    let p1 = path.next();
                    let p2 = path.next();
                    match p0 {
                        Some("file") => {
                            zipfile = p1.map(|x| x.to_owned()).unwrap_or_default();
                            filename = p2.map(|x| x.to_owned()).unwrap_or_default();
                        }
                        Some("genre") => {
                            if let Some(x) = p2 {
                                genre.push(x.to_owned())
                            }
                        } //skip level 1: "/genre/sf/sf_horror" -> "sf_horror"
                        _ => (),
                    }
                }
            }

            let seqnum = first_i64_value(&doc, self.fields.seqnum);
            matches.push(BookMeta {
                zipfile,
                filename,
                length: first_u64_value(&doc, self.fields.length),
                title: first_string(&doc, self.fields.title).unwrap_or_default(),
                lang: first_string(&doc, self.fields.lang).unwrap_or_default(),
                date: first_string(&doc, self.fields.date),
                genre,
                keyword: vec_string(&doc, self.fields.keyword),
                author: vec_string(&doc, self.fields.author),
                translator: vec_string(&doc, self.fields.translator),
                sequence: first_string(&doc, self.fields.sequence),
                seqnum: if seqnum != 0 { Some(seqnum) } else { None },
                annotation: first_string(&doc, self.fields.annotation),
            });
        }
        Ok(matches)
    }

    fn find_book(
        &self,
        searcher: &Searcher,
        zipfile: &str,
        filename: &str,
    ) -> Result<Option<tantivy::DocAddress>> {
        let facet_term = Term::from_facet(self.fields.facet, &file_facet(zipfile, filename));
        let query = TermQuery::new(facet_term, IndexRecordOption::Basic);
        let found = searcher.search(&query, &TopDocs::with_limit(1))?;
        if !found.is_empty() {
            Ok(Some(found[0].1))
        } else {
            Ok(None)
        }
    }

    pub fn get_book_info(&self, zipfile: &str, filename: &str) -> Result<Option<(String, String)>> {
        // (title,encoding)
        let searcher = self.reader.searcher();
        if let Some(doc_address) = self.find_book(&searcher, zipfile, filename)? {
            let doc = searcher.doc(doc_address)?;
            let title: &str = first_str(&doc, self.fields.title).unwrap_or_default();
            let encoding: &str = first_str(&doc, self.fields.encoding).unwrap_or_default();
            return Ok(Some((title.to_string(), encoding.to_string())));
        }
        Ok(None)
    }

    pub fn get_cover(&self, zipfile: &str, filename: &str) -> Result<Option<Vec<u8>>> {
        let searcher = self.reader.searcher();
        if let Some(doc_address) = self.find_book(&searcher, zipfile, filename)? {
            let doc = searcher.doc(doc_address)?;
            if let Some(base64_str) = first_string(&doc, self.fields.cover_image) {
                if let Ok(jpeg) = base64::decode(base64_str) {
                    return Ok(Some(jpeg));
                }
            }
        }
        Ok(None)
    }

    pub fn get_facet(
        &self,
        path: &str,
        query: Option<&str>,
        hits: Option<usize>,
        debug: bool,
    ) -> Result<HashMap<String, u64>> {
        let searcher = self.reader.searcher();
        let mut facet_collector = FacetCollector::for_field(self.fields.facet);
        facet_collector.add_facet(path);
        let query = match query {
            Some(q) => self.parse_query(q, debug).unwrap(),
            None => Box::new(AllQuery),
        };
        let facet_counts = searcher.search(&query, &facet_collector)?;
        let mut facets = HashMap::<String, u64>::new();
        if let Some(k) = hits {
            for (facet, count) in facet_counts.top_k(path, k) {
                facets.insert(facet.to_path_string(), count);
            }
        } else {
            for (facet, count) in facet_counts.get(path) {
                facets.insert(facet.to_path_string(), count);
            }
        }
        Ok(facets)
    }

    pub fn parse_query(&self, query: &str, debug: bool) -> Result<Box<dyn Query>> {
        //emulate wildcard queries (word* or word?) with regexes
        let mut words = vec![];
        let mut regexes = vec![];
        let mut fuzzy = vec![];
        let looks_like_regex = Regex::new(r"[.\])][*+?]").unwrap(); //  foo.* | foo[0-9]+ | (foo)?
        let looks_like_wildcard = Regex::new(r"[*?]").unwrap(); // foo* | fo?
        let looks_like_fuzzy = Regex::new(r"~$").unwrap(); // foo~

        //TODO: phrase quoting, now just split query to words
        for i in query.split_whitespace() {
            if i == "*" {
                words.push(i);
            } else if looks_like_regex.is_match(i) {
                regexes.push(i.to_lowercase());
            } else if looks_like_wildcard.is_match(i) {
                let re = i.replace('*', ".*").replace('?', ".").to_lowercase();
                regexes.push(re);
            } else if looks_like_fuzzy.is_match(i) {
                fuzzy.push(i.to_lowercase());
            } else {
                words.push(i);
            }
        }
        if debug {
            println!("debug: words={:?} regexes={:?} fuzzy={:?}", words, regexes, fuzzy);
        }
        let mut queries: Vec<(Occur, Box<dyn Query>)> = vec![];
        if !words.is_empty() {
            let std_query = words.join(" ");
            let q = self.query_parser.parse_query(&std_query)?;
            if regexes.is_empty() && fuzzy.is_empty() {
                //regular query
                return Ok(q);
            }
            queries.push((Occur::Must, q));
        }
        let field_re = Regex::new("^([a-z]+):(.+)").unwrap();
        for i in regexes {
            if let Some(m) = field_re.captures(&i) {
                let field_name = m.get(1).unwrap().as_str();
                let regex = m.get(2).unwrap().as_str();
                let field = self
                    .schema
                    .get_field(field_name)
                    .ok_or_else(|| QueryParserError::FieldDoesNotExist(field_name.to_string()))?;
                let q = RegexQuery::from_pattern(regex, field)?;
                queries.push((Occur::Must, Box::new(q)));
            } else {
                let mut subqueries: Vec<(Occur, Box<dyn Query>)> = vec![];
                for field in self.default_fields.iter() {
                    let q = RegexQuery::from_pattern(&i, *field)?; //don't want directly use tantivy_fst::Regex
                    subqueries.push((Occur::Should, Box::new(q)));
                }
                let q = BooleanQuery::from(subqueries);
                queries.push((Occur::Must, Box::new(q)));
            }
        }
        for i in fuzzy {
            if let Some(m) = field_re.captures(&i) {
                let field_name = m.get(1).unwrap().as_str();
                let pat = m.get(2).unwrap().as_str();
                let (word, distance) = parse_fuzzy_pattern(pat);
                let field = self
                    .schema
                    .get_field(field_name)
                    .ok_or_else(|| QueryParserError::FieldDoesNotExist(field_name.to_string()))?;
                let term = Term::from_field_text(field, &word);
                let q = FuzzyTermQuery::new(term, distance as u8, true);
                queries.push((Occur::Must, Box::new(q)));
            } else {
                let mut subqueries: Vec<(Occur, Box<dyn Query>)> = vec![];
                let (word, distance) = parse_fuzzy_pattern(&i);
                for field in self.default_fields.iter() {
                    let term = Term::from_field_text(*field, &word);
                    let q = FuzzyTermQuery::new(term, distance, true);
                    subqueries.push((Occur::Should, Box::new(q)));
                }
                let q = BooleanQuery::from(subqueries);
                queries.push((Occur::Must, Box::new(q)));
            }
        }
        let query = BooleanQuery::from(queries);
        Ok(Box::new(query))
    }
}
