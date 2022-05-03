use crate::cmd::*;
use crate::tr;
use log::error;

pub fn run_query(args: &QueryArgs, app: Application) {
    let fts = app.open_book_reader().unwrap_or_else(|e| {
        error!("{}", e);
        std::process::exit(4);
    });
    match fts.search_as_json(&args.query, args.stem, args.disjunction, "default", args.hits, 0) {
        Ok(res) => {
            println!("{}", res);
        }
        Err(e) => {
            error!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
            std::process::exit(2);
        }
    }
}
