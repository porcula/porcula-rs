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
            hits = Some(hits_str.parse().expect(&format!(
                "{} {}",
                tr!["Invalid number of hits", "Некорректное число"],
                hits_str
            )));
        }
        match fts.get_facet(&path, hits) {
            Ok(res) => {
                println!("{}", serde_json::to_string(&res).unwrap());
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
                std::process::exit(2);
            }
        }
    }
}