use crate::cmd::*;
use crate::tr;

pub fn run_query(args: &QueryArgs, app: Application) {
    let fts = app.open_book_reader().unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(4);
    });
    match fts.search_as_json(&args.query, "default", args.hits, 0, app.debug) {
        Ok(res) => {
            println!("{}", res);
        }
        Err(e) => {
            eprintln!("{}: {}", tr!["Query error", "Ошибка запроса"], e);
            std::process::exit(2);
        }
    }
}
