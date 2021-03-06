use clap::ArgMatches;

use crate::cmd::*;
use crate::tr;

pub fn run_query(matches: &ArgMatches, app: &Application) {
    if let Some(query) = matches.value_of("QUERY-TEXT") {
        let fts = app.open_book_reader().unwrap_or_else(|e| {
            eprintln!("{}", e);
            std::process::exit(4);
        });
        let default_query_hits_str = DEFAULT_QUERY_HITS.to_string();
        let hits_str = matches.value_of("hits").unwrap_or(&default_query_hits_str);
        let hits: usize = hits_str.parse().unwrap_or_else(|_| {
            eprintln!(
                "{} {}",
                tr!["Invalid number of hits", "Некорректное число"],
                hits_str
            );
            std::process::exit(4);
        });
        match fts.search_as_json(&query, "default", hits, 0, app.debug) {
            Ok(res) => {
                println!("{}", res);
            }
            Err(e) => {
                eprintln!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
                std::process::exit(2);
            }
        }
    }
}
