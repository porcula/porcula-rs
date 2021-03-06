use clap::ArgMatches;

use crate::cmd::*;
use crate::tr;

pub fn run_facet(matches: &ArgMatches, app: &Application) {
    if let Some(path) = matches.value_of("PATH") {
        let fts = app.open_book_reader().unwrap_or_else(|e| {
            eprintln!("{}", e);
            std::process::exit(4);
        });
        let mut hits: Option<usize> = None;
        if let Some(hits_str) = matches.value_of("hits") {
            hits = Some(hits_str.parse().unwrap_or_else(|_| {
                eprintln!(
                    "{} {}",
                    tr!["Invalid number of hits", "Некорректное число"],
                    hits_str
                );
                std::process::exit(4);
            }));
        }
        match fts.get_facet(path, None, hits, app.debug) {
            Ok(res) => {
                println!("{}", serde_json::to_string(&res).unwrap());
            }
            Err(e) => {
                eprintln!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
                std::process::exit(2);
            }
        }
    }
}
